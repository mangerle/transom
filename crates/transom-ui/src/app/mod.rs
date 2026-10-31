//! GPUI 顶层应用外壳状态与生命周期主入口。

mod modal_hub;
mod overlays;
mod secret_prefill;
mod task_hub;
mod types;

pub(crate) use modal_hub::ModalHub;
pub(crate) use secret_prefill::prefill_secret_fields;
pub(crate) use task_hub::TaskHub;

use crate::UiServices;
use crate::broadcast::BroadcastBarState;
use crate::cloud_sync::GitSyncScheduler;
use crate::editor::EditorManager;
use crate::keymap;
use crate::keymap::ShortcutRecordingState;
use crate::keys::KnownHostsModalState;
use crate::palette::localized_error;
use crate::probe::ProbeBadge;
use crate::settings::SettingsModalState;
use crate::sftp::SftpModals;
use crate::sidebar::SidebarState;
use crate::snippets::SnippetPanelState;
use crate::workspace::WorkspaceState;
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::component::slider::{SliderEvent, SliderValue};
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use transom_core::{
    AppCommand, AppSettings, PanelSide, SessionCatalog, SessionId, StartupTabs, TabPosition,
    UiThemeMode,
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

pub struct AppShell {
    pub(crate) services: UiServices,
    pub(crate) settings: AppSettings,
    /// 会话与文件夹共享同一份目录快照，任何一方变化都整体持久化。
    pub(crate) catalog: SessionCatalog,
    /// 会话目录版本：每次持久化（= 目录被修改）递增，侧栏行缓存以此为失效键。
    pub(crate) catalog_revision: u64,
    /// 统一托管的侧栏核心状态与选择集。
    pub(crate) sidebar: SidebarState,
    /// 统一托管的工作区多标签与分屏网格核心状态。
    pub(crate) workspace: WorkspaceState,
    /// 工作区分组（侧栏、主区、SFTP、转发）的列宽状态。
    pub(crate) workspace_panels: Entity<ResizableState>,
    /// 停靠左/右各自独立的列宽状态。
    pub(crate) workspace_panels_left: Entity<ResizableState>,
    /// 纵向标签面板的搜索框状态。
    pub(crate) vertical_tabs_search: Entity<InputState>,
    /// 左侧槽位当前是否显示纵向标签面板；为假时槽位显示会话树或收起。
    pub(crate) vertical_tabs_open: bool,
    /// 统一托管的应用顶层交互模态与表单集合。
    pub(crate) modals: ModalHub,
    /// 统一纳管的应用外壳后台任务与事件订阅。
    pub(crate) tasks: TaskHub,
    /// 定时 Git 同步的分钟节拍。
    pub(crate) git_sync_schedule: GitSyncScheduler,
    /// 命令历史内存快照：终端视图的查询回调共享它，避免每次按键都读盘。
    pub(crate) history: Arc<parking_lot::RwLock<Vec<String>>>,
    /// 共享给每个终端视图的历史查询回调。
    pub(crate) history_provider: transom_terminal::HistoryProvider,
    /// 会话列表行尾的拨测状态点，按会话 id 索引。
    pub(crate) probe_badges: HashMap<SessionId, ProbeBadge>,
    /// 主机监控看板与进程管理器状态。
    pub(crate) monitor: crate::monitor::MonitorState,
    /// 统一托管的远端文本编辑器管理器。
    pub(crate) editors: EditorManager,
    /// SFTP 五类弹窗（权限、重命名等输入、属性、过滤、同步）的聚合状态。
    pub(crate) sftp_modals: SftpModals,
    /// 远程与本地文件分栏对比面板；同一时刻至多一个。
    pub(crate) diff_viewer: Option<crate::diff_viewer::DiffViewerState>,
    /// 底部广播输入条：目标集合、单行/多行输入与展开状态。
    pub(crate) broadcast: BroadcastBarState,
    /// 底栏右段的兜底状态文字（没有指标徽标时显示）。
    pub(crate) status: Option<String>,
    /// 应用级错误通知的待发队列：写入与渲染解耦，逐帧统一投递。
    pub(crate) pending_notices: PendingNotices,
    /// 沉浸式全屏：隐藏全部应用外框，仅保留终端视口。
    pub(crate) immersive: bool,
    /// SFTP 面板上次同步到的目标宽度；目标变了才调整面板宽度。
    pub(crate) sftp_panel_width_target: Option<f32>,
    /// 快捷键域的聚合状态：生效键位过滤器、已应用键位串与改键捕获。
    pub(crate) shortcut_recording: ShortcutRecordingState,
    /// 组件库弹窗栈的影子记录：与容器实际栈同序，逐帧对账的依据。
    pub(crate) open_modals: crate::dialog_host::OpenModals,
    /// 弹窗内的一次性提示：脏表单被拦下不关时，说明为什么按键没有反应。
    pub(crate) dialog_hint: Option<String>,
}

impl AppShell {
    /// 向控制器提交命令的统一入口：非阻塞 try_send，队列满时快速失败。
    pub(crate) fn send_command(&mut self, command: impl Into<AppCommand>) -> bool {
        match self.services.command_tx.try_send(command.into()) {
            Ok(()) => true,
            Err(error) => {
                self.set_error(command_send_error_text(error));
                false
            }
        }
    }

    /// 记录一条应用级错误，由下一次渲染统一转成通知。
    ///
    /// 设计原理（Why）：错误写入点多数不持有窗口句柄，无法在写入时推送通知；
    /// 单槽暂存又会让同一帧内的多次失败互相覆盖。入队后由渲染统一投递，
    /// 既保留每条错误，也让渲染只负责取队列、不再承担"暂存"职责。
    pub(crate) fn set_error(&mut self, message: impl Into<String>) {
        self.pending_notices.push(message);
    }

    /// 设置工具面板（SFTP/转发）停靠方向。
    pub(crate) fn set_sftp_panel_side(&mut self, side: PanelSide, cx: &mut Context<Self>) {
        if self.settings.window.sftp_panel_side == side {
            return;
        }
        self.settings.window.sftp_panel_side = side;
        std::mem::swap(&mut self.workspace_panels, &mut self.workspace_panels_left);
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

        let mut this = Self {
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

    /// 持久化会话与文件夹目录。
    pub(crate) fn persist_catalog(&mut self) -> bool {
        self.catalog_revision = self.catalog_revision.wrapping_add(1);
        match self.services.session_store.save(&self.catalog) {
            Ok(()) => true,
            Err(error) => {
                self.set_error(localized_error(&error));
                false
            }
        }
    }
}
