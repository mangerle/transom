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
        if let Some(tab_id) = self.workspace.active_tab {
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
        if let Some(tab_id) = self.workspace.active_tab {
            self.reconnect_tab(tab_id, window, cx);
        }
    }

    pub(crate) fn on_toggle_sidebar(
        &mut self,
        _: &keymap::ToggleSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_visible = !self.sidebar_visible;
        cx.notify();
    }

    /// 切换沉浸式全屏：同步系统窗口全屏状态与应用外框显隐。
    pub(crate) fn on_toggle_immersive(
        &mut self,
        _: &keymap::ToggleImmersive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.immersive = !self.immersive;
        window.toggle_fullscreen();
        if self.immersive {
            self.modals.snippet_panel.open = false;
        }
        if self.immersive
            && let Some(focus_handle) = self
                .workspace
                .active_tab
                .and_then(|tab_id| self.workspace.tabs.iter().find(|tab| tab.id == tab_id))
                .and_then(|tab| tab.terminal.as_ref())
                .map(|terminal| terminal.read(cx).focus_handle().clone())
        {
            window.focus(&focus_handle, cx);
        }
        cx.notify();
    }

    /// 渲染所有全局模态框与浮层。
    pub(crate) fn render_overlays(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = Palette::from(cx);
        let mut overlays = Vec::new();
        if let Some(preview) = self.render_sftp_hover_preview(window, &p) {
            overlays.push(preview);
        }
        if self.modals.form.is_some() {
            overlays.push(self.render_session_form(window, cx, &p));
        }
        if self.modals.audit_viewer.is_some() {
            overlays.push(self.render_audit_viewer(window, cx, &p));
        }
        if self.modals.replay.is_some() {
            overlays.push(self.render_replay_modal(window, cx, &p));
        }
        if self.modals.folder_form.is_some() {
            overlays.push(self.render_folder_form(window, cx, &p));
        }
        if self.modals.auth_prompt.is_some() {
            overlays.push(self.render_auth_prompt(window, cx, &p));
        }
        if self.modals.host_key_confirm.is_some() {
            overlays.push(self.render_host_key_confirm(window, cx, &p));
        }
        if self.modals.known_hosts.open {
            overlays.push(self.render_known_hosts_modal(window, cx, &p));
        }
        if self.modals.confirm_forward_any.is_some() {
            overlays.push(self.render_forward_confirm_modal(window, cx, &p));
        }
        if self.modals.show_quick_open {
            overlays.push(self.render_quick_open_modal(window, cx, &p));
        }
        if self.modals.settings.open {
            overlays.push(self.render_settings_modal(window, cx, &p));
        }
        if self.shortcut_recording.capture.is_some() {
            overlays.push(self.render_shortcut_capture(cx, &p));
        }
        if self.modals.batch_edit.is_some() {
            overlays.push(self.render_batch_edit(window, cx, &p));
        }
        if self.modals.backup_prompt.is_some() {
            overlays.push(self.render_backup_prompt(window, cx, &p));
        }
        if self.modals.settings.key_wizard.is_some() {
            overlays.push(self.render_key_wizard(window, cx, &p));
        }
        if self.sftp_modals.chmod.is_some() {
            overlays.push(self.render_sftp_chmod_modal(window, cx, &p));
        }
        if self.sftp_modals.input.is_some() {
            overlays.push(self.render_sftp_input_modal(window, cx, &p));
        }
        if self.sftp_modals.properties.is_some() {
            overlays.push(self.render_sftp_properties_modal(window, cx, &p));
        }
        if self.sftp_modals.filter.is_some() {
            overlays.push(self.render_sftp_filter_modal(window, cx, &p));
        }
        if self.sftp_modals.sync.is_some() {
            overlays.push(self.render_sftp_sync_modal(window, cx, &p));
        }
        if self.sftp_modals.goto_line.is_some() {
            overlays.push(self.render_editor_goto_line_modal(window, cx, &p));
        }
        if self.modals.snippet_panel.form.is_some() {
            overlays.push(self.render_snippet_form(&p, cx));
        }
        overlays
    }
}
