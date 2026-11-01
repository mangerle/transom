//! SFTP 悬停预览的交互状态机：延迟、读取请求、缩略图生成与任务生命周期。
//!
//! 与 `preview.rs` 的分工：本模块只负责「什么时候读、读到哪里去」，
//! 文件类型判定、文本行生成与缩略图解码等纯逻辑留在 `preview`，
//! 两者拆开是为了让状态机与内容处理都能被单独审阅。

use super::entity::SftpPanelView;
use super::format_size;
use super::preview::{
    HOVER_PREVIEW_DELAY, PREVIEW_IMAGE_MAX_BYTES, PREVIEW_TEXT_MAX_BYTES, PreviewContent,
    PreviewKey, PreviewKind, SftpHoverTarget, SftpPreviewResponse, SftpPreviewState,
    decode_preview_thumbnail, preview_kind, read_local_text_head, text_preview_lines,
};
use crate::app::{AppShell, command_send_error_text};
use crate::palette::localized_error;
use gpui_kit::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use transom_core::{PreviewMode, SftpCommand, SftpOperation, TabId, TransferId};

/// 图片缩略图任务的输入来源。
enum PreviewImageInput {
    /// 远端预览事件已经返回的完整字节。
    Remote(Vec<u8>),
    /// 本地图片路径与目录列表给出的文件大小。
    Local { path: PathBuf, size: u64 },
}

/// 图片缩略图任务参数，避免异步入口平铺过多参数。
struct PreviewImageJob {
    tab_id: TabId,
    op_id: TransferId,
    input: PreviewImageInput,
}

impl SftpPanelView {
    /// 文件行悬停变化：进入时安排延迟读取，离开时只清理自己这一项。
    ///
    /// 设计原理（Why）：行的 `on_hover` 在子元素间移动时可能连续触发
    /// leave/enter，按 `PreviewKey` 匹配清理可避免新目标的预览被旧行的
    /// 离场事件误清。
    pub(crate) fn hover_preview(
        &mut self,
        target: SftpHoverTarget,
        hovered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !hovered {
            self.clear_hover_preview(&target.key, cx);
            return;
        }
        if target.is_dir {
            self.clear_hover_preview(&target.key, cx);
            return;
        }
        let Some(kind) = preview_kind(&target.name) else {
            self.clear_hover_preview(&target.key, cx);
            return;
        };
        let same_target = self
            .state
            .hover_preview
            .as_ref()
            .is_some_and(|preview| preview.key == target.key);
        if same_target {
            return;
        }

        let anchor = window.mouse_position();
        let delay_task = Self::schedule_preview_start(cx, target.clone(), anchor, kind);
        // 旧状态若已渲染卡片则需清理重绘，若仅为 Pending 则无需重绘
        let need_notify = self
            .state
            .hover_preview
            .as_ref()
            .is_some_and(|p| !matches!(p.content, PreviewContent::Pending));
        self.state.hover_preview = Some(SftpPreviewState {
            key: target.key,
            name: target.name,
            kind,
            anchor,
            content: PreviewContent::Pending,
            op_id: None,
            _delay_task: Some(delay_task),
            decode_task: None,
        });
        if need_notify {
            cx.notify();
        }
    }

    /// 清理指定目标的预览；当前预览不是该目标时保持不动。
    pub(crate) fn clear_hover_preview(&mut self, key: &PreviewKey, cx: &mut Context<Self>) {
        if self
            .state
            .hover_preview
            .as_ref()
            .is_none_or(|preview| preview.key != *key)
        {
            return;
        }
        if self
            .state
            .hover_preview
            .take()
            .is_some_and(|prev| !matches!(prev.content, PreviewContent::Pending))
        {
            cx.notify();
        }
    }

    /// 无条件收起当前预览（点击、刷新、切换目录时调用）。
    pub(crate) fn dismiss_hover_preview(&mut self, cx: &mut Context<Self>) {
        if self
            .state
            .hover_preview
            .take()
            .is_some_and(|prev| !matches!(prev.content, PreviewContent::Pending))
        {
            cx.notify();
        }
    }

