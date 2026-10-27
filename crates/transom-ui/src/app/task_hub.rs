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
    pub(crate) git_sync: Option<Task<()>>,
    pub(crate) git_sync_minutes: u64,
    pub(crate) backup: Option<Task<()>>,
    pub(crate) sync_pull: Option<Task<()>>,
    pub(crate) _appearance_subscription: Subscription,
    pub(crate) _terminal_appearance_subscriptions: Vec<Subscription>,
    pub(crate) _event: Task<()>,
}

impl TaskHub {
    /// 构造新的任务聚合容器。
    pub(crate) fn new(
        appearance_subscription: Subscription,
        terminal_appearance_subscriptions: Vec<Subscription>,
        event: Task<()>,
    ) -> Self {
        Self {
            probe_expiry: HashMap::new(),
            monitor: None,
            git_sync: None,
            git_sync_minutes: 0,
            backup: None,
            sync_pull: None,
            _appearance_subscription: appearance_subscription,
            _terminal_appearance_subscriptions: terminal_appearance_subscriptions,
            _event: event,
        }
    }
}
