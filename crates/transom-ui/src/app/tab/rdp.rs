//! RDP 远程桌面标签页。

use super::{TabContent, WorkspaceTab};
use crate::rdp::RdpView;
use gpui_kit::{Entity, Subscription};
use transom_core::{Session, TabId};

/// RDP 远程桌面专属标签。
pub(crate) struct RdpTab {
    pub(crate) session: Session,
    pub(crate) view: Entity<RdpView>,
    _subscription: Subscription,
}

impl WorkspaceTab {
    /// 构造 RDP 桌面标签并托管视图事件订阅（随标签关闭自动取消）。
    pub(crate) fn new_rdp(
        id: TabId,
        session: Session,
        view: Entity<RdpView>,
        subscription: Subscription,
    ) -> Self {
        Self {
            id,
            title: session.name.clone(),
            status: super::TabStatus::Connecting,
            content: TabContent::Rdp(Box::new(RdpTab {
                session,
                view,
                _subscription: subscription,
            })),
        }
    }

    /// 获取 RDP 视图实体句柄。
    pub(crate) fn rdp_view(&self) -> Option<Entity<RdpView>> {
        match &self.content {
            TabContent::Rdp(rdp) => Some(rdp.view.clone()),
            _ => None,
        }
    }
}
