//! GPUI 顶层应用外壳状态与生命周期主入口。

// 功能模块的真实父模块是 `app`，文件仍留在原目录。
//
// 设计原理（Why）：AppShell 的字段只应对应用外壳及其功能模块可见。
// 若把功能模块继续挂在 crate 根，它们与 AppShell 是兄弟模块，只能通过
// `pub(crate)` 字段访问状态，封装形同虚设。用 `#[path]` 把模块挂到 `app`
// 之下后，字段可以收成模块私有，同时不必搬动文件；crate 根再重导出旧路径，
// 现有 `crate::sftp` / `crate::settings` 引用无需批量改写。
#[path = "../activity_bar/mod.rs"]
pub(crate) mod activity_bar;
#[path = "../audit_viewer/mod.rs"]
pub(crate) mod audit_viewer;
#[path = "../backup/mod.rs"]
pub(crate) mod backup;
#[path = "../batch.rs"]
pub(crate) mod batch;
#[path = "../broadcast/mod.rs"]
pub(crate) mod broadcast;
#[path = "../chrome.rs"]
pub(crate) mod chrome;
#[path = "../clipboard_prompt.rs"]
pub(crate) mod clipboard_prompt;
#[path = "../cloud_sync.rs"]
pub(crate) mod cloud_sync;
#[path = "../command_palette/mod.rs"]
pub(crate) mod command_palette;
#[path = "../connection/mod.rs"]
pub(crate) mod connection;
#[path = "../dialog_host.rs"]
pub(crate) mod dialog_host;
#[path = "../diff_viewer/mod.rs"]
pub(crate) mod diff_viewer;
#[path = "../editor/mod.rs"]
pub(crate) mod editor;
#[path = "../event_handlers.rs"]
pub(crate) mod event_handlers;
#[path = "../events.rs"]
pub(crate) mod events;
#[path = "../forward/mod.rs"]
pub(crate) mod forward;
#[path = "../identity/mod.rs"]
pub(crate) mod identity;
#[path = "../keys/mod.rs"]
pub(crate) mod keys;
#[path = "../modals.rs"]
pub(crate) mod modals;
#[path = "../monitor/mod.rs"]
pub(crate) mod monitor;
#[path = "../notify.rs"]
pub(crate) mod notify;
#[path = "../probe.rs"]
pub(crate) mod probe;
#[path = "../record/mod.rs"]
pub(crate) mod record;
#[path = "../app_render.rs"]
mod render;
#[path = "../session_appearance.rs"]
pub(crate) mod session_appearance;
#[path = "../session_form/mod.rs"]
pub(crate) mod session_form;
#[path = "../session_serial.rs"]
pub(crate) mod session_serial;
#[path = "../settings/mod.rs"]
pub(crate) mod settings;
#[path = "../sftp/mod.rs"]
pub(crate) mod sftp;
#[path = "../sftp_handlers.rs"]
pub(crate) mod sftp_handlers;
#[path = "../shortcuts.rs"]
pub(crate) mod shortcuts;
#[path = "../sidebar/mod.rs"]
pub(crate) mod sidebar;
#[path = "../snippets/mod.rs"]
pub(crate) mod snippets;
#[path = "../split.rs"]
pub(crate) mod split;
#[path = "../startup.rs"]
pub(crate) mod startup;
#[path = "../status_bar/mod.rs"]
pub(crate) mod status_bar;
#[path = "../tabs/mod.rs"]
pub(crate) mod tabs;
#[path = "../terminal_copy_mode.rs"]
pub(crate) mod terminal_copy_mode;
#[path = "../terminal_search.rs"]
pub(crate) mod terminal_search;
#[path = "../tiling/mod.rs"]
pub(crate) mod tiling;
#[path = "../trigger_panel/mod.rs"]
pub(crate) mod trigger_panel;
#[path = "../window_state.rs"]
pub(crate) mod window_state;
#[path = "../workspace/mod.rs"]
pub(crate) mod workspace;
#[path = "../zmodem.rs"]
pub(crate) mod zmodem;

mod modal_hub;
mod overlays;
mod secret_prefill;
mod state;
mod task_hub;
mod types;

pub(crate) use modal_hub::ModalHub;
pub(crate) use secret_prefill::prefill_secret_fields;
pub(crate) use state::AppState;
pub(crate) use task_hub::TaskHub;

use self::broadcast::BroadcastBarState;
use self::cloud_sync::GitSyncScheduler;
use self::editor::EditorManager;
use self::keys::KnownHostsModalState;
use self::settings::SettingsModalState;
use self::sftp::SftpModals;
use self::sidebar::SidebarState;
use self::snippets::SnippetPanelState;
use self::workspace::WorkspaceState;
use crate::UiServices;
use crate::keymap;
use crate::keymap::ShortcutRecordingState;
use crate::palette::localized_error;
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::component::slider::{SliderEvent, SliderValue};
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use transom_core::{
    AppCommand, AppSettings, PanelSide, SessionCatalog, StartupTabs, TabPosition, UiThemeMode,
};

pub(crate) use types::*;

