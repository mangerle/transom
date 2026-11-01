//! 应用状态容器：把领域状态从 GPUI 外壳中剥离。
//!
//! # 设计原理（Why）
//! - `AppShell` 只保留一个 `state` 字段，渲染、事件路由与生命周期仍由外壳负责；
//! - 状态字段的可见性收紧为 `pub(in crate::app)`：只有挂在 `app` 下的功能模块
//!   可以访问，crate 其它模块无法绕过外壳直接改状态；
//! - 命令发送、错误入队与目录持久化这类不依赖 GPUI 上下文的操作定义在
//!   [`AppState`] 上，外壳以 `self.state.xxx` 显式访问，避免继续把纯状态操作
//!   堆进外壳；刻意不实现 `Deref`，调用点必须看得出状态归属。
//!
//! # 代价与局限
//! 字段仍是应用级共享状态，新增字段时必须评估是否真的属于全局状态；
//! 功能模块之间仍共享同一份写入权限，后续按领域继续下沉到 feature 结构体。

use super::broadcast::BroadcastBarState;
use super::dialog_host::OpenModals;
use super::modal_hub::ModalHub;
use super::persistence::Persistence;
use super::probe::ProbeBadge;
use super::task_hub::TaskHub;
use super::types::PendingNotices;
use crate::UiServices;
use crate::cloud_sync::GitSyncScheduler;
use crate::editor::EditorPanels;
use crate::keymap::ShortcutRecordingState;
use crate::palette::localized_error;
use crate::sftp::SftpModals;
use crate::sidebar::SidebarState;
use crate::workspace::WorkspaceState;
use gpui_kit::Entity;
use gpui_kit::component::input::InputState;
use gpui_kit::component::resizable::ResizableState;
use std::collections::HashMap;
use std::sync::Arc;
use transom_core::{AppCommand, AppSettings, SessionCatalog, SessionId};

