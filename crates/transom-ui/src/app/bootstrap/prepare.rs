//! 启动期准备阶段：历史、快捷键过滤与目录预加载。

use super::*;
use crate::cloud_sync::{CloudSyncView, SyncHistoryWriter, sync_history_path};
use std::sync::atomic::AtomicBool;

/// 构造期一次性准备的服务与设置快照。
pub(super) struct BootstrapPrepared {
    pub(super) history: Arc<parking_lot::RwLock<Vec<String>>>,
    pub(super) completion: Arc<CompletionSource>,
    pub(super) completion_provider: transom_terminal::CompletionProvider,
    pub(super) shortcut_filter: Arc<parking_lot::RwLock<Vec<gpui_kit::KeybindingKeystroke>>>,
    pub(super) applied_shortcut_keys: Vec<(String, &'static str)>,
    pub(super) catalog: SessionCatalog,
    pub(super) pending_notices: PendingNotices,
}

/// 启动状态装配输入：准备结果、实体与订阅。
pub(super) struct BootstrapState {
    pub(super) services: UiServices,
    pub(super) prepared: BootstrapPrepared,
    pub(super) inputs: BootstrapInputs,
    pub(super) settings_view: Entity<crate::settings::SettingsView>,
    pub(super) monitor: Entity<crate::monitor::MonitorView>,
    pub(super) editors: Entity<EditorPanels>,
    pub(super) persistence: Persistence,
    pub(super) appearance_subscription: Subscription,
    pub(super) event_task: gpui::Task<()>,
    pub(super) known_hosts_records: Vec<transom_core::HostKeyRecord>,
    pub(super) sidebar_visible: bool,
    pub(super) sidebar_section: SidebarSection,
    /// 安装完成后由关于页置位的重启意图，随 `run` 返回值交回 `main`。
    pub(super) restart_requested: Arc<AtomicBool>,
    pub(super) local_shells: Arc<Vec<transom_platform::shell::ShellOption>>,
}

/// 组装 `AppState`：把启动准备、实体句柄与事件订阅一次性收进状态。
pub(super) fn assemble_state(
    input: BootstrapState,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) -> AppState {
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
        sidebar_section,
        restart_requested,
        local_shells,
    } = input;
    let git_registry = services.git_projects.load().unwrap_or_default();
    let (api_registry, api_load_error) = match services.api_projects.load() {
        Ok(registry) => (registry, None),
        Err(error) => {
            let message = error.detail.unwrap_or_else(|| format!("{:?}", error.code));
            log::error!("加载 API 项目失败，已按空登记表启动: {message}");
            (transom_api::ApiRegistry::default(), Some(message))
        }
    };
    let session_form = cx.new(|_| crate::session_form::SessionFormView::new());
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
    let replay = cx.new(|_| crate::record::ReplayModalView::new());
    let connection_prompts = cx.new(|_| crate::connection::ConnectionPromptsView::new());
    let container_form =
        cx.new(|_| crate::container::ContainerFormView::new(services.command_tx.clone()));
    let container_run_form = cx.new(|_| crate::container::ContainerRunFormView::new());
    let container_create_dialog = cx.new(|_| crate::container::ContainerCreateDialogView::new());
    let cloud_sync = cx.new(|_| CloudSyncView::new(SyncHistoryWriter::spawn(sync_history_path())));
    let update = cx.new(|_| crate::app::update::UpdateView::new());
    AppState {
        runtime: super::super::state::RuntimeState {
            services,
            tasks: TaskHub::new(appearance_subscription, event_task),
            automation: super::super::state::AutomationState::default(),
            history: prepared.history,
            completion: prepared.completion,
            completion_provider: prepared.completion_provider,
            persistence,
            restart_requested,
        },
        session: super::super::state::SessionState {
            local_shells,
            catalog: prepared.catalog,
            catalog_revision: 0,
            sidebar: SidebarState::new(sidebar_visible, sidebar_section, inputs.search.clone()),
            session_sidebar: cx.new(|_| crate::sidebar::SessionSidebarView::new()),
            probe_badges: HashMap::new(),
        },
        workspace: WorkspaceState::new(),
        ui: super::super::state::UiState {
            workspace_panels: inputs.workspace_panels.clone(),
            workspace_panels_left: inputs.workspace_panels_left.clone(),
            vertical_tabs_search: inputs.vertical_tabs_search.clone(),
            modals: ModalHub::new(
                session_form,
                known_hosts_view,
                connection_prompts,
                settings_view,
                ToolModalEntities {
                    backup_prompt,
                    audit_viewer,
                    key_wizard,
                    replay,
                    container_form,
                    run_form: container_run_form,
                    create_dialog: container_create_dialog,
                },
                inputs.quick_open_state.clone(),
                SnippetPanelState {
                    open: false,
                    form: None,
                },
            ),
            status: None,
            immersive: false,
            window_requests: Default::default(),
            shortcut_recording: ShortcutRecordingState {
                filter: prepared.shortcut_filter,
                applied_keys: prepared.applied_shortcut_keys,
                capture: None,
            },
            open_modals: crate::dialog_host::OpenModals::default(),
            feedback: super::super::state::UiFeedbackState::new(prepared.pending_notices),
        },
        features: super::super::state::FeatureState {
            cloud_sync,
            update,
            monitor,
            containers: crate::container::ContainerUiState::new(window, cx),
            git_sidebar: cx.new(move |_| {
                crate::git::GitSidebarView::new(crate::git::GitHub::new(
                    git_registry.projects,
                    git_registry.folders,
                ))
            }),
            api_sidebar: cx
                .new(|_| crate::api_client::ApiSidebarView::new(api_registry, api_load_error)),
            editors,
            diff_viewer: None,
            broadcast: BroadcastBarState {
                open: false,
                multiline: false,
                targets_open: false,
                input: inputs.broadcast_input.clone(),
                textarea: inputs.broadcast_textarea.clone(),
                targets: HashSet::new(),
                targets_scope: Vec::new(),
            },
            status_metrics: cx.new(|_| crate::status_bar::StatusBarMetricsView::new()),
            stats_sampling: true,
        },
    }
}

