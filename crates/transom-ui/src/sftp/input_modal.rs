//! SFTP 输入弹窗的共享装配：新建、重命名、软链接、归档与提权口令共用。
//!
//! 设计原理（Why）：这些弹窗只有文案、初始值与提交目标不同；此前每个入口
//! 各写一遍“建输入框 → 聚焦 → 填 modal 状态”，新增字段时容易漏改。收敛到
//! 本文件后，输入框焦点与弹窗状态只有一个装配点。

use crate::app::AppShell;
use crate::sftp::SftpInputTarget;
use gpui_kit::*;
use transom_core::{ConnectionId, TabId};

/// 输入弹窗的装配参数。
#[derive(Clone)]
pub(crate) struct SftpInputModalParams {
    pub(crate) tab_id: TabId,
    /// `None` 表示主远端；`Some` 表示并列远端列。本地目标也使用 `None`。
    pub(crate) connection_id: Option<ConnectionId>,
    pub(crate) title: String,
    pub(crate) prompt: String,
    pub(crate) placeholder: String,
    pub(crate) value: String,
    pub(crate) target: SftpInputTarget,
}

/// 装配并打开一个 SFTP 输入弹窗。
pub(crate) fn open_sftp_input_modal(
    shell: &mut AppShell,
    params: SftpInputModalParams,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    crate::sftp::subscribe_sftp_input_modal(shell, params, window, cx);
}
