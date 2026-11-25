//! SFTP 文件传输、列表扫描与文本编辑事件。

use crate::error::AppError;
use crate::ids::{ConnectionId, TabId, TransferId};
use crate::protocol::sftp::{
    RemoteFile, RemoteMetadata, RemoteSearchResultItem, RemoteTextMeta, SudoCapability,
};
use crate::protocol::sftp_sync::DirComparison;

/// SFTP 文件传输、列表扫描与文本编辑事件。
pub enum SftpAppEvent {
    SftpListing {
        tab_id: TabId,
        connection_id: ConnectionId,
        path: String,
        entries: Vec<RemoteFile>,
    },
    SftpProgress {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        transferred: u64,
        total: Option<u64>,
        speed_bps: Option<u64>,
        eta_secs: Option<u64>,
    },
    /// 传输完成、进入完整性校验阶段。
    SftpVerifying {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
    },
    /// 校验不一致后的自动恢复：丢弃目标端内容，从头覆盖重传。
    ///
    /// 设计原理（Why）：校验的意义是“发现不一致并恢复”，不是只报一个错让
    /// 用户手点重试。续传前缀可能因为源文件在中断期间被修改而失真，此时唯一
    /// 正确的恢复方式是从头覆盖；把它做成独立事件，界面才能把任务从
    /// “校验中”切回“传输中”并把进度归零，而不是停在一条自相矛盾的文案上。
    SftpRetrying {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        /// 触发重传的不一致原因，供状态栏与日志定位。
        reason: String,
    },
    SftpFinished {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        transferred: Option<u64>,
        skipped: u64,
        verified: bool,
    },
    SftpMetadata {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        metadata: RemoteMetadata,
    },
    SftpPreview {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        bytes: Vec<u8>,
        truncated: bool,
    },
    SftpPreviewFailed {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        error: AppError,
    },
    SftpTextRead {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        content: String,
        meta: RemoteTextMeta,
    },
    SftpTextWritten {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        meta: RemoteTextMeta,
    },
    SftpTextFailed {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        error: AppError,
    },
    SftpTextConflict {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        error: AppError,
    },
    /// 远端提权能力探测结果：界面据此决定是弹口令框、直接提交写入，还是就地说明
    /// 为什么提权不可用。
    SudoProbed {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        capability: SudoCapability,
    },
    SftpDirCompared {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        comparison: DirComparison,
    },
    SftpSearchResults {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        items: Vec<RemoteSearchResultItem>,
    },
    SftpFailed {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        error: AppError,
    },
}
