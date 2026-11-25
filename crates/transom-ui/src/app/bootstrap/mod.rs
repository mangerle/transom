//! AppShell 构造：存储加载、实体创建、订阅与启动恢复。

use super::*;
use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::audit_viewer::AuditViewerEvent;
use crate::backup::BackupPromptView;
use crate::cloud_sync::CloudSyncRequest;
use crate::keys::{KeyWizardView, KnownHostsEvent, KnownHostsView, default_ssh_dir};
use crate::record::ReplayEvent;
use crate::sidebar::SidebarSection;
use std::sync::atomic::AtomicBool;

mod prepare;
use prepare::{BootstrapInputs, BootstrapState, assemble_state, prepare_bootstrap};

impl AppShell {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        services: UiServices,
        mut settings: AppSettings,
        restart_requested: Arc<AtomicBool>,
    ) -> Self {
        let sync_before_normalize = settings.sync.clone();
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
            services.api_projects.clone(),
        );
        let local_shells = Arc::new(transom_platform::shell::discover_shells());
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
                    completion: prepared.completion.clone(),
                    local_shells: local_shells.clone(),
                },
                prepared.catalog.clone(),
            )
        });
        // 弹窗内部的滑块/步进器订阅由设置实体自己持有，外壳不再感知这些控件。
        let sidebar_visible = settings.window.sidebar_visible_at_startup();
        // 左侧标签模式启动即以标签面板作为槽位主视图；顶部模式回到会话树。
        let sidebar_section = if settings.window.tab_position == TabPosition::Left {
            SidebarSection::Tabs
        } else {
            SidebarSection::Sessions
        };
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
                sidebar_section,
                restart_requested,
                local_shells,
            },
            window,
            cx,
        );

        let mut this = Self { state };
        if settings.sync != sync_before_normalize
            && let Err(error) = this.state.persist_settings(&settings)
        {
            this.state.set_error(localized_error(&error));
        }
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
        this.state.runtime.tasks.background.update_confirm = Some(confirm_task);
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
                    app_entity.update(cx, |shell, cx| {
                        crate::settings::apply_appearance_to_live_panes(&shell.state, cx)
                    });
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
                        crate::git::actions::GitActions::new(&mut shell.state)
                            .git_refresh_active_projects(cx);
                    }
                });
            }
        })
    }

    /// 安装面板事件订阅与输入框联动；全部订阅由 `TaskHub` 托管生命周期。
    fn bootstrap_subscriptions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.runtime.tasks.subscriptions._git_focus =
            Some(Self::observe_window_focus(window, cx));
        self.subscribe_panel_entities(cx);
        self.subscribe_modal_entities(window, cx);
        self.subscribe_input_controls(window, cx);
    }

    fn subscribe_panel_entities(&mut self, cx: &mut Context<Self>) {
        self.state.runtime.tasks.subscriptions.monitor = Some(cx.subscribe(
            &self.state.features.monitor,
            |shell, _view, event: &crate::monitor::MonitorPanelEvent, cx| {
                crate::monitor::MonitorOps::new(&mut shell.state).on_monitor_panel_event(event, cx);
            },
        ));
        self.state.runtime.tasks.subscriptions.container_sidebar = Some(cx.subscribe(
            &self.state.features.containers.sidebar,
            |shell, _view, event: &crate::container::ContainerSidebarEvent, cx| {
                shell
                    .state
                    .ui
                    .window_requests
                    .push_container_sidebar(event.clone());
                cx.notify();
            },
        ));
        self.state.runtime.tasks.subscriptions.git_sidebar = Some(cx.subscribe(
            &self.state.features.git_sidebar,
            |shell, _view, event: &crate::git::GitSidebarEvent, cx| {
                shell
                    .state
                    .ui
                    .window_requests
                    .push_git_sidebar(event.clone());
                cx.notify();
            },
        ));
        self.state.runtime.tasks.subscriptions.api_sidebar = Some(cx.subscribe(
            &self.state.features.api_sidebar,
            |shell, _view, event: &crate::api_client::ApiSidebarEvent, cx| {
                crate::api_client::on_sidebar_event(&mut shell.state, event, cx);
            },
        ));
        self.state.runtime.tasks.subscriptions.api_sidebar_observer =
            Some(cx.observe(&self.state.features.api_sidebar, |_, _, cx| cx.notify()));
        self.state.runtime.tasks.subscriptions.session_sidebar = Some(cx.subscribe(
            &self.state.session.session_sidebar,
            |shell, _view, event: &crate::sidebar::SessionSidebarEvent, cx| {
                shell
                    .state
                    .ui
                    .window_requests
                    .push_session_sidebar(event.clone());
                cx.notify();
            },
        ));
        self.state.runtime.tasks.subscriptions._status_metrics = Some(cx.subscribe(
            &self.state.features.status_metrics,
            |shell, _view, event: &crate::status_bar::metrics::StatusMetricsEvent, cx| {
                if matches!(
                    event,
                    crate::status_bar::metrics::StatusMetricsEvent::LayoutChanged
                ) {
                    shell.state.persist_layout_settings(cx);
                }
            },
        ));
        let editor_command_tx = self.state.runtime.services.command_tx.clone();
        self.state.runtime.tasks.subscriptions.editor = Some(cx.subscribe(
            &self.state.features.editors,
            |shell, _panels, event: &crate::editor::EditorPanelEvent, cx| {
                shell.state.ui.window_requests.push_editor(event.clone());
                cx.notify();
            },
        ));
        self.state
            .features
            .editors
            .update(cx, |panels, _| panels.set_command_tx(editor_command_tx));
    }

    fn subscribe_modal_entities(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 更新状态变化时，活动栏提示必须随实体通知重绘；这里只负责转发脏标记。
        cx.observe(&self.state.features.update, |_, _, cx| cx.notify())
            .detach();
        self.state.runtime.tasks.subscriptions.update = Some(cx.subscribe(
            &self.state.features.update,
            |shell, _view, event: &crate::app::update::UpdateEvent, cx| {
                if matches!(event, crate::app::update::UpdateEvent::RestartRequested) {
                    shell
                        .state
                        .runtime
                        .restart_requested
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    cx.quit();
                }
            },
        ));
        self.state.runtime.tasks.subscriptions.known_hosts = Some(cx.subscribe(
            &self.state.ui.modals.connection.known_hosts,
            |shell, _, event: &KnownHostsEvent, cx| {
                crate::app::events::on_known_hosts_event(&mut shell.state, event, cx)
            },
        ));
        self.state.runtime.tasks.subscriptions.connection_prompts = Some(cx.subscribe(
            &self.state.ui.modals.connection.prompts,
            |shell, _view, event: &crate::connection::ConnectionPromptsEvent, cx| {
                crate::connection::on_connection_prompts_event(&mut shell.state, event, cx);
            },
        ));
        self.state.runtime.tasks.subscriptions.key_wizard = Some(cx.subscribe(
            &self.state.ui.modals.tools.key_wizard,
            |shell, _view, event: &crate::keys::KeyWizardEvent, cx| {
                crate::keys::on_key_wizard_event(&mut shell.state, event, cx);
            },
        ));
        self.state.runtime.tasks.subscriptions.backup = Some(cx.subscribe(
            &self.state.ui.modals.tools.backup_prompt,
            |shell, _view, event: &crate::backup::BackupPromptEvent, cx| {
                shell.on_backup_event(event, cx);
            },
        ));
        self.state.runtime.tasks.subscriptions.audit_viewer = Some(cx.subscribe_in(
            &self.state.ui.modals.tools.audit_viewer,
            window,
            |shell, _view, event: &AuditViewerEvent, window, cx| {
                crate::audit_viewer::on_audit_viewer_event(&mut shell.state, event, window, cx);
            },
        ));
        self.state.runtime.tasks.subscriptions.replay = Some(cx.subscribe_in(
            &self.state.ui.modals.tools.replay,
            window,
            |shell, _view, event: &ReplayEvent, window, cx| {
                crate::record::on_replay_event(&mut shell.state, event, window, cx);
            },
        ));
        self.state.runtime.tasks.subscriptions.settings = Some(cx.subscribe(
            &self.state.ui.modals.settings,
            |shell, _view, event: &crate::settings::SettingsEvent, cx| {
                crate::settings::on_settings_event(&mut shell.state, event, cx);
            },
        ));
        self.state.runtime.tasks.subscriptions.session_form = Some(cx.subscribe_in(
            &self.state.ui.modals.session.form,
            window,
            |shell, _view, event: &crate::session_form::SessionFormEvent, window, cx| {
                shell.on_session_form_event(event, window, cx);
            },
        ));
        self.state.runtime.tasks.subscriptions.container_form = Some(cx.subscribe_in(
            &self.state.ui.modals.tools.container_form,
            window,
            |shell, _view, event: &crate::container::ContainerFormEvent, window, cx| {
                crate::container::on_container_form_event(&mut shell.state, event, window, cx);
            },
        ));
        self.state.runtime.tasks.subscriptions.container_run_form = Some(cx.subscribe(
            &self.state.ui.modals.tools.run_form,
            |shell, _view, event: &crate::container::ContainerRunFormEvent, cx| {
                crate::container::on_container_run_form_event(&mut shell.state, event, cx);
            },
        ));
        self.state.runtime.tasks.subscriptions.container_create = Some(cx.subscribe_in(
            &self.state.ui.modals.tools.create_dialog,
            window,
            |shell, _view, event: &crate::container::ContainerCreateDialogEvent, window, cx| {
                crate::container::on_container_create_event(&mut shell.state, event, window, cx);
            },
        ));
        self.state.runtime.tasks.subscriptions.cloud_sync = Some(cx.subscribe_in(
            &self.state.features.cloud_sync,
            window,
            |shell, _view, event: &crate::cloud_sync::CloudSyncEvent, window, cx| {
                if let Some(CloudSyncRequest::OpenSettings) =
                    crate::cloud_sync::on_cloud_sync_event(&mut shell.state, event, window, cx)
                {
                    shell.open_settings_modal(None, window, cx);
                }
            },
        ));
    }

    fn subscribe_input_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.subscribe_in(
            &self.state.features.broadcast.input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    crate::broadcast::BroadcastOps::new(&mut this.state).send(window, cx);
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &self.state.features.broadcast.textarea,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    crate::broadcast::BroadcastOps::new(&mut this.state).send(window, cx);
                }
            },
        )
        .detach();
        cx.subscribe(
            &self.state.ui.vertical_tabs_search,
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
        self.restore_api_tabs(window, cx);

        // 每次进程启动只检查一次远端清单；发现新版本时仅点亮左侧提示，
        // 不主动打断用户当前工作，设置页与侧栏点击共用同一个更新实体。
        let update = self.state.features.update.clone();
        update.update(cx, |view, cx| view.start_startup_check(cx));

        let history_path = crate::cloud_sync::sync_history_path();
        self.state.runtime.tasks.background.sync_history_load =
            Some(cx.spawn(async move |this, cx| {
                let Some(history) =
                    spawn_blocking_local(move || crate::cloud_sync::load_history(&history_path))
                        .await
                else {
                    return;
                };
                let _ = this.update(cx, |this, cx| {
                    this.state.features.cloud_sync.update(cx, |view, cx| {
                        view.replace_history(history, cx);
                    });
                    cx.notify();
                });
            }));

        self.state.runtime.tasks.background.git_sync = Some(cx.spawn(async move |this, cx| {
            let mut minute_elapsed = 0u64;
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                minute_elapsed = minute_elapsed.saturating_add(1);
                if this
                    .update(cx, |this, cx| {
                        crate::cloud_sync::tick_cloud_sync_auto(&mut this.state, cx);
                        if minute_elapsed >= 60 {
                            minute_elapsed = 0;
                            this.tick_git_sync(cx);
                            this.tick_git_auto_fetch(cx);
                            this.tick_container_cleanup(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }
}