/// 命令发送失败的界面文案：队列满提示稍后重试，控制器退出提示重启应用。
pub(crate) fn command_send_error_text(error: flume::TrySendError<AppCommand>) -> String {
    match error {
        flume::TrySendError::Full(_) => rust_i18n::t!("error.command_queue_full").to_string(),
        flume::TrySendError::Disconnected(_) => {
            rust_i18n::t!("error.controller_stopped").to_string()
        }
    }
}

pub(crate) struct AppShell {
    /// 全部应用状态集中存放；外壳只负责渲染、事件路由与生命周期。
    state: AppState,
}

impl std::ops::Deref for AppShell {
    type Target = AppState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl std::ops::DerefMut for AppShell {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl AppShell {
    /// 设置工具面板（SFTP/转发）停靠方向。
    pub(crate) fn set_sftp_panel_side(&mut self, side: PanelSide, cx: &mut Context<Self>) {
        if self.settings.window.sftp_panel_side == side {
            return;
        }
        self.settings.window.sftp_panel_side = side;
        let state = &mut self.state;
        std::mem::swap(
            &mut state.workspace_panels,
            &mut state.workspace_panels_left,
        );
        self.persist_layout_settings(cx);
    }

    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        services: UiServices,
        mut settings: AppSettings,
    ) -> Self {
        settings.activity_bar.normalize();
        settings.status_bar.normalize();
        settings.appearance.normalize();
        settings.window.normalize();
        cx.bind_keys(keymap::bindings(&settings.shortcuts));
        let shortcut_filter = Arc::new(parking_lot::RwLock::new(keymap::reserved_keystrokes(
            &settings.shortcuts,
        )));
        let applied_shortcut_keys = keymap::effective_key_strings(&settings.shortcuts);
        let history: Arc<parking_lot::RwLock<Vec<String>>> =
            Arc::new(parking_lot::RwLock::new(if settings.history.enabled {
                services.history.load()
            } else {
                Vec::new()
            }));
        let history_provider: transom_terminal::HistoryProvider = {
            let snapshot = Arc::clone(&history);
            Arc::new(move |prefix: &str| {
                transom_core::command_suggestions(
                    &snapshot.read(),
                    prefix,
                    transom_core::SUGGESTION_LIMIT,
                )
            })
        };
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("app.search").to_string())
        });
        let quick_open_state = cx.new(|cx| CommandState::new(window, cx));
        let broadcast_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("broadcast.single_placeholder").to_string())
        });
        let broadcast_textarea = cx.new(|cx| {
            TextareaState::new(window, cx)
                .submit_on_enter(true)
                .placeholder(rust_i18n::t!("broadcast.multi_placeholder").to_string())
        });
        let vertical_tabs_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("tab.vertical_search").to_string())
        });
        let workspace_panels = cx.new(|_| ResizableState::default());
        let workspace_panels_left = cx.new(|_| ResizableState::default());
        cx.observe(&workspace_panels, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&workspace_panels_left, |_, _, cx| cx.notify())
            .detach();
        let (catalog, catalog_error) = match services.session_store.load() {
            Ok(catalog) => (catalog, None),
            Err(error) => {
                log::error!(
                    "读取会话目录失败，界面按空目录启动，原因: {}",
                    localized_error(&error)
                );
                (SessionCatalog::default(), Some(localized_error(&error)))
            }
        };
        let mut pending_notices = PendingNotices::new();
        if let Some(error) = catalog_error {
            pending_notices.push(error);
        }
        log::info!(
            "会话目录就绪，会话数: {}，分组数: {}，凭据模板数: {}",
            catalog.sessions.len(),
            catalog.folders.len(),
            catalog.identities.len()
        );

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

        let app_entity = cx.entity();
        let appearance_subscription = window.observe_window_appearance(move |window, cx| {
            let follows_system =
                app_entity.read(cx).settings.appearance.mode == UiThemeMode::System;
            if follows_system {
                crate::theme::sync_with_system(window, cx);
            }
        });

        let known_hosts_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("app.search").to_string())
        });
        let known_hosts_records = services.known_hosts.list().unwrap_or_default();

        let settings_modal = SettingsModalState::new(window, cx, &settings);
        let settings_window_transparency = settings_modal.window_transparency.clone();
        let window_transparency_sub = cx.subscribe_in(
            &settings_window_transparency,
            window,
            |this, _, event: &SliderEvent, window, cx| match event {
                SliderEvent::Change(SliderValue::Single(value)) => {
                    this.set_window_transparency(value.round() as u8, window, cx);
                }
                SliderEvent::Release(SliderValue::Single(value)) => {
                    this.set_window_transparency(value.round() as u8, window, cx);
                    this.push_window_appearance(cx);
                }
                _ => {}
            },
        );
        // 分割线粗细步进器：手输与加减按钮都发 Change，统一走即时预览；
        // 弹窗打开时的回填同样会触发一次，值相同时不产生任何副作用。
        let settings_divider_width = settings_modal.divider_width.clone();
        let divider_width_sub = cx.subscribe_in(
            &settings_divider_width,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.apply_divider_width_from_input(cx);
                }
            },
        );
        let terminal_appearance_subscriptions = vec![window_transparency_sub, divider_width_sub];
        let sidebar_visible = settings.window.sidebar_visible_at_startup();
        // 左侧标签模式启动即以标签面板作为槽位主视图；顶部模式该标志无效。
        let vertical_tabs_open = settings.window.tab_position == TabPosition::Left;
        let monitor = crate::monitor::MonitorState::new(window, &settings.monitor, cx);

        let state = AppState {
            services,
            settings,
            catalog,
            catalog_revision: 0,
            sidebar: SidebarState::new(sidebar_visible, search),
            workspace: WorkspaceState::new(),
            workspace_panels,
            workspace_panels_left,
            vertical_tabs_search,
            vertical_tabs_open,
            modals: ModalHub::new(
                KnownHostsModalState {
                    open: false,
                    records: known_hosts_records,
                    search: known_hosts_search,
                },
                settings_modal,
                quick_open_state,
                SnippetPanelState {
                    open: false,
                    form: None,
                },
            ),
            tasks: TaskHub::new(
                appearance_subscription,
                terminal_appearance_subscriptions,
                event_task,
            ),
            git_sync_schedule: GitSyncScheduler::default(),
            history,
            history_provider,
            probe_badges: HashMap::new(),
            monitor,
            editors: EditorManager::new(),
            diff_viewer: None,
            sftp_modals: SftpModals {
                chmod: None,
                confirm: None,
                input: None,
                properties: None,
                filter: None,
                sync: None,
                goto_line: None,
                search: None,
            },
            broadcast: BroadcastBarState {
                open: false,
                multiline: false,
                targets_open: false,
                input: broadcast_input,
                textarea: broadcast_textarea,
                targets: HashSet::new(),
                targets_scope: Vec::new(),
            },
            status: None,
            pending_notices,
            immersive: false,
            sftp_panel_width_target: None,
            shortcut_recording: ShortcutRecordingState {
                filter: shortcut_filter,
                applied_keys: applied_shortcut_keys,
                capture: None,
            },
            open_modals: crate::dialog_host::OpenModals::default(),
            dialog_hint: None,
        };
        let mut this = Self { state };

        cx.subscribe_in(
            &this.broadcast.input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    this.broadcast_send(window, cx);
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &this.broadcast.textarea,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    this.broadcast_send(window, cx);
                }
            },
        )
        .detach();
        // 搜索词变化时整壳重绘，纵向标签列表的过滤结果才会跟着输入实时更新。
        cx.subscribe(
            &this.vertical_tabs_search,
            |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        )
        .detach();

        if this.settings.appearance.mode == UiThemeMode::System {
            crate::theme::sync_with_system(window, cx);
        }
        this.sync_terminal_palette(cx);
        this.restore_startup_tabs(window, cx);

        this.tasks.git_sync = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(60))
                    .await;
                if this.update(cx, |this, cx| this.tick_git_sync(cx)).is_err() {
                    break;
                }
            }
        }));

        this
    }

    /// 按启动档位恢复工作台。
    fn restore_startup_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.settings.window.startup_tabs {
            StartupTabs::RestoreLast => {
                let saved_groups = self.settings.window.previous_tab_groups.clone();
                let saved_tilings = self.settings.window.tab_tilings.clone();
                if !saved_groups.is_empty() {
                    let mut restored = Vec::with_capacity(saved_groups.len());
                    for group in &saved_groups {
                        restored.push(self.restore_tab_group(group, window, cx));
                    }
                    let mut focus = None;
                    for saved in &saved_tilings {
                        if let Some(group) = self.restore_tab_tiling(saved, &restored) {
                            focus.get_or_insert(group);
                        }
                    }
                    if let Some(group) = focus
                        && let Some(index) = self.workspace.group_index(group)
                    {
                        let pane = self.workspace.groups[index].active_pane;
                        self.activate_tab(pane, window, cx);
                    }
                } else if !self.settings.window.previous_tab_sessions.is_empty() {
                    let session_keys = self.settings.window.previous_tab_sessions.clone();
                    for key in session_keys {
                        if key == "local" {
                            self.open_local_terminal(window, cx);
                        } else if let Some(session) = self
                            .catalog
                            .sessions
                            .iter()
                            .find(|s| s.id.to_string() == key)
                            .cloned()
                        {
                            self.connect_session(session.id, window, cx);
                        }
                    }
                }
            }
            StartupTabs::EmptyWorkspace => {}
            StartupTabs::NewLocalTerminal => self.open_local_terminal(window, cx),
        }
    }

    /// 每分钟节拍：达到配置间隔时触发一次后台 Git 同步。
    fn tick_git_sync(&mut self, cx: &mut Context<Self>) {
        let interval = self
            .settings
            .sync
            .git
            .as_ref()
            .map_or(0, |git| git.interval_minutes);
        if self.git_sync_schedule.tick(interval) {
            self.start_git_sync_scheduled(cx);
        }
    }
}
