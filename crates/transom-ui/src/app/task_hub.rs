//! 后台常驻任务与面板事件订阅的聚合容器。
//!
//! 设计原理（Why）：任务句柄与订阅必须被持有到窗口退出，否则会在下一帧被
//! 静默取消。按职责拆成两组：
//! - [`BackgroundTasks`]：可被替换/取消的异步任务（采样、同步）；
//! - [`PanelSubscriptions`]：实体事件订阅（面板发意图、外壳在此接收）。
//!
//! 两者的生命周期语义不同：任务可以被“顶掉”，订阅必须一直存活到实体销毁。
//!
//! # 代价与局限
//! 容器仍由外壳统一持有；某个任务真正属于哪个领域，需要在新增时判断并
//! 写进对应分组，而不是继续平铺字段。

use gpui_kit::{Subscription, Task};
use std::collections::{HashMap, VecDeque};
use transom_core::{SessionId, TabId};

/// 可替换的后台任务。
pub(crate) struct BackgroundTasks {
    pub(crate) probe_expiry: HashMap<SessionId, Task<()>>,
    pub(crate) monitor: Option<Task<()>>,
    pub(crate) git_sync: Option<Task<()>>,
    pub(crate) sync_pull: Option<Task<()>>,
    pub(crate) sync_history_load: Option<Task<()>>,
    /// 首帧稳定后的升级确认任务：延迟执行，避免把"窗口刚创建"误判为健康运行。
    pub(crate) update_confirm: Option<Task<()>>,
    /// Toast 自动关闭任务；有界保留最近任务，窗口关闭时统一取消。
    pub(crate) toast_tasks: VecDeque<(u64, Task<()>)>,
    /// 底栏一次性状态提示自动清空任务；新提示会替换旧任务从而取消旧定时。
    pub(crate) status_clear: Option<Task<()>>,
}

impl BackgroundTasks {
    fn new() -> Self {
        Self {
            probe_expiry: HashMap::new(),
            monitor: None,
            git_sync: None,
            sync_pull: None,
            sync_history_load: None,
            update_confirm: None,
            toast_tasks: VecDeque::with_capacity(32),
            status_clear: None,
        }
    }
}

/// 实体事件订阅：面板发意图、外壳在此持有订阅并处理。
pub(crate) struct PanelSubscriptions {
    pub(crate) sftp: HashMap<transom_core::TabId, Subscription>,
    pub(crate) editor: Option<Subscription>,
    pub(crate) monitor: Option<Subscription>,
    pub(crate) forward: HashMap<transom_core::TabId, Subscription>,
    /// 容器工作区发往外壳的意图事件：一个标签一份订阅，按标签回收。
    pub(crate) container_workspaces: HashMap<TabId, Subscription>,
    /// 容器引擎侧栏发往外壳的意图事件：侧栏是单例实体。
    pub(crate) container_sidebar: Option<Subscription>,
    pub(crate) settings: Option<Subscription>,
    /// 会话表单：保存落目录与删除会话由外壳处理。
    pub(crate) session_form: Option<Subscription>,
    /// 容器引擎连接表单：保存落目录与删除连接由外壳处理。
    pub(crate) container_form: Option<Subscription>,
    /// 运行容器表单：提交后的命令投递与通知由外壳处理。
    pub(crate) container_run_form: Option<Subscription>,
    /// 拉取/新建弹窗：主输入校验、命令投递与关闭由外壳处理。
    pub(crate) container_create: Option<Subscription>,
    pub(crate) known_hosts: Option<Subscription>,
    /// 连接确认弹窗：凭据暂存与剪贴板授权由外壳处理。
    pub(crate) connection_prompts: Option<Subscription>,
    /// 密钥向导：状态栏提示与命令发送失败由外壳处理。
    pub(crate) key_wizard: Option<Subscription>,
    /// 备份弹窗结果：目录合并与状态栏提示由外壳处理。
    pub(crate) backup: Option<Subscription>,
    /// 审计查看器：状态栏提示、错误与“打开设置”意图由外壳处理。
    pub(crate) audit_viewer: Option<Subscription>,
    /// 会话回放弹窗：打开播放器、终端回放与外部文件选择由外壳处理。
    pub(crate) replay: Option<Subscription>,
    pub(crate) diff_viewer: Option<Subscription>,
    /// 订阅本身即生命周期守卫，无需读取。
    pub(crate) _status_metrics: Option<Subscription>,
    pub(crate) _git_focus: Option<Subscription>,
}

impl PanelSubscriptions {
    fn new() -> Self {
        Self {
            sftp: HashMap::new(),
            editor: None,
            monitor: None,
            forward: HashMap::new(),
            container_workspaces: HashMap::new(),
            container_sidebar: None,
            settings: None,
            session_form: None,
            container_form: None,
            container_run_form: None,
            container_create: None,
            known_hosts: None,
            connection_prompts: None,
            key_wizard: None,
            backup: None,
            audit_viewer: None,
            replay: None,
            diff_viewer: None,
            _status_metrics: None,
            _git_focus: None,
        }
    }

    /// 丢弃已关闭标签的容器工作区事件订阅。
    ///
    /// 设计原理（Why）：订阅的生命周期必须跟随标签，而不是跟随进程。关闭窗格、
    /// 关闭整组、删除会话与连接失败四条路径最终都会走外壳的存活标签裁剪，因此
    /// 集中在这里按 `TabId` 回收即可，避免某个关闭入口漏掉后订阅一直留在映射里。
    pub(crate) fn prune_container_workspaces(&mut self, alive: &[TabId]) {
        self.container_workspaces
            .retain(|tab_id, _| alive.contains(tab_id));
    }
}

/// 聚合应用外壳持有的全部后台异步任务与事件订阅。
pub(crate) struct TaskHub {
    pub(crate) background: BackgroundTasks,
    pub(crate) subscriptions: PanelSubscriptions,
    pub(crate) _appearance_subscription: Subscription,
    pub(crate) _event: Task<()>,
}

impl TaskHub {
    /// 构造新的任务聚合容器。
    pub(crate) fn new(appearance_subscription: Subscription, event: Task<()>) -> Self {
        Self {
            background: BackgroundTasks::new(),
            subscriptions: PanelSubscriptions::new(),
            _appearance_subscription: appearance_subscription,
            _event: event,
        }
    }
}
