//! 后台常驻与生命周期任务聚合容器。
//!
//! 设计说明（Why）：
//! - 实现初衷：将散落在 `AppShell` 结构体中的各类定时轮询、拨测超时、
//!   数据备份与事件循环 `Task<()>` 句柄统一纳管；
//! - 核心优势：
//!   1. 集中监控并显式管理全部异步任务的生命周期；
//!   2. 避免单任务泄漏或在窗口退出时未被正确中止；
//!   3. 清晰区分界面视图状态与后台异步副作用。

use gpui_kit::{Subscription, Task};
use std::collections::HashMap;
use transom_core::SessionId;

/// 聚合应用外壳持有的全部后台异步任务与事件订阅。
pub(crate) struct TaskHub {
    pub(crate) probe_expiry: HashMap<SessionId, Task<()>>,
    pub(crate) monitor: Option<Task<()>>,
    /// 密钥生成任务。
    pub(crate) keygen: Option<Task<()>>,
    pub(crate) git_sync: Option<Task<()>>,
    pub(crate) backup: Option<Task<()>>,
    pub(crate) sync_pull: Option<Task<()>>,
    pub(crate) _appearance_subscription: Subscription,
    /// 面板事件订阅：面板发意图、外壳在此持有订阅并处理。
    pub(crate) sftp_panel_subscriptions: HashMap<transom_core::TabId, Subscription>,
    /// 编辑面板事件订阅。
    pub(crate) editor_panel_subscription: Option<Subscription>,
    /// 监控面板事件订阅。
    pub(crate) monitor_panel_subscription: Option<Subscription>,
    /// 设置实体事件订阅。
    pub(crate) settings_subscription: Option<Subscription>,
    /// 转发面板事件订阅。
    pub(crate) forward_panel_subscriptions: HashMap<transom_core::TabId, Subscription>,
    pub(crate) _event: Task<()>,
}

impl TaskHub {
    /// 构造新的任务聚合容器。
    pub(crate) fn new(appearance_subscription: Subscription, event: Task<()>) -> Self {
        Self {
            probe_expiry: HashMap::new(),
            monitor: None,
            keygen: None,
            git_sync: None,
            backup: None,
            sync_pull: None,
            _appearance_subscription: appearance_subscription,
            sftp_panel_subscriptions: HashMap::new(),
            forward_panel_subscriptions: HashMap::new(),
            monitor_panel_subscription: None,
            settings_subscription: None,
            editor_panel_subscription: None,
            _event: event,
        }
    }
}
