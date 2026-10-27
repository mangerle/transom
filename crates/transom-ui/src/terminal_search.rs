//! 终端缓冲区检索的界面侧落地：把工作区快捷键转发给当前窗格的终端视图。
//!
//! 检索状态与搜索栏都归属 `TerminalView`（每个窗格一份），这里只负责"哪一个窗格"
//! 的解析：布局与焦点由工作区决定，检索本身不知道窗格之外的概念。

use crate::app::AppShell;
use crate::keymap;
use gpui_kit::*;
use transom_terminal::TerminalView;

impl AppShell {
    pub(crate) fn on_search_buffer(
        &mut self,
        _: &keymap::SearchBuffer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.active_pane_terminal() else {
            // 窗格还在连接/重连时没有终端视图，给出可见反馈而不是静默无响应。
            self.status = Some(rust_i18n::t!("search.terminal_unavailable").to_string());
            cx.notify();
            return;
        };
        terminal.update(cx, |view, cx| view.toggle_search(window, cx));
    }

    /// 运行时一键切换语义块/经典连续流，不重启会话。
    pub(crate) fn on_toggle_block_view(
        &mut self,
        _: &keymap::ToggleBlockView,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.active_pane_terminal() else {
            self.status = Some(rust_i18n::t!("search.terminal_unavailable").to_string());
            cx.notify();
            return;
        };
        let enabled = terminal.update(cx, |view, cx| view.toggle_block_view(cx));
        let key = if enabled {
            "block_view_status.enabled"
        } else {
            "block_view_status.disabled"
        };
        self.status = Some(rust_i18n::t!(key).to_string());
        cx.notify();
    }

    /// 当前窗格已建好的终端实体；未连接或连接已断开的窗格返回 `None`。
    pub(crate) fn active_pane_terminal(&self) -> Option<Entity<TerminalView>> {
        let tab_id = self.active_tab?;
        self.tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.terminal.clone())
    }
}
