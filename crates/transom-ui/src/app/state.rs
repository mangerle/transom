//! 应用状态容器：把领域状态从 GPUI 外壳中剥离。
//!
//! # 设计原理（Why）
//! - `AppShell` 只保留一个 `state` 字段，渲染、事件路由与生命周期仍由外壳负责；
//! - 状态字段的可见性收紧为 `pub(crate)`：只有挂在 `app` 下的功能模块
//!   可以访问，crate 其它模块无法绕过外壳直接改状态；
//! - 命令发送、错误入队与目录持久化这类不依赖 GPUI 上下文的操作定义在
//!   [`AppState`] 上，外壳以 `self.state.xxx` 显式访问，避免继续把纯状态操作
//!   堆进外壳；刻意不实现 `Deref`，调用点必须看得出状态归属。
//!
//! # 代价与局限
//! 字段仍是应用级共享状态，新增字段时必须评估是否真的属于全局状态；
//! 功能模块之间仍共享同一份写入权限，后续按领域继续下沉到 feature 结构体。

use super::AppShell;
use super::app_settings;
use super::modal_hub::ModalHub;
use super::persistence::Persistence;
use super::task_hub::TaskHub;
use super::types::PendingNotices;
use super::update_app_settings;
use crate::UiServices;
use crate::broadcast::BroadcastBarState;
use crate::cloud_sync::{CloudSyncView, GitSyncScheduler};
use crate::dialog_host::OpenModals;
use crate::editor::EditorPanels;
use crate::git::GitWorkspace;
use crate::keymap::ShortcutRecordingState;
use crate::palette::localized_error;
use crate::probe::ProbeBadge;
use crate::sidebar::SidebarState;
use crate::workspace::WorkspaceState;
use gpui_kit::component::input::InputState;
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::{App, Context, Entity, Window};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use transom_core::PanelSide;
use transom_core::{AppCommand, AppSettings, GitProjectId, SessionCatalog, SessionId, TabId};
use transom_platform::shell::ShellOption;

/// 后台定时自动化节拍状态。
#[derive(Default)]
pub(crate) struct AutomationState {
    /// 定时 Git 同步的分钟节拍。
    pub(crate) git_sync_schedule: GitSyncScheduler,
    /// 上次容器自动清理时间；关闭自动化后清空，重新开启从头计时。
    pub(crate) container_auto_prune_last: Option<std::time::Instant>,
}

/// 应用级界面反馈、通知队列与帧副作用调度状态。
pub(crate) struct UiFeedbackState {
    /// 底栏一次性提示的自增序号：定时器到期时只清除与自身匹配的那一条。
    pub(crate) status_seq: u64,
    /// 应用级错误通知的待发队列：写入与渲染解耦，逐帧统一投递。
    pub(crate) pending_notices: PendingNotices,
    /// 弹窗内的一次性提示：脏表单被拦下不关时，说明为什么按键没有反应。
    pub(crate) dialog_hint: Option<String>,
    /// 右上角操作结果提示的自增序号：用作通知 key，定时关闭时只关自己那一条。
    pub(crate) toast_seq: u64,
    /// 上一条 Toast 提示的内容与时间戳，用于短时间内的相同通知防抖去重。
    pub(crate) last_toast: Option<(String, std::time::Instant)>,
    /// 当前是否已安排绘制后的帧副作用，避免同一帧重复调度。
    pub(crate) frame_effects_scheduled: bool,
}

impl UiFeedbackState {
    pub(crate) fn new(pending_notices: PendingNotices) -> Self {
        Self {
            status_seq: 0,
            pending_notices,
            dialog_hint: None,
            toast_seq: 0,
            last_toast: None,
            frame_effects_scheduled: false,
        }
    }

    /// 检查短时间内是否刚弹出过相同 Toast，未重复则更新记录并返回 `false`。
    pub(crate) fn should_dedup_toast(&mut self, msg: &str) -> bool {
        let now = std::time::Instant::now();
        if let Some((last_msg, last_time)) = &self.last_toast
            && last_msg == msg
            && now.duration_since(*last_time).as_millis() < 1200
        {
            return true;
        }
        self.last_toast = Some((msg.to_string(), now));
        false
    }
}

