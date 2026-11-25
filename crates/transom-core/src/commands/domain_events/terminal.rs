//! 终端会话与连接事件。

use crate::error::AppError;
use crate::ids::{ConnectionId, TabId};
use crate::prompt::PromptInfo;
use crate::transport::TerminalIo;

/// 终端会话与连接相关的事件。
pub enum TerminalAppEvent {
    TerminalReady {
        tab_id: TabId,
        connection_id: ConnectionId,
        io: TerminalIo,
    },
    /// FTP 会话登录成功：不产生终端流，宿主据此把标签置为已连接并打开
    /// 文件面板（FTP 的主交互面就是文件面板，没有终端可渲染）。
    FtpReady {
        tab_id: TabId,
        connection_id: ConnectionId,
    },
    ConnectionFailed {
        tab_id: TabId,
        connection_id: ConnectionId,
        error: AppError,
    },
    Disconnected {
        tab_id: TabId,
        connection_id: ConnectionId,
    },
    AuthPrompt {
        tab_id: TabId,
        connection_id: ConnectionId,
        name: String,
        instructions: String,
        prompts: Vec<PromptInfo>,
        responder: tokio::sync::mpsc::Sender<Vec<String>>,
    },
    /// 首次连接遇到未知主机密钥，请求用户确认是否信任该指纹（TOFU）。
    HostKeyConfirm {
        tab_id: TabId,
        connection_id: ConnectionId,
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
        responder: tokio::sync::mpsc::Sender<bool>,
    },
}
