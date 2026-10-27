//! SFTP 协议传输任务、操作命令与元数据契约定义。

mod ops;
mod path;
mod task;
mod types;

#[cfg(test)]
mod tests;

pub use ops::{PreviewMode, SftpOperation};
pub use path::{
    format_octal_permissions, format_rwx_permissions, is_safe_file_name, parse_octal_permissions,
    safe_local_join,
};
pub use task::{
    TransferDirection, TransferKind, TransferMode, TransferStatus, TransferSummary, TransferTask,
};
pub use types::{RemoteDeleteTarget, RemoteFile, RemoteMetadata, RemoteTextMeta};
