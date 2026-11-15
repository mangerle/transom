//! SFTP 弹窗状态定义。

use super::chmod::SftpChmodModalView;
use super::common::*;
use super::confirm::SftpConfirmModalView;
use super::filter::SftpFilterModalView;
use super::input::SftpInputModalView;
use super::properties::SftpPropertiesModalView;
use crate::sftp::{SftpSearchModalView, SftpSyncModalView};

/// 待用户确认的破坏性操作。
#[derive(Clone)]
pub(crate) enum SftpPendingDelete {
    Local {
        tab_id: TabId,
        paths: Vec<PathBuf>,
    },
    Remote {
        tab_id: TabId,
        /// 目标连接；`None` 表示标签页主远端，`Some` 表示并列远端列。
        connection_id: Option<ConnectionId>,
        targets: Vec<RemoteDeleteTarget>,
    },
}

/// 弹窗实体的事件订阅槽位，随弹窗关闭自动释放。
#[derive(Default)]
pub(crate) struct SftpModalSubscriptions {
    pub(crate) chmod: Option<Subscription>,
    pub(crate) input: Option<Subscription>,
    pub(crate) properties: Option<Subscription>,
    pub(crate) filter: Option<Subscription>,
    pub(crate) sync: Option<Subscription>,
    pub(crate) confirm: Option<Subscription>,
    pub(crate) search: Option<Subscription>,
}

/// SFTP 七类模态弹窗的聚合实体句柄。
///
/// 每个弹窗都是独立的 GPUI 实体，通过事件向外壳上交意图，不再共享外壳渲染自由函数。
#[derive(Default)]
pub(crate) struct SftpModals {
    pub(crate) chmod: Option<Entity<SftpChmodModalView>>,
    pub(crate) input: Option<Entity<SftpInputModalView>>,
    pub(crate) properties: Option<Entity<SftpPropertiesModalView>>,
    pub(crate) filter: Option<Entity<SftpFilterModalView>>,
    pub(crate) sync: Option<Entity<SftpSyncModalView>>,
    pub(crate) confirm: Option<Entity<SftpConfirmModalView>>,
    pub(crate) search: Option<Entity<SftpSearchModalView>>,
    pub(crate) subs: SftpModalSubscriptions,
}
