//! GPUI 顶层应用外壳状态与生命周期主入口。

mod overlays;
mod types;

use crate::UiServices;
use crate::broadcast::BroadcastBarState;
use crate::keymap;
use crate::palette::localized_error;
use crate::probe::ProbeBadge;
use crate::session_form::SessionForm;
use crate::sftp::{EditorPanelState, PendingEditorClose};
use crate::tiling::TiledTabs;
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::component::slider::{SliderEvent, SliderValue};
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use transom_core::{
    AppCommand, AppSettings, GroupId, PanelSide, SessionCatalog, SessionId, SplitId, StartupTabs,
    TabId, UiThemeMode,
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
    /// 侧栏行快照缓存：目录与搜索词不变时复用，避免整壳重绘反复构建行表。
    pub(crate) sidebar_rows_cache: Option<SidebarRowsCache>,
    pub(crate) tabs: Vec<TerminalTab>,
    /// 标签栏条目：每项持有一棵分割树，树里的叶子才是 `tabs` 中的窗格。
    pub(crate) groups: Vec<TabGroup>,
    /// 并排网格集合；每张网格在标签栏里合并成一个条目，同一窗口可并存多张，
    /// 只有活动标签所在的那张显示在内容区。
    pub(crate) tiled_sets: Vec<TiledTabs>,
    /// Ctrl/Cmd+点击选中的标签组；用于多选后打开并排布局菜单。
    pub(crate) tab_selection: HashSet<GroupId>,
    /// 各分割节点的拖拽尺寸状态，由外壳持有以跨过不可见的帧。
    pub(crate) pane_split_states: HashMap<SplitId, Entity<ResizableState>>,
    /// 工作区分组（侧栏、主区、SFTP、转发）的列宽状态。
    pub(crate) workspace_panels: Entity<ResizableState>,
    /// 停靠左/右各自独立的列宽状态。
    pub(crate) workspace_panels_left: Entity<ResizableState>,
    pub(crate) active_tab: Option<TabId>,
    pub(crate) search: Entity<InputState>,
    pub(crate) form: Option<SessionForm>,
    pub(crate) folder_form: Option<FolderForm>,
    pub(crate) audit_viewer: Option<crate::audit_viewer::AuditViewer>,
    /// 交互脚本回放弹窗状态。
    pub(crate) replay_modal: Option<crate::record::ReplayModalState>,
    /// 认证阶段由 net 层发起的凭据询问；连接前不再拦截。
    pub(crate) auth_prompt: Option<AuthPromptState>,
    /// TOFU 首连主机指纹确认弹窗；同一时刻最多一个在等用户作答。
    pub(crate) host_key_confirm: Option<HostKeyConfirmState>,
    /// 认证成功后才落盘的凭据。
    pub(crate) pending_credential: Option<PendingCredential>,
    /// known_hosts 管理弹窗的聚合状态（开关、记录与搜索框）。
    pub(crate) known_hosts_modal: KnownHostsModalState,
    /// 命令历史内存快照：终端视图的查询回调共享它，避免每次按键都读盘。
    pub(crate) history: Arc<parking_lot::RwLock<Vec<String>>>,
    /// 共享给每个终端视图的历史查询回调。
    pub(crate) history_provider: transom_terminal::HistoryProvider,
    /// 会话列表行尾的拨测状态点，按会话 id 索引。
    pub(crate) probe_badges: HashMap<SessionId, ProbeBadge>,
    /// 每个状态点的到期清理任务。
    pub(crate) probe_expiry_tasks: HashMap<SessionId, Task<()>>,
    pub(crate) show_quick_open: bool,
    pub(crate) quick_open_state: Entity<CommandState>,
    /// 设置弹窗的聚合状态：开关、密钥向导与全部输入框实体。
    pub(crate) settings_modal: SettingsModalState,
    /// 主机监控看板与进程管理器状态。
    pub(crate) monitor: crate::monitor::MonitorState,
    /// 面板打开后负责每 3 秒拉取进程快照的常驻任务。
    pub(crate) monitor_task: Option<Task<()>>,
    /// Git 定时同步的分钟节拍任务与已累计分钟数。
    pub(crate) git_sync_task: Option<Task<()>>,
    pub(crate) git_sync_minutes: u64,
    pub(crate) confirm_forward_any: Option<crate::forward::PendingForwardConfirm>,
    /// 已打开的远端文本编辑面板。
    pub(crate) editor_panels: Vec<EditorPanelState>,
    /// 被脏编辑面板拦下的标签/会话关闭意图。
    pub(crate) pending_editor_close: Option<PendingEditorClose>,
    /// 工具栏"已打开编辑器"浮层的受控显隐。
    pub(crate) editor_list_open: bool,
    /// SFTP 五类弹窗（权限、重命名等输入、属性、过滤、同步）的聚合状态。
    pub(crate) sftp_modals: SftpModals,
    /// 快捷命令面板的聚合状态：卡片展开开关与新增/编辑小弹窗。
    pub(crate) snippet_panel: SnippetPanelState,
    /// 底部广播输入条：目标集合、单行/多行输入与展开状态。
    pub(crate) broadcast: BroadcastBarState,
    /// 底栏右段的兜底状态文字（没有指标徽标时显示）。
    pub(crate) status: Option<String>,
    /// 全局错误缓冲。
    pub(crate) error: Option<String>,
    pub(crate) sidebar_visible: bool,
    /// 沉浸式全屏：隐藏全部应用外框，仅保留终端视口。
    pub(crate) immersive: bool,
    /// 快捷键域的聚合状态：生效键位过滤器、已应用键位串与改键捕获。
    pub(crate) shortcut_recording: ShortcutRecordingState,
    /// 侧边栏多选模式与已选会话。
    pub(crate) selection_mode: bool,
    pub(crate) selected_sessions: HashSet<SessionId>,
    /// 单击会话行留下的单选高亮。
    pub(crate) tree_selection: Option<SessionId>,
    pub(crate) batch_edit: Option<BatchEditForm>,
    pub(crate) backup_prompt: Option<BackupPromptState>,
    pub(crate) _backup_task: Option<Task<()>>,
    /// 云同步下载合并的后台任务；合并完成后置空。
    pub(crate) _sync_pull_task: Option<Task<()>>,
    /// 系统明暗变化订阅；随 AppShell 存活，drop 即退订。
    pub(crate) _appearance_subscription: Subscription,
    /// 终端外观滑块的订阅句柄；随 AppShell 存活，drop 即退订。
    pub(crate) _terminal_appearance_subscriptions: Vec<Subscription>,
    pub(crate) _event_task: Task<()>,
}

