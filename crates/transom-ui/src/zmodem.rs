//! ZMODEM 传输的界面状态：进度徽章、上传文件选择与取消入口。
//!
//! 设计原理：传输协议状态全部留在连接层，界面只持有"当前活跃会话"的
//! 展示快照（方向、文件名、进度）；事件由控制器从协议线程转发而来，
//! `AwaitUpload` 时弹系统文件选择框把用户决策经命令通道送回连接层。

use std::path::PathBuf;

use gpui_kit::base::h_flex;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_core::{ZmodemCommand, ZmodemDirection, ZmodemEvent, ZmodemFinished};

use crate::app::AppShell;
use crate::palette::Palette;
use crate::sftp::format_size;

/// 活跃 ZMODEM 会话在标签上的展示快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ZmodemState {
    pub(crate) direction: ZmodemDirection,
    pub(crate) name: String,
    pub(crate) transferred: u64,
    pub(crate) total: Option<u64>,
}

impl AppShell {
    /// 处理控制器转发的 ZMODEM 事件。
    pub(crate) fn handle_zmodem_event(
        &mut self,
        tab_id: transom_core::TabId,
        event: ZmodemEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        match event {
            ZmodemEvent::Started { direction } => {
                tab.zmodem = Some(ZmodemState {
                    direction,
                    name: String::new(),
                    transferred: 0,
                    total: None,
                });
                let text = rust_i18n::t!("zmodem.started").to_string();
                self.status = Some(text);
            }
            ZmodemEvent::Progress {
                direction,
                name,
                transferred,
                total,
            } => {
                tab.zmodem = Some(ZmodemState {
                    direction,
                    name,
                    transferred,
                    total,
                });
            }
            ZmodemEvent::AwaitUpload => {
                // 用户取消文件选择视同放弃上传：让对端 rz 尽快退出轮询。
                let Some(files) = rfd::FileDialog::new().pick_files() else {
                    self.send_zmodem_cancel(tab_id);
                    return;
                };
                self.send_zmodem_upload(tab_id, files);
                return;
            }
            ZmodemEvent::Finished(outcome) => {
                tab.zmodem = None;
                let text = match outcome {
                    ZmodemFinished::Completed { files } => {
                        send_zmodem_notification(true, files);
                        rust_i18n::t!("zmodem.done", files = files).to_string()
                    }
                    ZmodemFinished::Cancelled => rust_i18n::t!("zmodem.cancelled").to_string(),
                    ZmodemFinished::Failed(reason) => {
                        send_zmodem_notification(false, 0);
                        reason
                    }
                };
                self.status = Some(text);
            }
        }
        cx.notify();
    }

    /// 提交上传决策；命令通道满时丢弃并提示（低频用户操作，不排队）。
    fn send_zmodem_upload(&mut self, tab_id: transom_core::TabId, files: Vec<PathBuf>) {
        let command = ZmodemCommand::ZmodemUpload { tab_id, files };
        if let Err(error) = self.services.command_tx.try_send(command.into()) {
            log::warn!("ZMODEM 上传命令发送失败: {error}");
            self.status = Some(rust_i18n::t!("zmodem.command_busy").to_string());
        }
    }

    /// 请求取消当前标签的 ZMODEM 传输。
    pub(crate) fn send_zmodem_cancel(&mut self, tab_id: transom_core::TabId) {
        let command = ZmodemCommand::ZmodemCancel { tab_id };
        if let Err(error) = self.services.command_tx.try_send(command.into()) {
            log::warn!("ZMODEM 取消命令发送失败: {error}");
        }
    }

    /// 传输进行中把 Esc 解释为取消命令。
    ///
    /// 设计原理：协议激活期间写过滤本就丢弃用户输入，Esc 到不了远端程序，
    /// 因此单独截获它不会与 vim 等全屏程序争键；无传输时保持零开销直接返回，
    /// 避免干扰搜索栏、弹窗等既有 Esc 语义。
    pub(crate) fn on_zmodem_cancel_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if !event.keystroke.key.eq_ignore_ascii_case("escape") {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }
        let Some(tab_id) = self.workspace.active_tab else {
            return;
        };
        if self
            .workspace
            .tabs
            .iter()
            .any(|tab| tab.id == tab_id && tab.zmodem.is_some())
        {
            self.send_zmodem_cancel(tab_id);
        }
    }

    /// 状态栏的 ZMODEM 进度徽章：进度/文件名 + 取消按钮。
    /// 无活跃会话时返回 None。
    pub(crate) fn render_zmodem_badge(
        &mut self,
        tab_id: transom_core::TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = self
            .workspace
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.zmodem.clone())?;
        let direction_label = match state.direction {
            ZmodemDirection::Upload => rust_i18n::t!("zmodem.upload").to_string(),
            ZmodemDirection::Download => rust_i18n::t!("zmodem.download").to_string(),
        };
        let name = if state.name.is_empty() {
            direction_label.clone()
        } else {
            format!("{direction_label}: {}", state.name)
        };
        let progress = match state.total.filter(|total| *total > 0) {
            Some(total) => format!(
                "{} / {}",
                format_size(state.transferred),
                format_size(total)
            ),
            None => format_size(state.transferred),
        };
        let tooltip = format!(
            "{name} · {progress} · {}",
            rust_i18n::t!("zmodem.cancel_hint")
        );
        let cancel = cx.listener(move |this, _, _, _cx| {
            this.send_zmodem_cancel(tab_id);
        });
        Some(
            h_flex()
                .id("zmodem-badge")
                .flex_none()
                .max_w(rems(18.0))
                .gap_2()
                .items_center()
                .h(rems(crate::status_bar::BADGE_HEIGHT))
                .px_2()
                .rounded(p.radius.md)
                .bg(p.muted)
                .border_1()
                .border_color(p.foreground.opacity(0.08))
                .tooltip(move |_window, cx| Tooltip::new(tooltip.clone()).build(_window, cx))
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(progress),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(p.foreground)
                        .child(name),
                )
                .child(
                    div()
                        .id("zmodem-cancel")
                        .flex_none()
                        .px_1()
                        .rounded(p.radius.sm)
                        .text_xs()
                        .text_color(p.danger)
                        .cursor_pointer()
                        .hover(|style| style.bg(p.danger.opacity(0.12)))
                        .child(rust_i18n::t!("zmodem.cancel").to_string())
                        .on_click(cancel),
                )
                .into_any_element(),
        )
    }
}

/// 桌面通知：仅完成与失败需要提示（进行中有状态栏徽章，取消是用户主动行为）。
fn send_zmodem_notification(success: bool, files: usize) {
    let title = if success {
        rust_i18n::t!("zmodem.notify_done_title").to_string()
    } else {
        rust_i18n::t!("zmodem.notify_failed_title").to_string()
    };
    let body = if success {
        rust_i18n::t!("zmodem.notify_done_body", files = files).to_string()
    } else {
        rust_i18n::t!("zmodem.notify_failed_body").to_string()
    };
    crate::sftp::send_desktop_notification(&title, &body);
}