    fn schedule_preview_start(
        cx: &mut Context<Self>,
        target: SftpHoverTarget,
        anchor: Point<Pixels>,
        kind: PreviewKind,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_PREVIEW_DELAY).await;
            let _ = this.update(cx, |view, cx| {
                view.start_preview(target, anchor, kind, cx);
            });
        })
    }

    /// 悬停延迟结束：按目标来源读取预览内容。
    fn start_preview(
        &mut self,
        target: SftpHoverTarget,
        anchor: Point<Pixels>,
        kind: PreviewKind,
        cx: &mut Context<Self>,
    ) {
        let Some(command_tx) = self.state.command_tx.clone() else {
            return;
        };
        // 本地缩略图任务在状态变更前启动：回调晚于本函数返回，句柄随即挂回预览项。
        let mut local_image = None;
        if kind == PreviewKind::Image
            && !target.key.is_remote
            && target.size <= PREVIEW_IMAGE_MAX_BYTES
        {
            let op_id = TransferId::new();
            let task = Self::spawn_preview_image_thumbnail(
                cx,
                PreviewImageJob {
                    tab_id: target.tab_id,
                    op_id,
                    input: PreviewImageInput::Local {
                        path: PathBuf::from(&target.key.path),
                        size: target.size,
                    },
                },
            );
            local_image = Some((op_id, task));
        }

        let Some(preview) = self.state.hover_preview.as_mut() else {
            return;
        };
        if preview.key != target.key || !matches!(preview.content, PreviewContent::Pending) {
            return;
        }
        preview.anchor = anchor;
        preview.content = PreviewContent::Loading;

        if kind == PreviewKind::Image && target.size > PREVIEW_IMAGE_MAX_BYTES {
            preview.content = PreviewContent::Error(
                rust_i18n::t!(
                    "sftp.preview.image_too_large",
                    size = format_size(target.size),
                    max = format_size(PREVIEW_IMAGE_MAX_BYTES)
                )
                .to_string(),
            );
            cx.notify();
            return;
        }

        if target.key.is_remote {
            let Some(connection_id) = target.key.connection_id else {
                preview.content =
                    PreviewContent::Error(rust_i18n::t!("sftp.preview.ssh_not_ready").to_string());
                cx.notify();
                return;
            };
            let (mode, max_bytes) = match kind {
                PreviewKind::Image => (PreviewMode::Full, PREVIEW_IMAGE_MAX_BYTES),
                PreviewKind::Text => (PreviewMode::Head, PREVIEW_TEXT_MAX_BYTES),
            };
            let op_id = TransferId::new();
            preview.op_id = Some(op_id);
            // preview 借用活跃，无法直接走外壳命令通道：用克隆的发送端 try_send，
            // 失败把原因写进预览气泡而不是静默丢弃。
            if let Err(error) = command_tx.try_send(
                SftpCommand::Sftp {
                    tab_id: target.tab_id,
                    connection_id,
                    op_id,
                    op: SftpOperation::Preview {
                        path: target.key.path,
                        mode,
                        max_bytes,
                    },
                }
                .into(),
            ) {
                preview.content = PreviewContent::Error(command_send_error_text(error));
                preview.op_id = None;
            }
        } else {
            match kind {
                PreviewKind::Image => {
                    if let Some((op_id, task)) = local_image {
                        preview.op_id = Some(op_id);
                        preview.decode_task = Some(task);
                    }
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
                        Err(error) => PreviewContent::Error(
                            rust_i18n::t!("sftp.preview.local_read_failed", error = error)
                                .to_string(),
                        ),
                    };
                }
            }
        }
        cx.notify();
    }

    /// 处理远端预览的读取结果（成功或失败）。
    pub(crate) fn handle_preview(&mut self, response: SftpPreviewResponse, cx: &mut Context<Self>) {
        let SftpPreviewResponse {
            tab_id,
            connection_id: _,
            op_id,
            result,
        } = response;
        let kind = self
            .state
            .hover_preview
            .as_ref()
            .filter(|preview| preview.op_id == Some(op_id))
            .map(|preview| preview.kind);
        let Some(kind) = kind else {
            return;
        };
        let Some(preview) = self.state.hover_preview.as_mut() else {
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
            Ok((bytes, truncated)) => match kind {
                PreviewKind::Text => {
                    preview.op_id = None;
                    let (lines, truncated) = text_preview_lines(&bytes, truncated);
                    preview.content = PreviewContent::Text { lines, truncated };
                }
                PreviewKind::Image => {
                    if bytes.is_empty() {
                        preview.op_id = None;
                        preview.content = PreviewContent::Error(
                            rust_i18n::t!("sftp.preview.remote_empty").to_string(),
                        );
                    } else {
                        preview.content = PreviewContent::Loading;
                        preview.decode_task = Some(Self::spawn_preview_image_thumbnail(
                            cx,
                            PreviewImageJob {
                                tab_id,
                                op_id,
                                input: PreviewImageInput::Remote(bytes),
                            },
                        ));
                    }
                }
            },
        }
        cx.notify();
    }

    /// 在后台线程读取本地图片并生成缩略图，避免解码与磁盘 IO 卡住 UI 线程。
    fn spawn_preview_image_thumbnail(cx: &mut Context<Self>, job: PreviewImageJob) -> Task<()> {
        let PreviewImageJob {
            tab_id,
            op_id,
            input,
        } = job;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let bytes = match input {
                        PreviewImageInput::Remote(bytes) => bytes,
                        PreviewImageInput::Local { path, size } => {
                            if size > PREVIEW_IMAGE_MAX_BYTES {
                                return Err(rust_i18n::t!(
                                    "sftp.preview.image_too_large",
                                    size = format_size(size),
                                    max = format_size(PREVIEW_IMAGE_MAX_BYTES)
                                )
                                .to_string());
                            }
                            let bytes = std::fs::read(&path).map_err(|error| {
                                rust_i18n::t!("sftp.preview.local_image_read_failed", error = error)
                                    .to_string()
                            })?;
                            if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                                > PREVIEW_IMAGE_MAX_BYTES
                            {
                                return Err(rust_i18n::t!(
                                    "sftp.preview.image_too_large",
                                    size =
                                        format_size(u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
                                    max = format_size(PREVIEW_IMAGE_MAX_BYTES)
                                )
                                .to_string());
                            }
                            bytes
                        }
                    };
                    decode_preview_thumbnail(&bytes)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.finish_preview_image_thumbnail(tab_id, op_id, result, cx);
            });
        })
    }

    fn finish_preview_image_thumbnail(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        result: Result<Arc<RenderImage>, String>,
        cx: &mut Context<Self>,
    ) {
        let _ = tab_id;
        let Some(preview) = self.state.hover_preview.as_mut() else {
            return;
        };
        if preview.op_id != Some(op_id) {
            return;
        }
        preview.op_id = None;
        preview.decode_task = None;
        preview.content = match result {
            Ok(thumbnail) => PreviewContent::Image { thumbnail },
            Err(message) => PreviewContent::Error(message),
        };
        cx.notify();
    }
}

impl AppShell {
    pub(crate) fn dismiss_sftp_hover_preview(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(view) = self.state.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.dismiss_hover_preview(cx));
        }
    }

    pub(crate) fn handle_sftp_preview(
        &mut self,
        response: SftpPreviewResponse,
        cx: &mut Context<Self>,
    ) {
        let tab_id = response.tab_id;
        let connection_id = response.connection_id;
        if self
            .sftp_state_for_connection(tab_id, connection_id, cx)
            .is_none()
        {
            return;
        }
        if let Some(view) = self.state.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.handle_preview(response, cx));
        }
    }
}
