//! AppShell 构造：存储加载、实体创建、订阅与启动恢复。

use super::*;
use crate::app::app_settings;
use crate::app::update_app_settings;

impl AppShell {
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
        // 设置与进程同生命周期，装进 Global；外壳不再持字段。
        install_app_settings(cx, settings.clone());
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
        let inputs = BootstrapInputs::new(window, cx);
        cx.observe(&inputs.workspace_panels, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&inputs.workspace_panels_left, |_, _, cx| cx.notify())
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
        let appearance_app = app_entity.clone();
        let appearance_subscription = window.observe_window_appearance(move |window, cx| {
            let follows_system =
                update_app_settings(cx, |s| s.appearance.mode == UiThemeMode::System);
            if follows_system {
                crate::theme::sync_with_system(window, cx);
                if crate::settings::sync_terminal_palette(cx) {
                    appearance_app.update(cx, |shell, cx| shell.apply_appearance_to_live_panes(cx));
                }
            }
        });

        let known_hosts_records = services.known_hosts.list().unwrap_or_default();

        let persistence = Persistence::new(
            services.session_store.clone(),
            services.settings_store.clone(),
        );
        let settings_view = cx.new(|cx| {
            crate::settings::SettingsView::new(
                window,
                cx,
                &settings,
                crate::settings::SettingsServices {
                    secret_store: services.secret_store.clone(),
                    persistence: persistence.clone(),
                    history: history.clone(),
                    history_store: services.history.clone(),
                },
                catalog.clone(),
            )
        });
        // 弹窗内部的滑块/步进器订阅由设置实体自己持有，外壳不再感知这些控件。
        let sidebar_visible = settings.window.sidebar_visible_at_startup();
        // 左侧标签模式启动即以标签面板作为槽位主视图；顶部模式该标志无效。
        let vertical_tabs_open = settings.window.tab_position == TabPosition::Left;
        let monitor = cx.new(|cx| crate::monitor::MonitorView::new(window, &settings.monitor, cx));
        let editors = cx.new(|_| EditorPanels::new());

        let state = AppState {
            services,
            catalog,
            catalog_revision: 0,
            sidebar: SidebarState::new(sidebar_visible, inputs.search.clone()),
            workspace: WorkspaceState::new(),
            workspace_panels: inputs.workspace_panels.clone(),
            workspace_panels_left: inputs.workspace_panels_left.clone(),
            vertical_tabs_search: inputs.vertical_tabs_search.clone(),
            vertical_tabs_open,
            modals: ModalHub::new(
                KnownHostsModalState {
                    open: false,
                    records: known_hosts_records,
                    search: inputs.known_hosts_search.clone(),
                },
                settings_view,
                inputs.quick_open_state.clone(),
                SnippetPanelState {
                    open: false,
                    form: None,
                },
            ),
            tasks: TaskHub::new(appearance_subscription, event_task),
            git_sync_schedule: GitSyncScheduler::default(),
            history,
            history_provider,
            probe_badges: HashMap::new(),
            monitor,
            editors,
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
                input: inputs.broadcast_input.clone(),
                textarea: inputs.broadcast_textarea.clone(),
                targets: HashSet::new(),
                targets_scope: Vec::new(),
            },
            status: None,
            persistence,
            pending_notices,
            immersive: false,
            sftp_panel_width_target: None,
            window_requests: Default::default(),
            key_wizard: None,
            stats_sampling: true,
            shortcut_recording: ShortcutRecordingState {
                filter: shortcut_filter,
                applied_keys: applied_shortcut_keys,
                capture: None,
            },
            open_modals: crate::dialog_host::OpenModals::default(),
            dialog_hint: None,
        };
        let mut this = Self { state };
        this.bootstrap_subscriptions(window, cx);

        this.bootstrap_startup(window, cx);

        this
    }
    /// 安装面板事件订阅与输入框联动；全部订阅由 `TaskHub` 托管生命周期。
    fn bootstrap_subscriptions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 监控面板的意图事件由外壳订阅处理。
        let monitor_subscription = cx.subscribe(
            &self.state.monitor,
            |shell, _view, event: &crate::monitor::MonitorPanelEvent, cx| {
                shell.on_monitor_panel_event(event, cx);
            },
        );
        self.state.tasks.subscriptions.monitor = Some(monitor_subscription);
        let settings_subscription = cx.subscribe(
            &self.state.modals.settings,
            |shell, _view, event: &crate::settings::SettingsEvent, cx| {
                shell.on_settings_event(event, cx);
            },
        );
        self.state.tasks.subscriptions.settings = Some(settings_subscription);
        // 编辑器面板的意图事件由外壳订阅处理；命令通道在创建时注入，
        // 面板自己投递 SFTP 命令，不需要回指外壳。
        let editor_command_tx = self.state.services.command_tx.clone();
        let editor_subscription = cx.subscribe(
            &self.state.editors,
            |shell, _panels, event: &crate::editor::EditorPanelEvent, cx| {
                shell.on_editor_panel_event(event, cx);
            },
        );
        self.state.tasks.subscriptions.editor = Some(editor_subscription);
        self.state
            .editors
            .update(cx, |panels, _| panels.set_command_tx(editor_command_tx));

        cx.subscribe_in(
            &self.state.broadcast.input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    this.broadcast_send(window, cx);
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &self.state.broadcast.textarea,
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
            &self.state.vertical_tabs_search,
            |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        )
        .detach();
    }

    /// 启动期主题同步、标签恢复与 Git 同步定时器。
    fn bootstrap_startup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if app_settings(cx).appearance.mode == UiThemeMode::System {
            crate::theme::sync_with_system(window, cx);
        }
        crate::settings::sync_terminal_palette(cx);
        self.restore_startup_tabs(window, cx);

        self.state.tasks.background.git_sync = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(60))
                    .await;
                if this.update(cx, |this, cx| this.tick_git_sync(cx)).is_err() {
                    break;
                }
            }
        }));
    }
}
/// 构造期一次性创建的输入框/面板句柄，避免 `new` 内平铺二十个同名变量。
struct BootstrapInputs {
    search: Entity<InputState>,
    quick_open_state: Entity<CommandState>,
    broadcast_input: Entity<InputState>,
    broadcast_textarea: Entity<TextareaState>,
    vertical_tabs_search: Entity<InputState>,
    workspace_panels: Entity<ResizableState>,
    workspace_panels_left: Entity<ResizableState>,
    known_hosts_search: Entity<InputState>,
}

impl BootstrapInputs {
    fn new(window: &mut Window, cx: &mut Context<AppShell>) -> Self {
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
        let known_hosts_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("app.search").to_string())
        });
        Self {
            search,
            quick_open_state,
            broadcast_input,
            broadcast_textarea,
            vertical_tabs_search,
            workspace_panels,
            workspace_panels_left,
            known_hosts_search,
        }
    }
}