/// 进程级运行资源：服务、后台任务、持久化、历史与退出信号。
///
/// 生命周期与应用进程一致，不由单个面板或标签页创建。
pub(crate) struct RuntimeState {
    pub(crate) services: UiServices,
    pub(crate) tasks: TaskHub,
    pub(crate) automation: AutomationState,
    pub(crate) history: Arc<parking_lot::RwLock<Vec<String>>>,
    /// 补全候选来源：汇集历史与片段，缓存候选池；设置变更后需显式同步。
    pub(crate) completion: Arc<super::CompletionSource>,
    pub(crate) completion_provider: transom_terminal::CompletionProvider,
    pub(crate) persistence: Persistence,
    pub(crate) restart_requested: Arc<AtomicBool>,
}

/// 会话目录域：本机 Shell、会话/文件夹目录、侧栏与拨测状态。
///
/// 生命周期与窗口中的会话目录一致，修改后需要整体持久化。
pub(crate) struct SessionState {
    pub(crate) local_shells: Arc<Vec<ShellOption>>,
    pub(crate) catalog: SessionCatalog,
    pub(crate) catalog_revision: u64,
    pub(crate) sidebar: SidebarState,
    pub(crate) session_sidebar: Entity<crate::sidebar::SessionSidebarView>,
    pub(crate) probe_badges: HashMap<SessionId, ProbeBadge>,
}

/// 窗口级界面状态：弹窗、布局句柄、焦点输入与即时反馈。
pub(crate) struct UiState {
    pub(crate) workspace_panels: Entity<ResizableState>,
    pub(crate) workspace_panels_left: Entity<ResizableState>,
    pub(crate) vertical_tabs_search: Entity<InputState>,
    pub(crate) modals: ModalHub,
    pub(crate) status: Option<String>,
    pub(crate) immersive: bool,
    pub(crate) window_requests: super::WindowRequests,
    pub(crate) shortcut_recording: ShortcutRecordingState,
    pub(crate) open_modals: OpenModals,
    pub(crate) feedback: UiFeedbackState,
}

/// 窗口级功能实体及其共享运行开关。
pub(crate) struct FeatureState {
    pub(crate) cloud_sync: Entity<CloudSyncView>,
    /// 窗口级应用更新状态：启动检查、侧栏提示与更新弹窗共享同一份状态机。
    pub(crate) update: Entity<crate::app::update::UpdateView>,
    pub(crate) monitor: Entity<crate::monitor::MonitorView>,
    pub(crate) containers: crate::container::ContainerUiState,
    pub(crate) git_sidebar: Entity<crate::git::GitSidebarView>,
    pub(crate) editors: Entity<EditorPanels>,
    pub(crate) diff_viewer: Option<Entity<crate::diff_viewer::DiffViewerView>>,
    pub(crate) broadcast: BroadcastBarState,
    pub(crate) status_metrics: Entity<crate::status_bar::StatusBarMetricsView>,
    pub(crate) stats_sampling: bool,
}

/// 应用运行期状态。
///
/// 顶层只按生命周期聚合，不再平铺三十余个字段；具体业务访问继续显式写
/// `state.runtime.xxx`、`state.session.xxx`、`state.ui.xxx` 或 `state.features.xxx`。
pub(crate) struct AppState {
    pub(crate) runtime: RuntimeState,
    pub(crate) session: SessionState,
    pub(crate) workspace: WorkspaceState,
    pub(crate) ui: UiState,
    pub(crate) features: FeatureState,
}

