//! SFTP 编辑面板的保存回传生命周期：整份缓冲区写回远端、版本指纹冲突的
//! 拦截与回填、以及结果事件的落地。
//!
//! 打开与读取在 `editor_open.rs`，面板状态与窗体动作在 `editor_panel.rs`，
//! 关闭保护在 `editor_close.rs`，渲染在 `editor_view.rs`。
//!
//! 设计原理（Why）：
//! - 回传走 `SftpOperation::WriteText` 独立通道，不进传输队列，理由同读取侧
//!   （见 `editor_open.rs` 模块注释）。
//! - 保存请求携带打开或上次回传成功时的版本指纹（乐观锁）：远端已被他人
//!   修改时 net 层拒绝覆盖并回报 `SftpTextConflict`，这里只置冲突横幅交用户
//!   决策（重新载入 / 强制覆盖 / 继续编辑），杜绝静默丢失他人改动。

use super::open::parent_dir;
use super::panel::{EditorPanelState, EditorPhase};
use crate::app::AppShell;
use crate::keymap;
use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use transom_core::{
    ConnectionId, ErrorCode, RemoteTextMeta, SftpCommand, SftpOperation, SudoCapability,
    SudoUnavailable, TabId, TransferId,
};

use crate::sftp::{SftpInputModalParams, SftpInputTarget};

/// 编辑回传通道：普通写回或经 sudo 提权写回。
///
/// 两条通道的准入闸门、载荷来源与投递失败兜底完全一致，差异只在命令变体
/// 与成功后清理的状态位；用一个枚举把它们收进同一投递路径，避免两份生命周期
/// 各自演化后出现口径漂移。
enum EditorWriteMode {
    /// 普通通道；`force` 为冲突横幅上显式触发的强制覆盖。
    Normal { force: bool },
    /// sudo 通道；`password` 为口令，`None` 表示探测确认可免密。
    Sudo { password: Option<String> },
}

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