/// 应用运行期状态。
pub(crate) struct AppState {
    pub(in crate::app) services: UiServices,
    /// 会话与文件夹共享同一份目录快照，任何一方变化都整体持久化。
    pub(in crate::app) catalog: SessionCatalog,
    /// 会话目录版本：每次持久化（= 目录被修改）递增，侧栏行缓存以此为失效键。
    pub(in crate::app) catalog_revision: u64,
    /// 统一托管的侧栏核心状态与选择集。
    pub(in crate::app) sidebar: SidebarState,
    /// 统一托管的工作区多标签与分屏网格核心状态。
    pub(in crate::app) workspace: WorkspaceState,
    /// 工作区分组（侧栏、主区、SFTP、转发）的列宽状态。
    pub(in crate::app) workspace_panels: Entity<ResizableState>,
    /// 停靠左/右各自独立的列宽状态。
    pub(in crate::app) workspace_panels_left: Entity<ResizableState>,
    /// 纵向标签面板的搜索框状态。
    pub(in crate::app) vertical_tabs_search: Entity<InputState>,
    /// 左侧槽位当前是否显示纵向标签面板；为假时槽位显示会话树或收起。
    pub(in crate::app) vertical_tabs_open: bool,
    /// 统一托管的应用顶层交互模态与表单集合。
    pub(in crate::app) modals: ModalHub,
    /// 统一纳管的应用外壳后台任务与事件订阅。
    pub(in crate::app) tasks: TaskHub,
    /// 定时 Git 同步的分钟节拍。
    pub(in crate::app) git_sync_schedule: GitSyncScheduler,
    /// 命令历史内存快照：终端视图的查询回调共享它，避免每次按键都读盘。
    pub(in crate::app) history: Arc<parking_lot::RwLock<Vec<String>>>,
    /// 共享给每个终端视图的历史查询回调。
    pub(in crate::app) history_provider: transom_terminal::HistoryProvider,
    /// 会话列表行尾的拨测状态点，按会话 id 索引。
    pub(in crate::app) probe_badges: HashMap<SessionId, ProbeBadge>,
    /// 主机监控看板与进程管理器状态。
    pub(in crate::app) monitor: Entity<crate::monitor::MonitorView>,
    /// 远端文本编辑面板实体：面板集合、关闭拦截与渲染都归它。
    pub(in crate::app) editors: Entity<EditorPanels>,
    /// SFTP 五类弹窗（权限、重命名等输入、属性、过滤、同步）的聚合状态。
    pub(in crate::app) sftp_modals: SftpModals,
    /// 远程与本地文件分栏对比面板；同一时刻至多一个。
    pub(in crate::app) diff_viewer: Option<Entity<crate::diff_viewer::DiffViewerView>>,
    /// 底部广播输入条：目标集合、单行/多行输入与展开状态。
    pub(in crate::app) broadcast: BroadcastBarState,
    /// 底栏右段的兜底状态文字（没有指标徽标时显示）。
    pub(in crate::app) status: Option<String>,
    /// 会话目录与设置的覆盖式异步写盘通道。
    pub(in crate::app) persistence: Persistence,
    /// 应用级错误通知的待发队列：写入与渲染解耦，逐帧统一投递。
    pub(in crate::app) pending_notices: PendingNotices,
    /// 沉浸式全屏：隐藏全部应用外框，仅保留终端视口。
    pub(in crate::app) immersive: bool,
    /// SFTP 面板上次同步到的目标宽度；目标变了才调整面板宽度。
    pub(in crate::app) sftp_panel_width_target: Option<f32>,
    /// 各面板/实体上报的窗口动作；渲染期一次取空并执行。
    pub(in crate::app) window_requests: super::WindowRequests,
    /// 状态栏右侧指标簇实体：统计/流量事件只重绘它，不重建整窗。
    pub(in crate::app) status_metrics: Entity<crate::status_bar::StatusBarMetricsView>,
    /// 密钥生成/部署向导状态：独立于设置草稿，随弹窗开关生死。
    pub(in crate::app) key_wizard: Option<crate::keys::KeyWizardState>,
    /// 当前是否要求控制面采样系统统计；界面完全没有消费方时置假。
    pub(in crate::app) stats_sampling: bool,
    /// 快捷键域的聚合状态：生效键位过滤器、已应用键位串与改键捕获。
    pub(in crate::app) shortcut_recording: ShortcutRecordingState,
    /// 组件库弹窗栈的影子记录：与容器实际栈同序，逐帧对账的依据。
    pub(in crate::app) open_modals: OpenModals,
    /// 弹窗内的一次性提示：脏表单被拦下不关时，说明为什么按键没有反应。
    pub(in crate::app) dialog_hint: Option<String>,
}

impl AppState {
    /// 向控制器提交命令：非阻塞 `try_send`，队列满时快速失败。
    pub(crate) fn send_command(&mut self, command: impl Into<AppCommand>) -> bool {
        match self.services.command_tx.try_send(command.into()) {
            Ok(()) => true,
            Err(error) => {
                self.set_error(super::command_send_error_text(error));
                false
            }
        }
    }

    /// 记录一条应用级错误，由下一次渲染统一转成通知。
    ///
    /// 设计原理（Why）：错误写入点多数不持有窗口句柄，无法在写入时推送通知；
    /// 单槽暂存又会让同一帧内的多次失败互相覆盖。入队后由渲染统一投递，
    /// 既保留每条错误，也让渲染只负责取队列、不再承担“暂存”职责。
    pub(crate) fn set_error(&mut self, message: impl Into<String>) {
        self.pending_notices.push(message);
    }

    /// 持久化会话与文件夹目录。
    ///
    /// 返回 `false` 表示保存失败，调用方可据此决定是否继续后续动作。
    /// 版本号只在保存成功后推进：失败时侧栏缓存不应失效，避免一次失败的
    /// 写盘让下一次渲染做无意义的全量行重建。
    pub(crate) fn persist_catalog(&mut self) -> bool {
        // 内存态先推进版本：侧栏缓存需要在本次修改后立即失效，不能等磁盘。
        let catalog = self.catalog.clone();
        match self.persistence.save_catalog(&catalog) {
            Ok(()) => {
                self.catalog_revision = self.catalog_revision.wrapping_add(1);
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
        self.persistence.save_settings(settings)
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
        self.persistence.take_error()
    }
}