/// 归一设置、装载 Global/键位、加载历史与会话目录。
pub(super) fn prepare_bootstrap(
    cx: &mut Context<AppShell>,
    services: &UiServices,
    settings: &mut AppSettings,
) -> BootstrapPrepared {
    settings.activity_bar.normalize();
    settings.status_bar.normalize();
    settings.sync.normalize();
    settings.appearance.normalize();
    settings.window.normalize();
    // 设置与进程同生命周期，装进 Global；外壳不再持字段。
    install_app_settings(cx, settings.clone());
    cx.bind_keys([
        KeyBinding::new(
            "tab",
            transom_terminal::SendTab,
            Some(transom_terminal::TERMINAL_CONTEXT),
        ),
        KeyBinding::new(
            "shift-tab",
            transom_terminal::SendBacktab,
            Some(transom_terminal::TERMINAL_CONTEXT),
        ),
    ]);
    let bindings = keymap::bindings(&settings.shortcuts, cx);
    cx.bind_keys(bindings);
    let shortcut_filter = Arc::new(parking_lot::RwLock::new(keymap::reserved_keystrokes(
        &settings.shortcuts,
        cx.platform_info().os,
    )));
    let applied_shortcut_keys =
        keymap::effective_key_strings(&settings.shortcuts, cx.platform_info().os);
    let history: Arc<parking_lot::RwLock<Vec<String>>> =
        Arc::new(parking_lot::RwLock::new(if settings.history.enabled {
            services.history.load()
        } else {
            Vec::new()
        }));
    // 补全候选来源：历史快照与设置页共用同一容器，片段与开关的初始值在这里灌入。
    let completion = Arc::new(CompletionSource::new(Arc::clone(&history)));
    completion.apply_settings(settings);
    let completion_provider: transom_terminal::CompletionProvider = {
        let source = Arc::clone(&completion);
        Arc::new(move |token: &str, before: &str| source.complete(token, before))
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
        completion,
        completion_provider,
        shortcut_filter,
        applied_shortcut_keys,
        catalog,
        pending_notices,
    }
}

/// 构造期一次性创建的输入框/面板句柄，避免 `new` 内平铺二十个同名变量。
pub(super) struct BootstrapInputs {
    search: Entity<InputState>,
    quick_open_state: Entity<CommandState>,
    broadcast_input: Entity<InputState>,
    broadcast_textarea: Entity<TextareaState>,
    vertical_tabs_search: Entity<InputState>,
    pub(super) workspace_panels: Entity<ResizableState>,
    pub(super) workspace_panels_left: Entity<ResizableState>,
    known_hosts_search: Entity<InputState>,
}

impl BootstrapInputs {
    pub(super) fn new(window: &mut Window, cx: &mut Context<AppShell>) -> Self {
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
