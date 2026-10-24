//! SFTP 悬停预览的交互状态机：延迟、请求、结果写回与任务生命周期。
//!
//! 与 `preview.rs` 的分工：本模块只负责「什么时候读、读到哪里去」，
//! 文件类型判定、文本行生成与缓存路径等纯逻辑留在 `preview`，
//! 两者拆开是为了让状态机与内容处理都能被单独审阅。

use super::preview::{
    HOVER_PREVIEW_DELAY, PREVIEW_IMAGE_MAX_BYTES, PREVIEW_TEXT_MAX_BYTES, PreviewContent,
    PreviewKey, PreviewKind, SftpHoverTarget, SftpPreviewResponse, SftpPreviewState,
    image_extension, preview_cache_path, preview_kind, read_local_text_head, text_preview_lines,
};
use super::{SftpPanelState, format_size};
use crate::app::AppShell;
use crate::palette::localized_error;
use gpui_kit::*;
use std::path::{Path, PathBuf};
use transom_core::{AppCommand, PreviewMode, SftpOperation, TabId, TransferId};

/// 远端图片写缓存所需的完整参数，避免写盘任务入口平铺过多参数。
struct PreviewImageWrite {
    tab_id: TabId,
    op_id: TransferId,
    path: PathBuf,
    bytes: Vec<u8>,
}

impl AppShell {
    /// 文件行悬停变化：进入时安排延迟读取，离开时只清理自己这一项。
    ///
    /// 设计原理（Why）：行的 `on_hover` 在子元素间移动时可能连续触发
    /// leave/enter，按 `PreviewKey` 匹配清理可避免新目标的预览被旧行的
    /// 离场事件误清。
    pub(crate) fn sftp_hover_preview(
        &mut self,
        target: SftpHoverTarget,
        hovered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !hovered {
            self.clear_sftp_hover_preview(target.tab_id, &target.key, cx);
            return;
        }
        if target.is_dir {
            self.clear_sftp_hover_preview(target.tab_id, &target.key, cx);
            return;
        }
        let Some(kind) = preview_kind(&target.name) else {
            self.clear_sftp_hover_preview(target.tab_id, &target.key, cx);
            return;
        };
        let same_target = self
            .sftp_panel_mut_by_tab(target.tab_id)
            .and_then(|panel| panel.hover_preview.as_ref())
            .is_some_and(|preview| preview.key == target.key);
        if same_target {
            return;
        }

        let anchor = window.mouse_position();
        let delay_task = Self::schedule_preview_start(cx, target.clone(), anchor, kind);
        let Some(panel) = self.sftp_panel_mut_by_tab(target.tab_id) else {
            return;
        };
        if let Some(previous) = panel.hover_preview.replace(SftpPreviewState {
            key: target.key,
            name: target.name,
            kind,
            anchor,
            content: PreviewContent::Pending,
            op_id: None,
            cache_file: None,
            _delay_task: Some(delay_task),
            write_task: None,
        }) {
            previous.discard_cache_file_if_any();
        }
        cx.notify();
    }

    /// 清理指定目标的预览；当前预览不是该目标时保持不动。
    pub(crate) fn clear_sftp_hover_preview(
        &mut self,
        tab_id: TabId,
        key: &PreviewKey,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.sftp_panel_mut_by_tab(tab_id) else {
            return;
        };
        if panel
            .hover_preview
            .as_ref()
            .is_none_or(|preview| preview.key != *key)
        {
            return;
        }
        if let Some(preview) = panel.hover_preview.take() {
            preview.discard_cache_file_if_any();
            cx.notify();
        }
    }

    /// 无条件收起当前预览（点击、刷新、切换目录时调用）。
    pub(crate) fn dismiss_sftp_hover_preview(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(panel) = self.sftp_panel_mut_by_tab(tab_id) else {
            return;
        };
        if let Some(preview) = panel.hover_preview.take() {
            preview.discard_cache_file_if_any();
            cx.notify();
        }
    }

