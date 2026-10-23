//! GPUI 顶层应用外壳状态、快捷键动作委托与主布局容器。

use crate::UiServices;
use crate::forward::{ForwardPanelState, PendingForwardConfirm};
use crate::keymap;
use crate::keys::KeyWizardState;
use crate::palette::{Palette, localized_error};
use crate::session_form::SessionForm;
use crate::sftp::{
    SftpChmodModalState, SftpEditorModalState, SftpFilterModalState, SftpInputModalState,
    SftpPanelState, SftpPropertiesModalState, SftpSyncModalState,
};
use gpui_kit::base::h_flex;
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::InputState;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::*;
use std::collections::HashMap;
use std::sync::Arc;
use transom_core::{
    AppError, AppSettings, Connection, ConnectionId, ConnectionSecrets, FolderId, ForwardId,
    ForwardStatus, GroupId, HostKeyRecord, NetworkTraffic, PromptInfo, SavedTabGroup, Session,
    SessionCatalog, SessionId, SplitAxis, SystemStats, TabId,
};
use transom_terminal::TerminalView;

pub(crate) enum TabStatus {
    Connecting,
    Connected,
    Failed(AppError),
    Disconnected,
}

/// 标签组：一个标签栏条目下挂载一个或多个终端窗格。
///
/// 采用扁平窗格列表而非递归分割树：同一时刻只允许一种分屏方向，
/// 切换方向会整体重排所有窗格。这样在满足并排对照的同时，标签生命周期、
/// 焦点与连接事件仍保持简单可预测。
pub(crate) struct TabGroup {
    pub(crate) id: GroupId,
    pub(crate) panes: Vec<TabId>,
    pub(crate) active_pane: TabId,
    pub(crate) axis: SplitAxis,
}

impl TabGroup {
    pub(crate) fn new(pane: TabId) -> Self {
        Self {
            id: GroupId::new(),
            panes: vec![pane],
            active_pane: pane,
            axis: SplitAxis::Horizontal,
        }
    }
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
    /// 当前 SSH 连接的传输层实时速率；本地终端为 `None`。
    pub(crate) network_traffic: Option<NetworkTraffic>,
    /// 初始化命令的后台调度任务；重连或关标签时丢弃以取消。
    pub(crate) _startup_task: Option<Task<()>>,
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

/// 新建或重命名文件夹的轻量模态表单。
pub(crate) struct FolderForm {
    /// 存在时表示重命名该文件夹，否则为新建。
    pub(crate) base: Option<FolderId>,
    /// 新建时的父文件夹；`None` 表示根层级。
    pub(crate) parent: Option<FolderId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) error: Option<String>,
}

/// 快捷键录制弹窗状态。
pub(crate) struct ShortcutCapture {
    pub(crate) id: &'static str,
    pub(crate) error: Option<String>,
    pub(crate) focus: FocusHandle,
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
    /// 会话与文件夹共享同一份目录快照，任何一方变化都整体持久化。
    pub(crate) catalog: SessionCatalog,
    pub(crate) tabs: Vec<TerminalTab>,
    /// 标签栏显示顺序；成员为各标签下的窗格。
    pub(crate) groups: Vec<TabGroup>,
    pub(crate) active_tab: Option<TabId>,
    pub(crate) search: Entity<InputState>,
    pub(crate) form: Option<SessionForm>,
    pub(crate) folder_form: Option<FolderForm>,
    pub(crate) password_prompt: Option<PasswordPrompt>,
    pub(crate) auth_prompt: Option<AuthPromptState>,
    pub(crate) pending_connect: Option<PendingConnect>,
    pub(crate) show_known_hosts_modal: bool,
    pub(crate) known_hosts_records: Vec<HostKeyRecord>,
    pub(crate) known_hosts_search: Entity<InputState>,
    pub(crate) show_quick_open: bool,
    pub(crate) quick_open_state: Entity<CommandState>,
    pub(crate) show_settings_modal: bool,
    pub(crate) key_wizard: Option<KeyWizardState>,
    pub(crate) settings_font_size: Entity<InputState>,
    pub(crate) settings_line_height: Entity<InputState>,
    pub(crate) settings_scrollback: Entity<InputState>,
    pub(crate) confirm_forward_any: Option<PendingForwardConfirm>,
    pub(crate) sftp_editor_modal: Option<SftpEditorModalState>,
    pub(crate) sftp_chmod_modal: Option<SftpChmodModalState>,
    pub(crate) sftp_input_modal: Option<SftpInputModalState>,
    pub(crate) sftp_properties_modal: Option<SftpPropertiesModalState>,
    pub(crate) sftp_filter_modal: Option<SftpFilterModalState>,
    pub(crate) sftp_sync_modal: Option<SftpSyncModalState>,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) sidebar_visible: bool,
    /// 沉浸式全屏：隐藏全部应用外框，仅保留终端视口。
    pub(crate) immersive: bool,
    /// 当前生效的应用快捷键，终端据此截留按键而不是编码为控制字符。
    pub(crate) shortcut_filter: Arc<parking_lot::RwLock<Vec<KeybindingKeystroke>>>,
    /// 上次应用到键位表的有效键位，改键时用于抑制旧绑定。
    pub(crate) applied_shortcut_keys: Vec<String>,
    pub(crate) shortcut_capture: Option<ShortcutCapture>,
    pub(crate) _event_task: Task<()>,
}

