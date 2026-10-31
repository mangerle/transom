//! SFTP 编辑面板的保存回传生命周期：整份缓冲区写回远端、版本指纹冲突的
//! 拦截与回填、以及结果事件的落地。
//!
//! 打开与读取在 `open.rs`，面板状态与窗体动作在 `panel.rs` / `actions.rs`，
//! 关闭保护在 `close.rs`，渲染在 `view.rs`。
//!
//! 设计原理（Why）：
//! - 回传走 `SftpOperation::WriteText` 独立通道，不进传输队列，理由同读取侧
//!   （见 `open.rs` 模块注释）。
//! - 保存请求携带打开或上次回传成功时的版本指纹（乐观锁）：远端已被他人
//!   修改时 net 层拒绝覆盖并回报 `SftpTextConflict`，这里只置冲突横幅交用户
//!   决策（重新载入 / 强制覆盖 / 继续编辑），杜绝静默丢失他人改动。
//! - 两段式拆分：实体只改面板状态并产出决策/请求，外壳负责发命令与开弹窗。
//!   这样实体回调回交外壳时，外壳不必再读写本实体，避免双重租借崩溃。

use super::entity::EditorPanels;
use super::open::parent_dir;
use super::panel::EditorPhase;
use crate::app::AppShell;
use crate::sftp::{SftpInputModalParams, SftpInputTarget};
use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use transom_core::{
    ConnectionId, ErrorCode, RemoteTextMeta, SftpCommand, SftpOperation, SudoCapability,
    SudoUnavailable, TabId, TransferId,
};

mod result;
mod save;

/// 把提权不可用原因翻译成用户能读懂、可据以采取行动的说明。
fn sudo_unavailable_text(reason: SudoUnavailable) -> String {
    match reason {
        SudoUnavailable::NotInstalled => rust_i18n::t!("editor.sudo_not_installed").to_string(),
        SudoUnavailable::RequiresTty => rust_i18n::t!("editor.sudo_requires_tty").to_string(),
        SudoUnavailable::NotAuthorized => rust_i18n::t!("editor.sudo_not_authorized").to_string(),
        SudoUnavailable::Other => rust_i18n::t!("editor.sudo_probe_failed_reason").to_string(),
    }
}
