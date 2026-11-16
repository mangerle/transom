//! 快捷键动作分发与顶层浮层遮罩渲染。

use super::AppShell;
use crate::keymap;
use crate::palette::Palette;
use gpui_kit::*;

impl AppShell {
    pub(crate) fn on_new_session(
        &mut self,
        _: &keymap::NewSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_new_session(window, cx);
    }

    pub(crate) fn on_close_tab(
        &mut self,
        _: &keymap::CloseTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab_id) = self.state.workspace.active_tab {
            self.close_tab(tab_id, window, cx);
        }
    }

    pub(crate) fn on_next_tab(
        &mut self,
        _: &keymap::NextTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_tab(true, window, cx);
    }

    pub(crate) fn on_prev_tab(
        &mut self,
        _: &keymap::PrevTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_tab(false, window, cx);
    }

    pub(crate) fn on_reconnect(
        &mut self,
        _: &keymap::Reconnect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab_id) = self.state.workspace.active_tab {
            self.reconnect_tab(tab_id, window, cx);
        }
    }

    pub(crate) fn on_toggle_sidebar(
        &mut self,
        _: &keymap::ToggleSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.session.sidebar.toggle_visible();
        cx.notify();
    }

    /// 切换沉浸式全屏：同步系统窗口全屏状态与应用外框显隐。
    pub(crate) fn on_toggle_immersive(
        &mut self,
        _: &keymap::ToggleImmersive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.ui.immersive = !self.state.ui.immersive;
        window.toggle_fullscreen();
        if self.state.ui.immersive {
            self.state.ui.modals.close_transient();
        }
        if self.state.ui.immersive
            && let Some(focus_handle) = self
                .state
                .workspace
                .active_tab
                .and_then(|tab_id| self.state.workspace.tab(tab_id))
                .and_then(|tab| tab.terminal())
                .map(|terminal| terminal.read(cx).focus_handle().clone())
        {
            window.focus(&focus_handle, cx);
        }
        cx.notify();
    }

    /// 渲染仍走自绘遮罩的浮层。
    ///
    /// 已迁入组件库弹窗栈的模态不在此列：它们的开关与层序由
    /// `dialog_host::reconcile_dialogs` 按期望栈对账，若这里再渲染一次，
    /// 同一个弹窗就会同时出现在自绘遮罩与容器栈两层。
    pub(crate) fn render_overlays(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = Palette::from(cx);
        let mut overlays = Vec::new();
        if let Some(preview) = self
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| crate::sftp::sftp_state(&self.state.workspace, tab_id, cx))
            .and_then(|state| state.hover_preview.as_ref())
            .and_then(|preview| crate::sftp::render_sftp_hover_preview(Some(preview), window, &p))
        {
            overlays.push(preview);
        }
        if self.state.ui.modals.tools.show_quick_open {
            overlays.push(crate::command_palette::render_command_palette_modal(
                self, window, cx, &p,
            ));
        }
        if let Some(tab_id) = self.state.features.containers.fullscreen {
            overlays.push(self.render_container_fullscreen(tab_id, window, cx, &p));
        }
        if let Some(clone_task) =
            crate::git::clone_modal::render_git_clone_task(self, window, cx, &p)
        {
            overlays.push(clone_task);
        }
        overlays
    }
}
