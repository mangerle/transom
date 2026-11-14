//! AppShell 构造：存储加载、实体创建、订阅与启动恢复。

use super::*;
use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::audit_viewer::AuditViewerEvent;
use crate::backup::BackupPromptView;
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
                    app_entity.update(cx, |shell, cx| crate::settings::apply_appearance_to_live_panes(shell, cx));
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
                        crate::git::actions::git_refresh_active_projects(shell, cx);
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
                crate::app::monitor::on_monitor_panel_event(shell, event, cx);
            },
        );
        self.state.tasks.subscriptions.monitor = Some(monitor_subscription);
        let known_hosts_subscription = cx.subscribe(
            &self.state.modals.connection.known_hosts,
            |shell, _, event: &KnownHostsEvent, cx| crate::app::known_hosts_modal::on_known_hosts_event(shell, event, cx),
        );
        self.state.tasks.subscriptions.known_hosts = Some(known_hosts_subscription);
        let connection_prompts_subscription = cx.subscribe(
            &self.state.modals.connection.prompts,
            |shell, _view, event: &crate::connection::ConnectionPromptsEvent, cx| {
                crate::connection::on_connection_prompts_event(shell, event, cx);
            },
        );
        self.state.tasks.subscriptions.connection_prompts = Some(connection_prompts_subscription);
        let key_wizard_subscription = cx.subscribe(
            &self.state.modals.tools.key_wizard,
            |shell, _view, event: &crate::keys::KeyWizardEvent, cx| {
                crate::app::keys::on_key_wizard_event(shell, event, cx);
            },
        );
        self.state.tasks.subscriptions.key_wizard = Some(key_wizard_subscription);
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
                crate::app::audit_viewer::on_audit_viewer_event(shell, event, window, cx);
            },
        );
        self.state.tasks.subscriptions.audit_viewer = Some(audit_subscription);
        let replay_subscription = cx.subscribe_in(
            &self.state.modals.tools.replay,
            window,
            |shell, _view, event: &ReplayEvent, window, cx| {
                crate::record::on_replay_event(shell, event, window, cx);
            },
        );
        self.state.tasks.subscriptions.replay = Some(replay_subscription);
        let settings_subscription = cx.subscribe(
            &self.state.modals.settings,
            |shell, _view, event: &crate::settings::SettingsEvent, cx| {
                crate::settings::on_settings_event(shell, event, cx);
            },
        );
        self.state.tasks.subscriptions.settings = Some(settings_subscription);
        let session_form_subscription = cx.subscribe_in(
            &self.state.modals.session.form,
            window,
            |shell, _view, event: &crate::session_form::SessionFormEvent, window, cx| {
                shell.on_session_form_event(event, window, cx);
            },
        );
        self.state.tasks.subscriptions.session_form = Some(session_form_subscription);
        // 容器连接表单的保存与删除仍由外壳落目录：实体只上交意图。
        let container_form_subscription = cx.subscribe_in(
            &self.state.modals.tools.container_form,
            window,
            |shell, _view, event: &crate::container::ContainerFormEvent, window, cx| {
                crate::container::on_container_form_event(shell, event, window, cx);
            },
        );
        self.state.tasks.subscriptions.container_form = Some(container_form_subscription);
        // 运行容器表单只上交提交意图：目标工作区定位与命令投递仍归外壳。
        let container_run_form_subscription = cx.subscribe(
            &self.state.modals.tools.run_form,
            |shell, _view, event: &crate::container::ContainerRunFormEvent, cx| {
                crate::container::on_container_run_form_event(shell, event, cx);
            },
        );
        self.state.tasks.subscriptions.container_run_form = Some(container_run_form_subscription);
        // 拉取/新建弹窗只上交附加字段：主输入校验、命令投递与关闭仍归外壳。
        let container_create_subscription = cx.subscribe_in(
            &self.state.modals.tools.create_dialog,
            window,
            |shell, _view, event: &crate::container::ContainerCreateDialogEvent, window, cx| {
                crate::container::on_container_create_event(shell, event, window, cx);
            },
        );
        self.state.tasks.subscriptions.container_create = Some(container_create_subscription);
        // 容器侧栏只上交目录/窗口动作：事件自带数据，外壳在渲染期统一执行。
        let container_sidebar_subscription = cx.subscribe(
            &self.state.containers.sidebar,
            |shell, _view, event: &crate::container::ContainerSidebarEvent, cx| {
                shell
                    .state
                    .window_requests
                    .push_container_sidebar(event.clone());
                cx.notify();
            },
        );
        self.state.tasks.subscriptions.container_sidebar = Some(container_sidebar_subscription);
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
                crate::editor::on_editor_panel_event(shell, event, cx);
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
                    crate::app::broadcast::broadcast_send(this, window, cx);
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &self.state.broadcast.textarea,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    crate::app::broadcast::broadcast_send(this, window, cx);
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

        let history_path = crate::cloud_sync::sync_history_path();
        self.state.tasks.background.sync_history_load = Some(cx.spawn(async move |this, cx| {
            let Some(history) =
                spawn_blocking_local(move || crate::cloud_sync::load_history(&history_path)).await
            else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.state.cloud_sync.replace_history(history);
                cx.notify();
            });
        }));

        self.state.tasks.background.git_sync = Some(cx.spawn(async move |this, cx| {
            let mut minute_elapsed = 0u64;
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                minute_elapsed = minute_elapsed.saturating_add(1);
                if this
                    .update(cx, |this, cx| {
                        crate::cloud_sync::tick_cloud_sync_auto(this, cx);
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
