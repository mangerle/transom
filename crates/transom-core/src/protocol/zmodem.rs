//! ZMODEM 文件传输的事件与设置契约。
//!
//! 类型只承载界面需要的信息（方向、文件名、进度、结局），协议细节全部
//! 留在传输层；控制器把传输层的 [`ZmodemEvent`] 原样转发为
//! `AppEvent::Zmodem`，界面据此驱动状态栏与文件选择。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 传输方向：上传（对端 rz 接收）或下载（对端 sz 发送）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmodemDirection {
    Upload,
    Download,
}

/// 一次 ZMODEM 会话的结局。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZmodemFinished {
    /// 正常完成，`files` 为成功传输的文件数。
    Completed { files: usize },
    /// 用户主动取消。
    Cancelled,
    /// 协议错误中止，携带面向用户的中文描述。
    Failed(String),
}

/// ZMODEM 会话过程中的事件流。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZmodemEvent {
    /// 检测到对端发起传输，会话开始。
    Started { direction: ZmodemDirection },
    /// 传输进度；`total` 为对端声明的大小（未知为 `None`）。
    Progress {
        direction: ZmodemDirection,
        name: String,
        transferred: u64,
        total: Option<u64>,
    },
    /// 请求用户选择待上传文件（对端 rz 已在等待）。
    AwaitUpload,
    /// 会话结束。
    Finished(ZmodemFinished),
}

/// ZMODEM 行为设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZmodemSettings {
    /// 下载落盘目录；`None` 时由界面侧解析系统下载目录兜底。
    #[serde(default)]
    pub download_dir: Option<PathBuf>,
    /// 检测到对端 sz 发起下载时是否自动接收；关闭则立即取消传输并提示，
    /// 避免在不受信任的链路上被动的字节落盘。
    #[serde(default = "default_auto_receive")]
    pub auto_receive: bool,
}

impl Default for ZmodemSettings {
    fn default() -> Self {
        Self {
            download_dir: None,
            auto_receive: true,
        }
    }
}

impl ZmodemSettings {
    /// 解析实际生效的下载目录：显式配置优先，否则按系统目录约定兜底。
    pub fn resolved_download_dir(&self) -> PathBuf {
        self.download_dir
            .clone()
            .unwrap_or_else(transom_platform::paths::default_download_dir)
    }
}

fn default_auto_receive() -> bool {
    true
}
