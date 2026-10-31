//! SFTP 弹窗状态定义。

use super::common::*;

/// 编辑器"跳转到行"弹窗的状态。
pub(crate) struct EditorGotoLineModalState {
    pub(crate) editor: Entity<EditorState>,
    pub(crate) input: Entity<InputState>,
    pub(crate) error: Option<String>,
    pub(crate) _enter_sub: Subscription,
}

/// 待用户确认的破坏性操作。
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

/// 删除确认弹窗：破坏性操作必须经过一次显式确认，且文案说明不可恢复。
pub(crate) struct SftpConfirmModalState {
    pub(crate) action: SftpPendingDelete,
    pub(crate) title: String,
    pub(crate) description: String,
}

/// SFTP 五类模态弹窗与编辑器跳转行的聚合状态。
///
/// 这些弹窗都依附于 SFTP 双栏或内置编辑器，统一由本模块管理显隐；
/// AppShell 只持有聚合对象，不再平铺五个互不相关的 `Option` 字段。
pub(crate) struct SftpModals {
    pub(crate) chmod: Option<SftpChmodModalState>,
    pub(crate) input: Option<SftpInputModalState>,
    pub(crate) properties: Option<SftpPropertiesModalState>,
    pub(crate) filter: Option<SftpFilterModalState>,
    pub(crate) sync: Option<SftpSyncModalState>,
    pub(crate) goto_line: Option<EditorGotoLineModalState>,
    pub(crate) confirm: Option<SftpConfirmModalState>,
    pub(crate) search: Option<crate::sftp::search_modal::SftpSearchModalState>,
}
