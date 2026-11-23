//! CLIPRDR 通道与剪贴板桥接。
//!
//! 设计原理（Why）：RDP 剪贴板重定向通道（CLIPRDR）处理文本、位图与文件流。
//! 必须遵循局部容错原则：剪贴板格式协商失败或对端格式不规范，一律只记日志
//! 并局部降级，绝对严禁向外抛出致命错误导致主桌面会话中断。

pub mod file_stream;
pub mod text;

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

/// 剪贴板通道管理状态。
#[derive(Default)]
pub struct ClipboardManager {
    /// 最近一次同步的文本指纹/内容，避免双向回环风暴。
    last_synced_text: Option<String>,
}

impl ClipboardManager {
    /// 构造新的剪贴板通道管理器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 检查是否为重复文本，若有新内容则更新内部记录。
    pub fn should_sync_remote_text(&mut self, text: &str) -> bool {
        if self.last_synced_text.as_deref() == Some(text) {
            return false;
        }
        self.last_synced_text = Some(text.to_string());
        true
    }

    /// 记录本地更新并准备发往对端。
    pub fn update_local_text(&mut self, text: String) -> Option<Vec<u8>> {
        if self.last_synced_text.as_ref() == Some(&text) {
            return None;
        }
        self.last_synced_text = Some(text.clone());
        Some(text::encode_text_payload(&text))
    }
}
