//! GPUI 顶层应用外壳状态、快捷键动作委托与主布局容器。

use crate::UiServices;
use crate::forward::{ForwardPanelState, PendingForwardConfirm};
use crate::keymap;
use crate::palette::{Palette, localized_error};
use crate::session_form::SessionForm;
use crate::sftp::{
    SftpChmodModalState, SftpEditorModalState, SftpFilterModalState, SftpInputModalState,
    SftpPanelState, SftpPropertiesModalState,
};
use gpui_kit::base::h_flex;
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::InputState;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::*;
use std::collections::HashMap;
use transom_core::{
    AppError, AppSettings, Connection, ConnectionId, ConnectionSecrets, ForwardId, ForwardStatus,
    HostKeyRecord, PromptInfo, Session, SessionId, SystemStats, TabId,
};
use transom_terminal::TerminalView;

pub(crate) enum TabStatus {
    Connecting,
    Connected,
    Failed(AppError),
    Disconnected,
}

pub(crate) struct TerminalTab {
    pub(crate) id: TabId,
    pub(crate) session_id: SessionId,
    pub(crate) session: Session,
    pub(crate) connection_id: Option<ConnectionId>,
    pub(crate) title: String,
    pub(crate) status: TabStatus,
    pub(crate) terminal: Option<Entity<TerminalView>>,
    pub(crate) sftp: Option<SftpPanelState>,
    pub(crate) forwards_panel: Option<ForwardPanelState>,
    pub(crate) forward_statuses: HashMap<ForwardId, ForwardStatus>,
    pub(crate) system_stats: Option<SystemStats>,
    pub(crate) _subscription: Option<Subscription>,
}

pub(crate) struct PendingConnect {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) session: Session,
    pub(crate) secrets: ConnectionSecrets,
}

pub(crate) struct PasswordPrompt {
    pub(crate) input: Entity<InputState>,
    pub(crate) error: Option<String>,
}

pub(crate) struct AuthPromptState {
    pub(crate) name: String,
    pub(crate) instructions: String,
    pub(crate) prompts: Vec<PromptInfo>,
    pub(crate) inputs: Vec<Entity<InputState>>,
    pub(crate) responder: flume::Sender<Vec<String>>,
    pub(crate) error: Option<String>,
}

pub struct AppShell {
    pub(crate) services: UiServices,
    pub(crate) settings: AppSettings,
    pub(crate) sessions: Vec<Session>,
    pub(crate) tabs: Vec<TerminalTab>,
    pub(crate) active_tab: Option<TabId>,
    pub(crate) search: Entity<InputState>,
    pub(crate) form: Option<SessionForm>,
    pub(crate) password_prompt: Option<PasswordPrompt>,
    pub(crate) auth_prompt: Option<AuthPromptState>,
    pub(crate) pending_connect: Option<PendingConnect>,
    pub(crate) show_known_hosts_modal: bool,
    pub(crate) known_hosts_records: Vec<HostKeyRecord>,
    pub(crate) known_hosts_search: Entity<InputState>,
    pub(crate) show_quick_open: bool,
    pub(crate) quick_open_state: Entity<CommandState>,
    pub(crate) show_settings_modal: bool,
    pub(crate) settings_font_size: Entity<InputState>,
    pub(crate) settings_line_height: Entity<InputState>,
    pub(crate) settings_scrollback: Entity<InputState>,
    pub(crate) confirm_forward_any: Option<PendingForwardConfirm>,
    pub(crate) sftp_editor_modal: Option<SftpEditorModalState>,
    pub(crate) sftp_chmod_modal: Option<SftpChmodModalState>,
    pub(crate) sftp_input_modal: Option<SftpInputModalState>,
    pub(crate) sftp_properties_modal: Option<SftpPropertiesModalState>,
    pub(crate) sftp_filter_modal: Option<SftpFilterModalState>,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) sidebar_visible: bool,
    pub(crate) _event_task: Task<()>,
}

