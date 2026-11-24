//! CLIPRDR 通道与剪贴板桥接。
//!
//! 设计原理（Why）：RDP 剪贴板重定向通道（CLIPRDR）处理文本、位图与文件流。
//! 必须遵循局部容错原则：剪贴板格式协商失败或对端格式不规范，一律只记日志
//! 并局部降级，绝对严禁向外抛出致命错误导致主桌面会话中断。

pub mod backend;
pub mod file_stream;
pub mod text;

pub use backend::{CliprdrSyncContext, TransomCliprdrBackend};
pub use file_stream::FileTransferEvent;

/// 剪贴板事件流转定义。
#[derive(Debug, Clone)]
pub enum ClipboardEvent {
    /// 收到来自远端的 UTF-8 纯文本内容。
    RemoteText(String),
    /// 远端复制了文件列表（提供下载到本地的机会）。
    RemoteFilesAvailable(Vec<String>),
    /// 本地向远端发送文本剪贴板更新。
    LocalTextUpdate(String),
}
