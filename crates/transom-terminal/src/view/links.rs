//! OSC 8 超链接的鼠标交互：点击打开、悬停手型与系统打开器。

use super::TerminalView;
use crate::event::TerminalEvent;
use crate::hyperlink::{HyperlinkTarget, classify};
use gpui_kit::*;

impl TerminalView {
    /// Ctrl+点击打开终端里的 OSC 8 超链接，返回点击是否命中链接。
    ///
    /// 网页链接只放行 http/https/mailto/ftp 白名单；`file:line:col` 形式的
    /// 代码位置不在这里打开——终端没有编辑器，投递给应用层去解析路径与跳行。
    /// 命中但目标被拒同样视为已消费，避免 Ctrl+点击误启选择拖拽。
    pub(super) fn open_hyperlink_at(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(uri) = self.link_at(position) else {
            return false;
        };
        match classify(&uri) {
            HyperlinkTarget::Url(uri) => {
                log::info!("打开终端超链接: {uri}");
                #[cfg(unix)]
                open_external_uri(&uri);
                #[cfg(not(unix))]
                log::warn!("当前平台暂不支持打开终端超链接");
            }
            HyperlinkTarget::CodeLocation { path, line, column } => {
                log::info!("打开终端代码位置: {path}:{line}:{column:?}");
                cx.emit(TerminalEvent::OpenCodeLocation { path, line, column });
            }
            HyperlinkTarget::Unsupported => {
                log::warn!("超链接目标不受支持，已拒绝打开: {uri}");
            }
        }
        true
    }

    /// 当前像素位置下的链接 URI；不在链接格上返回 `None`。
    fn link_at(&self, position: Point<Pixels>) -> Option<String> {
        let point = self.hit_test_grid_point(position)?;
        self.state.engine.lock().hyperlink_at(point)
    }

    /// 悬停链接时把系统光标切成手型，离开后恢复箭头。
    ///
    /// 只在状态翻转时调用 `set_cursor_style`：鼠标移动是高频事件，每次移动都改光标
    /// 会向平台重复提交同一样式。不支持的目标不显示手型，避免暗示可点击。
    pub(super) fn update_link_cursor(&mut self, position: Point<Pixels>, window: &mut Window) {
        let over = self
            .link_at(position)
            .is_some_and(|uri| !matches!(classify(&uri), HyperlinkTarget::Unsupported));
        if over == self.hover_link {
            return;
        }
        self.hover_link = over;
        self.apply_pointer_style(window, over);
    }

    fn apply_pointer_style(&self, window: &mut Window, pointing: bool) {
        let hitbox = self.state.canvas_hitbox.lock().clone();
        if let Some(hitbox) = hitbox {
            let style = if pointing {
                CursorStyle::PointingHand
            } else {
                CursorStyle::Arrow
            };
            window.set_cursor_style(style, &hitbox);
        }
    }

    /// 指针离开终端画布：恢复箭头，避免手型光标滞留在工具栏上。
    pub(super) fn on_terminal_hover(
        &mut self,
        hovered: &bool,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        if !*hovered && self.hover_link {
            self.hover_link = false;
            self.apply_pointer_style(window, false);
        }
    }
}

/// 交给系统默认程序打开 URI，并在专用线程里回收子进程。
#[cfg(unix)]
fn open_external_uri(uri: &str) {
    let uri = uri.to_string();
    let spawned = std::thread::Builder::new()
        .name("transom-open-uri".into())
        .spawn(
            move || match std::process::Command::new("xdg-open").arg(&uri).spawn() {
                Ok(mut child) => {
                    let _ = child.wait();
                }
                Err(error) => log::warn!("打开超链接失败，原因: {error}"),
            },
        );
    if spawned.is_err() {
        log::warn!("启动超链接打开线程失败，已忽略本次请求");
    }
}
