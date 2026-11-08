//! AppShell 构造：存储加载、实体创建、订阅与启动恢复。

use super::*;
use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::audit_viewer::AuditViewerEvent;
use crate::backup::BackupPromptView;
use crate::keys::{KeyWizardView, KnownHostsEvent, KnownHostsView, default_ssh_dir};
use std::sync::atomic::AtomicBool;

impl AppShell {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        services: UiServices,
        mut settings: AppSettings,
        restart_requested: Arc<AtomicBool>,
    ) -> Self {
        let prepared = prepare_bootstrap(cx, &services, &mut settings);
        let inputs = BootstrapInputs::new(window, cx);
        cx.observe(&inputs.workspace_panels, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&inputs.workspace_panels_left, |_, _, cx| cx.notify())
            .detach();
        let event_task = Self::spawn_event_pump(window, cx, services.event_rx.clone());

        let appearance_subscription = Self::observe_window_appearance(window, cx);

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
                    history: prepared.history.clone(),
                    history_store: services.history.clone(),
                },
                prepared.catalog.clone(),
            )
        });
        // 弹窗内部的滑块/步进器订阅由设置实体自己持有，外壳不再感知这些控件。
        let sidebar_visible = settings.window.sidebar_visible_at_startup();
        // 左侧标签模式启动即以标签面板作为槽位主视图；顶部模式该标志无效。
        let vertical_tabs_open = settings.window.tab_position == TabPosition::Left;
        let monitor = cx.new(|cx| crate::monitor::MonitorView::new(window, &settings.monitor, cx));
        let editors = cx.new(|_| EditorPanels::new());

        let state = assemble_state(
            BootstrapState {
                services,
                prepared,
                inputs,
                settings_view,
                monitor,
                editors,
                persistence,
                appearance_subscription,
                event_task,
                known_hosts_records,
                sidebar_visible,
                vertical_tabs_open,
                restart_requested,
            },
            cx,
        );

        let mut this = Self { state };
        // 升级确认放在首帧之后的独立后台任务里：窗口刚创建不代表渲染链路、
        // 资源加载与事件泵都已稳定，过早确认会让崩溃回滚失去观察窗口。
        let executor = cx.background_executor().clone();
        let confirm_task = cx.background_spawn(async move {
            executor.timer(std::time::Duration::from_secs(3)).await;
            match spawn_blocking_local(transom_updater::confirm_update_success).await {
                Some(Ok(())) => log::debug!("升级状态确认完成"),
                Some(Err(error)) => log::warn!("升级状态确认失败，已保留观察期标记: {error}"),
                None => log::warn!("创建升级状态确认线程失败，已保留观察期标记"),
            }
        });
        this.state.tasks.background.update_confirm = Some(confirm_task);
        this.bootstrap_subscriptions(window, cx);

        this.bootstrap_startup(window, cx);

        this
    }
    /// 应用事件泵：逐条转交给外壳，接收端关闭或实体失效即退出。
    fn spawn_event_pump(
        window: &mut Window,
        cx: &mut Context<Self>,
        event_rx: flume::Receiver<transom_core::AppEvent>,
    ) -> gpui::Task<()> {
        cx.spawn_in(window, async move |this, cx| {
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
        })
    }

    /// 跟随系统外观：仅在设置为 System 时同步主题与终端调色板。
    fn observe_window_appearance(window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        let app_entity = cx.entity();
        window.observe_window_appearance(move |window, cx| {
            let follows_system =
                update_app_settings(cx, |s| s.appearance.mode == UiThemeMode::System);
            if follows_system {
                crate::theme::sync_with_system(window, cx);
                if crate::settings::sync_terminal_palette(cx) {
                    app_entity.update(cx, |shell, cx| shell.apply_appearance_to_live_panes(cx));
                }
            }
        })
    }

    /// 安装窗口可见性观察器：当窗口重新可见或恢复前台时，若开启 auto_update_on_focus，自动刷新所有打开的 Git 项目状态。
    fn observe_window_focus(window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        let app_entity = cx.entity();
        window.observe_window_visibility(move |visibility, _window, cx| {
            if visibility.is_visible() {
                app_entity.update(cx, |shell, cx| {
                    if app_settings(cx).git.auto_update_on_focus {
                        shell.git_refresh_active_projects(cx);
                    }
                });
            }
        })
    }

    /// 安装面板事件订阅与输入框联动；全部订阅由 `TaskHub` 托管生命周期。
    fn bootstrap_subscriptions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let git_focus = Self::observe_window_focus(window, cx);
        self.state.tasks.subscriptions._git_focus = Some(git_focus);

        // 监控面板的意图事件由外壳订阅处理。
        let monitor_subscription = cx.subscribe(
            &self.state.monitor,
            |shell, _view, event: &crate::monitor::MonitorPanelEvent, cx| {
                shell.on_monitor_panel_event(event, cx);
            },
        );
        self.state.tasks.subscriptions.monitor = Some(monitor_subscription);
        let known_hosts_subscription = cx.subscribe(
            &self.state.modals.connection.known_hosts,
            |shell, _, event: &KnownHostsEvent, cx| shell.on_known_hosts_event(event, cx),
        );
        self.state.tasks.subscriptions.known_hosts = Some(known_hosts_subscription);
        let backup_subscription = cx.subscribe(
            &self.state.modals.tools.backup_prompt,
            |shell, _view, event: &crate::backup::BackupPromptEvent, cx| {
                shell.on_backup_event(event, cx);
            },
        );
        self.state.tasks.subscriptions.backup = Some(backup_subscription);
        let audit_subscription = cx.subscribe_in(
            &self.state.modals.tools.audit_viewer,
            window,
            |shell, _view, event: &AuditViewerEvent, window, cx| {
                shell.on_audit_viewer_event(event, window, cx);
            },
        );
        self.state.tasks.subscriptions.audit_viewer = Some(audit_subscription);
        let settings_subscription = cx.subscribe(
            &self.state.modals.settings,
            |shell, _view, event: &crate::settings::SettingsEvent, cx| {
                shell.on_settings_event(event, cx);
            },
        );
        self.state.tasks.subscriptions.settings = Some(settings_subscription);
        let metrics_subscription = cx.subscribe(
            &self.state.status_metrics,
            |shell, _view, event: &crate::status_bar::metrics::StatusMetricsEvent, cx| {
                if matches!(
                    event,
                    crate::status_bar::metrics::StatusMetricsEvent::LayoutChanged
                ) {
                    shell.persist_layout_settings(cx);
                }
            },
        );
        self.state.tasks.subscriptions._status_metrics = Some(metrics_subscription);
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
                if this
                    .update(cx, |this, cx| {
                        this.tick_git_sync(cx);
                        this.tick_git_auto_fetch(cx);
                        this.tick_container_cleanup(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }
}
/// 构造期一次性准备的服务与设置快照。
struct BootstrapPrepared {
    history: Arc<parking_lot::RwLock<Vec<String>>>,
    history_provider: transom_terminal::HistoryProvider,
    shortcut_filter: Arc<parking_lot::RwLock<Vec<gpui_kit::KeybindingKeystroke>>>,
    applied_shortcut_keys: Vec<(String, &'static str)>,
    catalog: SessionCatalog,
    pending_notices: PendingNotices,
}

/// 启动状态装配输入：准备结果、实体与订阅。
struct BootstrapState {
    services: UiServices,
    prepared: BootstrapPrepared,
    inputs: BootstrapInputs,
    settings_view: Entity<crate::settings::SettingsView>,
    monitor: Entity<crate::monitor::MonitorView>,
    editors: Entity<EditorPanels>,
    persistence: Persistence,
    appearance_subscription: Subscription,
    event_task: gpui::Task<()>,
    known_hosts_records: Vec<transom_core::HostKeyRecord>,
    sidebar_visible: bool,
    vertical_tabs_open: bool,
    /// 安装完成后由关于页置位的重启意图，随 `run` 返回值交回 `main`。
    restart_requested: Arc<AtomicBool>,
}

/// 组装 `AppState`：把启动准备、实体句柄与事件订阅一次性收进状态。
fn assemble_state(input: BootstrapState, cx: &mut Context<AppShell>) -> AppState {
    let BootstrapState {
        services,
        prepared,
        inputs,
        settings_view,
        monitor,
        editors,
        persistence,
        appearance_subscription,
        event_task,
        known_hosts_records,
        sidebar_visible,
        vertical_tabs_open,
        restart_requested,
    } = input;
    let git_registry = services.git_projects.load().unwrap_or_default();
    let known_hosts_view = cx.new(|_| {
        KnownHostsView::new(
            services.known_hosts.clone(),
            known_hosts_records,
            inputs.known_hosts_search.clone(),
        )
    });
    let backup_prompt = cx.new(|_| BackupPromptView::new(services.backup.clone()));
    let audit_viewer = cx.new(|_| crate::audit_viewer::AuditViewerView::new());
    let key_wizard = cx.new(|_| KeyWizardView::new(default_ssh_dir(), services.command_tx.clone()));
    AppState {
        services,
        catalog: prepared.catalog,
        catalog_revision: 0,
        sidebar: SidebarState::new(sidebar_visible, inputs.search.clone()),
        workspace: WorkspaceState::new(),
        workspace_panels: inputs.workspace_panels.clone(),
        workspace_panels_left: inputs.workspace_panels_left.clone(),
        vertical_tabs_search: inputs.vertical_tabs_search.clone(),
        vertical_tabs_open,
        modals: ModalHub::new(
            known_hosts_view,
            settings_view,
            ToolModalEntities {
                backup_prompt,
                audit_viewer,
                key_wizard,
            },
            inputs.quick_open_state.clone(),
            SnippetPanelState {
                open: false,
                form: None,
            },
        ),
        tasks: TaskHub::new(appearance_subscription, event_task),
        git_sync_schedule: GitSyncScheduler::default(),
        container_auto_prune_last: None,
        history: prepared.history,
        history_provider: prepared.history_provider,
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
        pending_notices: prepared.pending_notices,
        immersive: false,
        window_requests: Default::default(),
        status_metrics: cx.new(|_| crate::status_bar::StatusBarMetricsView::new()),
        stats_sampling: true,
        shortcut_recording: ShortcutRecordingState {
            filter: prepared.shortcut_filter,
            applied_keys: prepared.applied_shortcut_keys,
            capture: None,
        },
        open_modals: crate::dialog_host::OpenModals::default(),
        dialog_hint: None,
        containers: crate::container::ContainerUiState::default(),
        git: crate::git::GitHub::new(git_registry.projects, git_registry.folders),
        toast_seq: 0,
        last_toast: None,
        restart_requested,
    }
}

/// 归一设置、装载 Global/键位、加载历史与会话目录。
fn prepare_bootstrap(
    cx: &mut Context<AppShell>,
    services: &UiServices,
    settings: &mut AppSettings,
) -> BootstrapPrepared {
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
    BootstrapPrepared {
        history,
        history_provider,
        shortcut_filter,
        applied_shortcut_keys,
        catalog,
        pending_notices,
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