    fn sftp_panel_mut_by_tab(&mut self, tab_id: TabId) -> Option<&mut SftpPanelState> {
        self.tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)?
            .sftp
            .as_mut()
    }

    fn schedule_preview_start(
        cx: &mut Context<Self>,
        target: SftpHoverTarget,
        anchor: Point<Pixels>,
        kind: PreviewKind,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_PREVIEW_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.start_sftp_hover_preview(target, anchor, kind, cx);
            });
        })
    }

    /// 悬停延迟结束：按目标来源读取预览内容。
    fn start_sftp_hover_preview(
        &mut self,
        target: SftpHoverTarget,
        anchor: Point<Pixels>,
        kind: PreviewKind,
        cx: &mut Context<Self>,
    ) {
        let connection_id = self
            .tabs
            .iter()
            .find(|tab| tab.id == target.tab_id)
            .and_then(|tab| tab.connection_id);
        let command_tx = self.services.command_tx.clone();
        let Some(panel) = self.sftp_panel_mut_by_tab(target.tab_id) else {
            return;
        };
        let Some(preview) = panel.hover_preview.as_mut() else {
            return;
        };
        if preview.key != target.key || !matches!(preview.content, PreviewContent::Pending) {
            return;
        }
        preview.anchor = anchor;
        preview.content = PreviewContent::Loading;

        if kind == PreviewKind::Image && target.size > PREVIEW_IMAGE_MAX_BYTES {
            preview.content = PreviewContent::Error(format!(
                "图片大小 {} 超过 8 MB，已跳过预览",
                format_size(target.size)
            ));
            cx.notify();
            return;
        }

        if target.key.is_remote {
            let Some(connection_id) = connection_id else {
                preview.content =
                    PreviewContent::Error("SSH 连接尚未就绪，无法读取预览".to_string());
                cx.notify();
                return;
            };
            let (mode, max_bytes) = match kind {
                PreviewKind::Image => (PreviewMode::Full, PREVIEW_IMAGE_MAX_BYTES),
                PreviewKind::Text => (PreviewMode::Head, PREVIEW_TEXT_MAX_BYTES),
            };
            let op_id = TransferId::new();
            preview.op_id = Some(op_id);
            let _ = command_tx.send(AppCommand::Sftp {
                tab_id: target.tab_id,
                connection_id,
                op_id,
                op: SftpOperation::Preview {
                    path: target.key.path,
                    mode,
                    max_bytes,
                },
            });
        } else {
            match kind {
                PreviewKind::Image => {
                    preview.content = PreviewContent::Image {
                        path: PathBuf::from(&target.key.path),
                    };
                }
                PreviewKind::Text => {
                    preview.content = match read_local_text_head(
                        Path::new(&target.key.path),
                        PREVIEW_TEXT_MAX_BYTES,
                    ) {
                        Ok((bytes, truncated)) => {
                            let (lines, truncated) = text_preview_lines(&bytes, truncated);
                            PreviewContent::Text { lines, truncated }
                        }
                        Err(error) => PreviewContent::Error(format!("读取本地文件失败: {error}")),
                    };
                }
            }
        }
        cx.notify();
    }

    /// 处理远端预览的读取结果（成功或失败）。
    pub(crate) fn handle_sftp_preview(
        &mut self,
        response: SftpPreviewResponse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SftpPreviewResponse {
            tab_id,
            connection_id,
            op_id,
            result,
        } = response;
        let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) else {
            return;
        };
        let Some(preview) = panel.hover_preview.as_mut() else {
            return;
        };
        if preview.op_id != Some(op_id) {
            return;
        }

        match result {
            Err(error) => {
                preview.op_id = None;
                preview.content = PreviewContent::Error(localized_error(&error));
            }
            Ok((bytes, truncated)) => match preview.kind {
                PreviewKind::Text => {
                    preview.op_id = None;
                    let (lines, truncated) = text_preview_lines(&bytes, truncated);
                    preview.content = PreviewContent::Text { lines, truncated };
                }
                PreviewKind::Image => {
                    if bytes.is_empty() {
                        preview.op_id = None;
                        preview.content =
                            PreviewContent::Error("远端图片内容为空，无法预览".to_string());
                    } else {
                        let path =
                            preview_cache_path(tab_id, op_id, image_extension(&preview.name));
                        if let Some(previous) = preview.cache_file.take() {
                            let _ = std::fs::remove_file(previous);
                        }
                        preview.content = PreviewContent::Loading;
                        preview.write_task = Some(Self::spawn_preview_image_write(
                            cx,
                            window,
                            PreviewImageWrite {
                                tab_id,
                                op_id,
                                path,
                                bytes,
                            },
                        ));
                    }
                }
            },
        }
        cx.notify();
    }

    /// 在后台线程把远端图片字节写入本地缓存，避免大图写盘卡住 UI 线程。
    fn spawn_preview_image_write(
        cx: &mut Context<Self>,
        window: &mut Window,
        write: PreviewImageWrite,
    ) -> Task<()> {
        let PreviewImageWrite {
            tab_id,
            op_id,
            path,
            bytes,
        } = write;
        cx.spawn_in(window, async move |this, cx| {
            let write_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(dir) = write_path.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    std::fs::write(&write_path, &bytes)?;
                    Ok::<PathBuf, std::io::Error>(write_path)
                })
                .await;
            let _ = this.update_in(cx, |this, _window, cx| {
                this.finish_sftp_preview_image_write(tab_id, op_id, result, cx);
            });
        })
    }

    fn finish_sftp_preview_image_write(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        result: std::io::Result<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.sftp_panel_mut_by_tab(tab_id) else {
            return;
        };
        let Some(preview) = panel.hover_preview.as_mut() else {
            return;
        };
        if preview.op_id != Some(op_id) {
            return;
        }
        preview.op_id = None;
        preview.write_task = None;
        match result {
            Ok(path) => {
                preview.content = PreviewContent::Image { path: path.clone() };
                preview.cache_file = Some(path);
            }
            Err(error) => {
                preview.content = PreviewContent::Error(format!("写入图片预览缓存失败: {error}"));
            }
        }
        cx.notify();
    }
}