impl AppShell {
    /// 向控制器提交命令的统一入口：非阻塞 try_send，队列满时快速失败。
    pub(crate) fn send_command(&mut self, command: AppCommand) -> bool {
        match self.services.command_tx.try_send(command) {
            Ok(()) => true,
            Err(error) => {
                self.error = Some(command_send_error_text(error));
                false
            }
        }
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
        let workspace_panels = cx.new(|_| ResizableState::default());
        let workspace_panels_left = cx.new(|_| ResizableState::default());
        cx.observe(&workspace_panels, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&workspace_panels_left, |_, _, cx| cx.notify())
            .detach();
        let (catalog, error) = match services.session_store.load() {
            Ok(catalog) => (catalog, None),
            Err(error) => {
                log::error!(
                    "读取会话目录失败，界面按空目录启动，原因: {}",
                    localized_error(&error)
                );
                (SessionCatalog::default(), Some(localized_error(&error)))
            }
        };
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
        let terminal_appearance_subscriptions = vec![window_transparency_sub];
        let sidebar_visible = settings.window.sidebar_visible_at_startup();

        let mut this = Self {
            services,
            settings,
            catalog,
            catalog_revision: 0,
            sidebar_rows_cache: None,
            tabs: Vec::new(),
            groups: Vec::new(),
            tiled_sets: Vec::new(),
            tab_selection: HashSet::new(),
            pane_split_states: HashMap::new(),
            workspace_panels,
            workspace_panels_left,
            active_tab: None,
            search,
            form: None,
            folder_form: None,
            audit_viewer: None,
            replay_modal: None,
            auth_prompt: None,
            host_key_confirm: None,
            pending_credential: None,
            history,
            history_provider,
            known_hosts_modal: KnownHostsModalState {
                open: false,
                records: known_hosts_records,
                search: known_hosts_search,
            },
            probe_badges: HashMap::new(),
            probe_expiry_tasks: HashMap::new(),
            show_quick_open: false,
            quick_open_state,
            monitor: crate::monitor::MonitorState::new(window, cx),
            monitor_task: None,
            git_sync_task: None,
            git_sync_minutes: 0,
            settings_modal,
            confirm_forward_any: None,
            editor_panels: Vec::new(),
            pending_editor_close: None,
            editor_list_open: false,
            sftp_modals: SftpModals {
                chmod: None,
                input: None,
                properties: None,
                filter: None,
                sync: None,
                goto_line: None,
            },
            snippet_panel: SnippetPanelState {
                open: false,
                form: None,
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
            error,
            sidebar_visible,
            immersive: false,
            shortcut_recording: ShortcutRecordingState {
                filter: shortcut_filter,
                applied_keys: applied_shortcut_keys,
                capture: None,
            },
            selection_mode: false,
            tree_selection: None,
            selected_sessions: HashSet::with_capacity(8),
            batch_edit: None,
            backup_prompt: None,
            _backup_task: None,
            _sync_pull_task: None,
            _appearance_subscription: appearance_subscription,
            _terminal_appearance_subscriptions: terminal_appearance_subscriptions,
            _event_task: event_task,
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

        if this.settings.appearance.mode == UiThemeMode::System {
            crate::theme::sync_with_system(window, cx);
        }
        this.sync_terminal_palette(cx);
        this.restore_startup_tabs(window, cx);

        this.git_sync_task = Some(cx.spawn(async move |this, cx| {
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
                        && let Some(index) = self.groups.iter().position(|item| item.id == group)
                    {
                        let pane = self.groups[index].active_pane;
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
        let Some(git) = &self.settings.sync.git else {
            self.git_sync_minutes = 0;
            return;
        };
        if git.interval_minutes == 0 {
            self.git_sync_minutes = 0;
            return;
        }
        self.git_sync_minutes = self.git_sync_minutes.saturating_add(1);
        if self.git_sync_minutes >= git.interval_minutes {
            self.git_sync_minutes = 0;
            self.start_git_sync_scheduled(cx);
        }
    }

    /// 持久化会话与文件夹目录。
    pub(crate) fn persist_catalog(&mut self) -> bool {
        self.catalog_revision = self.catalog_revision.wrapping_add(1);
        match self.services.session_store.save(&self.catalog) {
            Ok(()) => true,
            Err(error) => {
                self.error = Some(localized_error(&error));
                false
            }
        }
    }
}