impl AppShell {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        services: UiServices,
        settings: AppSettings,
    ) -> Self {
        cx.bind_keys(keymap::bindings(&settings.shortcuts));
        let shortcut_filter = Arc::new(parking_lot::RwLock::new(keymap::reserved_keystrokes(
            &settings.shortcuts,
        )));
        let applied_shortcut_keys = keymap::effective_key_strings(&settings.shortcuts);
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("app.search").to_string())
        });
        let quick_open_state = cx.new(|cx| CommandState::new(window, cx));
        let (catalog, error) = match services.session_store.load() {
            Ok(catalog) => (catalog, None),
            Err(error) => (SessionCatalog::default(), Some(localized_error(&error))),
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
            catalog,
            tabs: Vec::new(),
            groups: Vec::new(),
            active_tab: None,
            search,
            form: None,
            folder_form: None,
            password_prompt: None,
            auth_prompt: None,
            pending_connect: None,
            show_known_hosts_modal: false,
            known_hosts_records,
            known_hosts_search,
            show_quick_open: false,
            quick_open_state,
            show_settings_modal: false,
            key_wizard: None,
            settings_font_size,
            settings_line_height,
            settings_scrollback,
            confirm_forward_any: None,
            sftp_editor_modal: None,
            sftp_chmod_modal: None,
            sftp_input_modal: None,
            sftp_properties_modal: None,
            sftp_filter_modal: None,
            sftp_sync_modal: None,
            status: None,
            error,
            sidebar_visible: true,
            immersive: false,
            shortcut_filter,
            applied_shortcut_keys,
            shortcut_capture: None,
            _event_task: event_task,
        };

        if this.settings.window.restore_previous_tabs {
            let saved_groups = this.settings.window.previous_tab_groups.clone();
            if !saved_groups.is_empty() {
                for group in saved_groups {
                    this.restore_tab_group(&group, window, cx);
                }
            } else if !this.settings.window.previous_tab_sessions.is_empty() {
                let session_keys = this.settings.window.previous_tab_sessions.clone();
                for key in session_keys {
                    if key == "local" {
                        this.open_local_terminal(window, cx);
                    } else if let Some(session) = this
                        .catalog
                        .sessions
                        .iter()
                        .find(|s| s.id.to_string() == key)
                        .cloned()
                    {
                        this.connect_session(session.id, window, cx);
                    }
                }
            }
        }

        this
    }

    /// 持久化会话与文件夹目录。
    ///
    /// 统一入口避免各操作点漏存文件夹表；返回是否成功，调用方决定是否
    /// 继续后续动作（如选中项清理）。
    pub(crate) fn persist_catalog(&mut self) -> bool {
        match self.services.session_store.save(&self.catalog) {
            Ok(()) => true,
            Err(error) => {
                self.error = Some(localized_error(&error));
                false
            }
        }
    }

    /// 保存当前窗口几何尺寸、坐标以及打开的会话标签页列表
    pub(crate) fn save_window_state(&mut self, window: &mut Window) {
        // 全屏时的窗口几何是屏幕尺寸，写入设置会污染下次普通启动的窗口位置。
        if self.settings.window.remember_window_bounds && !window.is_fullscreen() {
            let bounds = window.bounds();
            self.settings.window.window_width = bounds.size.width / px(1.0);
            self.settings.window.window_height = bounds.size.height / px(1.0);
            self.settings.window.window_x = Some(bounds.origin.x / px(1.0));
            self.settings.window.window_y = Some(bounds.origin.y / px(1.0));
        }
        self.settings.window.previous_tab_sessions = self.tabs.iter().map(session_key).collect();
        self.settings.window.previous_tab_groups = self
            .groups
            .iter()
            .filter_map(|group| {
                let tab = self.tabs.iter().find(|tab| tab.id == group.active_pane)?;
                Some(SavedTabGroup {
                    session: session_key(tab),
                    panes: group.panes.len(),
                    axis: group.axis,
                })
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

    /// 切换沉浸式全屏：同步系统窗口全屏状态与应用外框显隐。
    ///
    /// 侧边栏、SFTP 与转发面板只做“隐藏渲染”而不清除自身开关状态，
    /// 退出沉浸模式后能原样恢复用户此前的工作布局。
    pub(crate) fn on_toggle_immersive(
        &mut self,
        _: &keymap::ToggleImmersive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.immersive = !self.immersive;
        window.toggle_fullscreen();
        if self.immersive
            && let Some(focus_handle) = self
                .active_tab
                .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
                .and_then(|tab| tab.terminal.as_ref())
                .map(|terminal| terminal.read(cx).focus_handle().clone())
        {
            window.focus(&focus_handle, cx);
        }
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
        if self.folder_form.is_some() {
            overlays.push(self.render_folder_form(window, cx, &p));
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
        if self.shortcut_capture.is_some() {
            overlays.push(self.render_shortcut_capture(cx, &p));
        }
        if self.key_wizard.is_some() {
            overlays.push(self.render_key_wizard(window, cx, &p));
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
        if self.sftp_sync_modal.is_some() {
            overlays.push(self.render_sftp_sync_modal(window, cx, &p));
        }
        overlays
    }
}

/// 窗口恢复使用的会话键：本地终端固定为 `local`，其余使用持久化会话 ID。
pub(crate) fn session_key(tab: &TerminalTab) -> String {
    if matches!(tab.session.connection, Connection::Local(_)) {
        "local".to_string()
    } else {
        tab.session_id.to_string()
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let top_bar = (!self.immersive).then(|| self.render_top_bar(window, cx, &p));
        let activity_bar = (!self.immersive).then(|| self.render_activity_bar(window, cx, &p));

        // 沉浸模式使用独立分组：侧边栏完全不参与布局，主面板成为首面板，
        // 避免其左侧残留一条用于拖动侧边栏的 1px 分隔线；
        // 独立 key 也保证退出沉浸后原分组的全部尺寸状态原样恢复。
        let resizable_id = if self.immersive {
            "workspace-panels-immersive"
        } else {
            "workspace-panels"
        };
        let mut resizable_group = h_resizable(resizable_id);
        if !self.immersive {
            // ResizableState 按面板索引记录尺寸：折叠时若移除首个面板，
            // 主终端会继承侧边栏的固定尺寸，再次展开时布局被撑满。
            // 因此面板始终参与分组，仅切换可见性以保持索引稳定。
            let mut sidebar_panel = resizable_panel()
                .size(px(270.0))
                .fixed()
                .visible(self.sidebar_visible);
            if self.sidebar_visible {
                sidebar_panel = sidebar_panel.child(self.render_sidebar(window, cx, &p));
            }
            resizable_group = resizable_group.child(sidebar_panel);
        }
        resizable_group =
            resizable_group.child(resizable_panel().child(self.render_main(window, cx, &p)));
        let show_sftp = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .is_some_and(|tab| tab.sftp.is_some());
        if show_sftp && !self.immersive {
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
        if show_forwards && !self.immersive {
            resizable_group = resizable_group.child(
                resizable_panel()
                    .size(px(340.0))
                    .fixed()
                    .child(self.render_forward_panel(window, cx, &p)),
            );
        }

        let mut main_row = h_flex().flex_1().overflow_hidden();
        if !self.immersive {
            // 与顶栏、状态栏、活动栏的边框一起围合主内容区，补齐右侧轮廓。
            main_row = main_row.border_r_1().border_color(p.border);
        }
        if let Some(activity_bar) = activity_bar {
            main_row = main_row.child(activity_bar);
        }
        main_row = main_row.child(
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
        let status_bar = (!self.immersive).then(|| self.render_status_bar(&p));
        let overlays = self.render_overlays(window, cx);

        let mut root = div()
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
            .on_action(cx.listener(Self::on_toggle_immersive))
            .on_action(cx.listener(Self::on_split_right))
            .on_action(cx.listener(Self::on_split_down))
            .on_action(cx.listener(Self::on_close_pane))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.background)
            .text_color(p.foreground);
        if let Some(top_bar) = top_bar {
            root = root.child(top_bar);
        }
        root = root.child(main_row);
        if let Some(error_banner) = error_banner {
            root = root.child(error_banner);
        }
        if let Some(status_bar) = status_bar {
            root = root.child(status_bar);
        }
        root.children(overlays)
    }
}
