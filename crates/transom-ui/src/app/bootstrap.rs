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

        let _app_entity = cx.entity();
        let appearance_subscription = window.observe_window_appearance(move |window, cx| {
            let follows_system =
                update_app_settings(cx, |s| s.appearance.mode == UiThemeMode::System);
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
        let monitor = cx.new(|cx| crate::monitor::MonitorView::new(window, &settings.monitor, cx));
        let editors = cx.new(|_| EditorPanels::new());

        let persistence = Persistence::new(
            services.session_store.clone(),
            services.settings_store.clone(),
        );
        let state = AppState {
            services,
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
                input: broadcast_input,
                textarea: broadcast_textarea,
                targets: HashSet::new(),
                targets_scope: Vec::new(),
            },
            status: None,
            persistence,
            pending_notices,
            immersive: false,
            sftp_panel_width_target: None,
            pending_sftp_requests: Vec::new(),
            shortcut_recording: ShortcutRecordingState {
                filter: shortcut_filter,
                applied_keys: applied_shortcut_keys,
                capture: None,
            },
            open_modals: crate::dialog_host::OpenModals::default(),
            dialog_hint: None,
        };
        let mut this = Self { state };
        // 面板需要回到外壳执行跨域动作；外壳此时才构造完成，因此两段式绑定。
        let app_weak = cx.entity().downgrade();
        this.state
            .monitor
            .update(cx, |view, _| view.attach(app_weak.clone()));
        this.state
            .editors
            .update(cx, |panels, _| panels.attach(app_weak));

        cx.subscribe_in(
            &this.state.broadcast.input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    this.broadcast_send(window, cx);
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &this.state.broadcast.textarea,
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
            &this.state.vertical_tabs_search,
            |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        )
        .detach();

        if app_settings(cx).appearance.mode == UiThemeMode::System {
            crate::theme::sync_with_system(window, cx);
        }
        this.sync_terminal_palette(cx);
        this.restore_startup_tabs(window, cx);

        this.state.tasks.git_sync = Some(cx.spawn(async move |this, cx| {
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
}
