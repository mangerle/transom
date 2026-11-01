//! 后台常驻任务与面板事件订阅的聚合容器。
//!
//! 设计原理（Why）：任务句柄与订阅必须被持有到窗口退出，否则会在下一帧被
//! 静默取消。按职责拆成两组：
//! - [`BackgroundTasks`]：可被替换/取消的异步任务（采样、同步、密钥生成）；
//! - [`PanelSubscriptions`]：实体事件订阅（面板发意图、外壳在此接收）。
//!
//! 两者的生命周期语义不同：任务可以被“顶掉”，订阅必须一直存活到实体销毁。
//!
//! # 代价与局限
//! 容器仍由外壳统一持有；某个任务真正属于哪个领域，需要在新增时判断并
//! 写进对应分组，而不是继续平铺字段。

use gpui_kit::{Subscription, Task};
use std::collections::HashMap;
use transom_core::SessionId;

/// 可替换的后台任务。
pub(crate) struct BackgroundTasks {
    pub(crate) probe_expiry: HashMap<SessionId, Task<()>>,
    pub(crate) monitor: Option<Task<()>>,
    /// 密钥生成任务。
    pub(crate) keygen: Option<Task<()>>,
    pub(crate) git_sync: Option<Task<()>>,
    pub(crate) backup: Option<Task<()>>,
    pub(crate) sync_pull: Option<Task<()>>,
}

impl BackgroundTasks {
    fn new() -> Self {
        Self {
            probe_expiry: HashMap::new(),
            monitor: None,
            keygen: None,
            git_sync: None,
            backup: None,
            sync_pull: None,
        }
    }
}

/// 实体事件订阅：面板发意图、外壳在此持有订阅并处理。
pub(crate) struct PanelSubscriptions {
    pub(crate) sftp: HashMap<transom_core::TabId, Subscription>,
    pub(crate) editor: Option<Subscription>,
    pub(crate) monitor: Option<Subscription>,
    pub(crate) forward: HashMap<transom_core::TabId, Subscription>,
    pub(crate) settings: Option<Subscription>,
}

impl PanelSubscriptions {
    fn new() -> Self {
        Self {
            sftp: HashMap::new(),
            editor: None,
            monitor: None,
            forward: HashMap::new(),
            settings: None,
        }
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