impl AppState {
    /// 当前焦点标签是否承载可输入终端。
    ///
    /// 快捷命令、广播与录制都直接作用于终端；容器管理、录像回放与主机监控
    /// 没有当前终端目标，状态栏据此隐藏这些入口，避免点击后无响应。
    pub(crate) fn active_tab_has_terminal(&self) -> bool {
        self.workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tab(tab_id))
            .is_some_and(|tab| tab.terminal().is_some())
    }

    /// 按标签 id 取 Git 工作区实体句柄。
    ///
    /// 返回的是句柄而不是引用：调用方自行决定 `read` 还是 `update`，避免同一
    /// 标签页在渲染期与动作期各持一份状态。`tab_id` 不存在或不是 Git 标签时
    /// 返回 `None`。
    pub(crate) fn git_workspace(&self, tab_id: TabId) -> Option<Entity<GitWorkspace>> {
        let index = self.workspace.tab_index(tab_id)?;
        self.workspace.tabs[index].git_workspace()
    }

    /// 按项目 id 取 Git 工作区实体句柄。
    ///
    /// 项目 id 存在实体内部，读取需要只读的 `App` 上下文；查找口径与旧实现
    /// 一致——按标签出现顺序取第一个打开该项目的 Git 标签。
    pub(crate) fn git_workspace_of_project(
        &self,
        project_id: GitProjectId,
        cx: &App,
    ) -> Option<Entity<GitWorkspace>> {
        self.workspace.git_workspace_of_project(project_id, cx)
    }

    /// 让设置实体执行保存：表单收集、主题校验与窗口外观落定都在实体内部完成。
    ///
    /// 设计原理（Why）：保存动作依赖 `Window` 与实体上下文，但不依赖外壳本身；
    /// 下沉到状态层后，弹窗提交路径与设置页按钮都能复用同一入口。
    pub(crate) fn save_settings(&mut self, window: &mut Window, cx: &mut Context<AppShell>) {
        let view = self.ui.modals.settings.clone();
        view.update(cx, |view, cx| view.save_settings(window, cx));
    }

    /// 让设置实体执行取消：回滚外观快照并关闭弹窗。
    pub(crate) fn cancel_settings(&mut self, window: &mut Window, cx: &mut Context<AppShell>) {
        let view = self.ui.modals.settings.clone();
        view.update(cx, |view, cx| view.cancel_settings(window, cx));
    }

    /// 把当前布局设置快照落盘并刷新界面。
    ///
    /// 设计原理（Why）：活动栏/状态栏拖拽、片段与监控布局都只改设置快照，
    /// 不依赖外壳；下沉到状态层后这些调用方不必再持整壳。
    pub(crate) fn persist_layout_settings(&mut self, cx: &mut Context<AppShell>) {
        let snapshot = app_settings(cx).clone();
        self.persist_settings_reported(&snapshot);
        cx.notify();
    }

    /// 切换 SFTP/转发工具面板的停靠方向，并落盘。
    pub(crate) fn set_sftp_panel_side(&mut self, side: PanelSide, cx: &mut Context<AppShell>) {
        if app_settings(cx).window.sftp_panel_side == side {
            return;
        }
        update_app_settings(cx, |s| s.window.sftp_panel_side = side);
        std::mem::swap(
            &mut self.ui.workspace_panels,
            &mut self.ui.workspace_panels_left,
        );
        let snapshot = app_settings(cx).clone();
        self.persist_settings_reported(&snapshot);
        cx.notify();
    }

    /// 打开备份导出弹窗：注入当前目录快照后交给实体。
    pub(crate) fn open_backup_export(&mut self, window: &mut Window, cx: &mut Context<AppShell>) {
        let catalog = self.session.catalog.clone();
        let view = self.ui.modals.tools.backup_prompt.clone();
        view.update(cx, |view, cx| {
            view.set_catalog(catalog);
            view.open_export(window, cx);
        });
        cx.notify();
    }

    /// 先让用户选文件，再打开备份导入弹窗。
    pub(crate) fn open_backup_import(&mut self, window: &mut Window, cx: &mut Context<AppShell>) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("backup.import_title").to_string())
            .add_filter("Transom Backup", &["json", "yaml", "yml"])
            .pick_file()
        else {
            return;
        };
        let catalog = self.session.catalog.clone();
        let view = self.ui.modals.tools.backup_prompt.clone();
        view.update(cx, |view, cx| {
            view.set_catalog(catalog);
            view.open_import(path, window, cx);
        });
        cx.notify();
    }

    /// 向控制器提交命令：非阻塞 `try_send`，队列满时快速失败。
    pub(crate) fn send_command(&mut self, command: impl Into<AppCommand>) -> bool {
        match self.runtime.services.command_tx.try_send(command.into()) {
            Ok(()) => true,
            Err(error) => {
                self.set_error(super::command_send_error_text(error));
                false
            }
        }
    }

    /// 设置会在固定时间后清除的底栏状态提示。
    ///
    /// 这是纯状态写入加定时清理的组合；实体和外壳只需传入文案，不持有
    /// `AppShell`，跨域动作也因此可以在窄上下文中复用。
    pub(crate) fn set_transient_status(
        &mut self,
        message: impl Into<String>,
        cx: &mut Context<AppShell>,
    ) {
        let text = message.into();
        self.ui.status = Some(text.clone());
        self.ui.feedback.status_seq = self.ui.feedback.status_seq.wrapping_add(1);
        let seq = self.ui.feedback.status_seq;
        cx.notify();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(4))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.state.ui.feedback.status_seq == seq
                    && this.state.ui.status.as_deref() == Some(text.as_str())
                {
                    this.state.ui.status = None;
                    crate::status_bar::refresh_status_metrics(&mut this.state, cx);
                    cx.notify();
                }
            });
        });
        self.runtime.tasks.background.status_clear = Some(task);
    }

    /// 记录一条应用级错误，由下一次渲染统一转成通知。
    ///
    /// 设计原理（Why）：错误写入点多数不持有窗口句柄，无法在写入时推送通知；
    /// 单槽暂存又会让同一帧内的多次失败互相覆盖。入队后由渲染统一投递，
    /// 既保留每条错误，也让渲染只负责取队列、不再承担“暂存”职责。
    pub(crate) fn set_error(&mut self, message: impl Into<String>) {
        self.ui.feedback.pending_notices.push(message);
    }

    /// 持久化会话与文件夹目录。
    ///
    /// 返回 `false` 表示保存失败，调用方可据此决定是否继续后续动作。
    /// 版本号只在保存成功后推进：失败时侧栏缓存不应失效，避免一次失败的
    /// 写盘让下一次渲染做无意义的全量行重建。
    pub(crate) fn persist_catalog(&mut self) -> bool {
        // 内存态先推进版本：侧栏缓存需要在本次修改后立即失效，不能等磁盘。
        let catalog = self.session.catalog.clone();
        match self.runtime.persistence.save_catalog(&catalog) {
            Ok(()) => {
                self.session.catalog_revision = self.session.catalog_revision.wrapping_add(1);
                true
            }
            Err(error) => {
                self.set_error(localized_error(&error));
                false
            }
        }
    }

    /// 设置快照入队异步落盘。
    ///
    /// 高频开关（监控板块折叠、窗口外观等）走这条路径，UI 不做同步 `fsync`；
    /// 写盘线程不可用时退回同步写并把错误交给调用方。
    pub(crate) fn persist_settings(
        &self,
        settings: &AppSettings,
    ) -> Result<(), transom_core::AppError> {
        self.runtime.persistence.save_settings(settings)
    }

    /// 统一的“保存并上报”入口：失败时记录日志并写入应用级错误队列。
    ///
    /// 高频开关不应各自决定“失败要不要提示”；统一走这里，避免再次出现
    /// `let _ = save(...)` 这类静默降级。
    pub(crate) fn persist_settings_reported(&mut self, settings: &AppSettings) -> bool {
        match self.persist_settings(settings) {
            Ok(()) => true,
            Err(error) => {
                log::error!("保存设置失败，界面仍是新值但下次启动会回退，原因: {error}");
                self.set_error(localized_error(&error));
                false
            }
        }
    }

    /// 取走后台写盘线程上报的错误。
    pub(crate) fn take_persistence_error(&self) -> Option<transom_core::AppError> {
        self.runtime.persistence.take_error()
    }
}