impl AppShell {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        services: UiServices,
        settings: AppSettings,
    ) -> Self {
        cx.bind_keys(keymap::bindings());
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("app.search").to_string())
        });
        let quick_open_state = cx.new(|cx| CommandState::new(window, cx));
        let (sessions, error) = match services.session_store.load() {
            Ok(sessions) => (sessions, None),
            Err(error) => (Vec::new(), Some(localized_error(&error))),
        };

        let event_rx = services.event_rx.clone();
        let event_task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = event_rx.recv_async().await {
                if this
                    .update_in(cx, |view, window, cx| {
                        view.handle_app_event(event, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let known_hosts_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("app.search").to_string())
        });
        let known_hosts_records = services.known_hosts.list().unwrap_or_default();

        let font_size_val = settings.terminal_profile.font_size.to_string();
        let settings_font_size = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("14")
                .default_value(&font_size_val)
        });
        let line_height_val = settings.terminal_profile.line_height.to_string();
        let settings_line_height = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("1.2")
                .default_value(&line_height_val)
        });
        let scrollback_val = settings.terminal_profile.scrollback.to_string();
        let settings_scrollback = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("10000")
                .default_value(&scrollback_val)
        });

        let mut this = Self {
            services,
            settings,
            sessions,
            tabs: Vec::new(),
            active_tab: None,
            search,
            form: None,
            password_prompt: None,
            auth_prompt: None,
            pending_connect: None,
            show_known_hosts_modal: false,
            known_hosts_records,
            known_hosts_search,
            show_quick_open: false,
            quick_open_state,
            show_settings_modal: false,
            settings_font_size,
            settings_line_height,
            settings_scrollback,
            confirm_forward_any: None,
            sftp_editor_modal: None,
            sftp_chmod_modal: None,
            sftp_input_modal: None,
            sftp_properties_modal: None,
            sftp_filter_modal: None,
            status: None,
            error,
            sidebar_visible: true,
            _event_task: event_task,
        };

        if this.settings.window.restore_previous_tabs
            && !this.settings.window.previous_tab_sessions.is_empty()
        {
            let session_keys = this.settings.window.previous_tab_sessions.clone();
            for key in session_keys {
                if key == "local" {
                    this.open_local_terminal(window, cx);
                } else if let Some(session) = this
                    .sessions
                    .iter()
                    .find(|s| s.id.to_string() == key)
                    .cloned()
                {
                    this.connect_session(session.id, window, cx);
                }
            }
        }

        this
    }

    /// 保存当前窗口几何尺寸、坐标以及打开的会话标签页列表
    pub(crate) fn save_window_state(&mut self, window: &mut Window) {
        if self.settings.window.remember_window_bounds {
            let bounds = window.bounds();
            self.settings.window.window_width = bounds.size.width / px(1.0);
            self.settings.window.window_height = bounds.size.height / px(1.0);
            self.settings.window.window_x = Some(bounds.origin.x / px(1.0));
            self.settings.window.window_y = Some(bounds.origin.y / px(1.0));
        }
        self.settings.window.previous_tab_sessions = self
            .tabs
            .iter()
            .map(|t| {
                if matches!(t.session.connection, Connection::Local(_)) {
                    "local".to_string()
                } else {
                    t.session_id.to_string()
                }
            })
            .collect();
        let _ = self.services.settings_store.save(&self.settings);
    }

    /// 关闭程序前保存状态并退出
    pub(crate) fn close_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_window_state(window);
        cx.quit();
    }

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
        if let Some(tab_id) = self.active_tab {
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
        if let Some(tab_id) = self.active_tab {
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

    pub(crate) fn render_overlays(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = Palette::from(cx);
        let mut overlays = Vec::new();
        if self.form.is_some() {
            overlays.push(self.render_session_form(window, cx, &p));
        }
        if self.password_prompt.is_some() {
            overlays.push(self.render_password_prompt(window, cx, &p));
        }
        if self.auth_prompt.is_some() {
            overlays.push(self.render_auth_prompt(window, cx, &p));
        }
        if self.show_known_hosts_modal {
            overlays.push(self.render_known_hosts_modal(window, cx, &p));
        }
        if self.confirm_forward_any.is_some() {
            overlays.push(self.render_forward_confirm_modal(window, cx, &p));
        }
        if self.show_quick_open {
            overlays.push(self.render_quick_open_modal(window, cx, &p));
        }
        if self.show_settings_modal {
            overlays.push(self.render_settings_modal(window, cx, &p));
        }
        if self.sftp_editor_modal.is_some() {
            overlays.push(self.render_sftp_editor_modal(window, cx, &p));
        }
        if self.sftp_chmod_modal.is_some() {
            overlays.push(self.render_sftp_chmod_modal(window, cx, &p));
        }
        if self.sftp_input_modal.is_some() {
            overlays.push(self.render_sftp_input_modal(window, cx, &p));
        }
        if self.sftp_properties_modal.is_some() {
            overlays.push(self.render_sftp_properties_modal(window, cx, &p));
        }
        if self.sftp_filter_modal.is_some() {
            overlays.push(self.render_sftp_filter_modal(window, cx, &p));
        }
        overlays
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let top_bar = self.render_top_bar(window, cx, &p);
        let activity_bar = self.render_activity_bar(window, cx, &p);

        let mut resizable_group = h_resizable("workspace-panels");
        if self.sidebar_visible {
            resizable_group = resizable_group.child(
                resizable_panel()
                    .size(px(270.0))
                    .fixed()
                    .child(self.render_sidebar(window, cx, &p)),
            );
        }
        resizable_group =
            resizable_group.child(resizable_panel().child(self.render_main(window, cx, &p)));
        let show_sftp = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .is_some_and(|tab| tab.sftp.is_some());
        if show_sftp {
            resizable_group = resizable_group.child(
                resizable_panel()
                    .size(px(760.0))
                    .fixed()
                    .child(self.render_sftp_panel(window, cx, &p)),
            );
        }
        let show_forwards = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .is_some_and(|tab| tab.forwards_panel.is_some());
        if show_forwards {
            resizable_group = resizable_group.child(
                resizable_panel()
                    .size(px(340.0))
                    .fixed()
                    .child(self.render_forward_panel(window, cx, &p)),
            );
        }

        let main_row = h_flex()
            .flex_1()
            .overflow_hidden()
            .child(activity_bar)
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .overflow_hidden()
                    .child(resizable_group),
            );
        let error_banner = self
            .error
            .as_ref()
            .map(|_| self.render_error_banner(&p, cx));
        let status_bar = self.render_status_bar(&p);
        let overlays = self.render_overlays(window, cx);

        div()
            .key_context(keymap::WORKSPACE_CONTEXT)
            .on_action(cx.listener(Self::on_new_session))
            .on_action(cx.listener(Self::on_new_local_terminal))
            .on_action(cx.listener(Self::on_quick_open))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_prev_tab))
            .on_action(cx.listener(Self::on_reconnect))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_toggle_sftp))
            .on_action(cx.listener(Self::on_toggle_forward))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.background)
            .text_color(p.foreground)
            .child(top_bar)
            .child(main_row)
            .children(error_banner)
            .child(status_bar)
            .children(overlays)
    }
}
