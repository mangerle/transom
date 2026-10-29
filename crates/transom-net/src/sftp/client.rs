//! SFTP 会话客户端封装与通用错误转换。

use crate::client::SshRemote;
use crate::timeout::{SFTP_HANDSHAKE_TIMEOUT, with_timeout};
use russh_sftp::client::{Config, SftpSession};
use std::sync::Arc;
use transom_core::{AppError, ErrorCode};

/// 顺序传输时单次读写缓冲的字节数。
///
/// 取 256 KiB 与 russh-sftp 单次 READ 的数据上限（约 255 KiB）对齐，每次
/// 轮询就能消费完一个预取块；若沿用 64 KiB，同一个块会被拆成四次异步
/// 轮询，单句柄在高吞吐链路上会因此损失约 30% 吞吐。
pub(crate) const TRANSFER_CHUNK: usize = 256 * 1024;

/// 单次目录列举的条目上限。
///
/// 设计原理（Why）：列表整体读入内存并整体搬进界面，而远端是不可信输入
/// 源——异常或恶意服务端可以让目录列出百万条目，把客户端内存吃光，UI 的
/// 虚拟列表只能省渲染、省不了内存。20 万条足以覆盖真实的运维目录（含
/// 构建产物目录），超出即中止并给出可操作的提示（过滤或进入子目录），
/// 而不是把半份列表交给界面。
pub(crate) const MAX_LIST_ENTRIES: usize = 200_000;

/// 列表条目是否已达上限（抽成纯函数供单测钉住边界）。
pub(crate) fn list_limit_reached(collected: usize) -> bool {
    collected >= MAX_LIST_ENTRIES
}

/// 跨并发任务共享的进度通知：任一任务推进都汇报当前全局累计字节。
pub(crate) fn notify_shared_progress<F>(
    progress: &Arc<std::sync::Mutex<F>>,
    transferred: u64,
    total: Option<u64>,
) where
    F: FnMut(u64, Option<u64>) + Send,
{
    // 回调内部只做同步 try_send，持锁不跨 await；锁中毒（回调 panic）
    // 时放弃该次通知即可，传输本身不受影响。
    if let Ok(mut callback) = progress.lock() {
        callback(transferred, total);
    }
}

/// 构造 SFTP 会话配置。
///
/// 设计原理（Why）：
/// - 吞吐取决于「在途请求数 × 单请求字节数」与 SSH 通道窗口的匹配。OpenSSH
///   单次 READ 上限约 255 KiB，64 个在途请求约 16 MiB，正好填满 `client.rs`
///   中提高后的 16 MiB 通道窗口；库默认的 16 深只能吃到约 4 MiB，单句柄
///   传输会把窗口闲置四分之三。
/// - 写请求按 `max_write_packet_len`（默认 32 KiB）分包，64 个在途约 2 MiB，
///   对齐 OpenSSH 服务端通告的 2 MiB 接收窗口。
/// - 深预取会拉长队尾请求在慢链路上的等待时间，默认 10 秒超时容易误杀正常
///   传输，因此放宽到 30 秒。
/// - 代价与局限：在途请求与待发数据随深度上升；极慢链路叠加目录级并发时，
///   队尾等待仍可能触及超时，这是吞吐与容错之间的固有权衡。
fn sftp_config() -> Config {
    Config {
        max_concurrent_reads: 64,
        max_concurrent_writes: 64,
        request_timeout_secs: 30,
        ..Config::default()
    }
}

#[derive(Clone)]
pub struct SftpClient {
    pub(crate) session: Arc<SftpSession>,
    pub(crate) home: String,
    pub(crate) ssh_remote: Option<SshRemote>,
}

impl SftpClient {
    pub async fn open(remote: &SshRemote) -> Result<Self, AppError> {
        let channel = remote.open_session().await?;
        with_timeout(
            SFTP_HANDSHAKE_TIMEOUT,
            ErrorCode::ChannelFailed,
            "启动 SFTP 子系统",
            channel.request_subsystem(true, "sftp"),
        )
        .await?;
        let session = with_timeout(
            SFTP_HANDSHAKE_TIMEOUT,
            ErrorCode::SftpFailed,
            "SFTP 协议初始化",
            SftpSession::new_with_config(channel.into_stream(), sftp_config()),
        )
        .await?;
        // 主目录只用于初始定位；查询失败或超时都退化为根目录，不能阻断连接。
        let home = match with_timeout(
            SFTP_HANDSHAKE_TIMEOUT,
            ErrorCode::SftpFailed,
            "查询 SFTP 主目录",
            session.canonicalize("."),
        )
        .await
        {
            Ok(home) => home,
            Err(error) => {
                log::debug!("查询 SFTP 主目录失败，按根目录继续，原因: {error}");
                "/".to_string()
            }
        };
        Ok(Self {
            session: Arc::new(session),
            home,
            ssh_remote: Some(remote.clone()),
        })
    }

    pub fn home(&self) -> &str {
        &self.home
    }

    pub async fn close(&self) {
        let _ = self.session.close().await;
    }
}

/// 把 SFTP 协议错误转成本模块的领域错误。
///
/// 权限拒绝必须单独成码：它是唯一有后续动作的失败（界面要据此提供提权入口），
/// 而服务器回的文本随实现与 locale 变化——中文系统不会回 "Permission denied"，
/// 靠字符串匹配等于在部分机器上这个功能根本不存在。状态码是协议规定的，可靠。
pub(crate) fn sftp_error(error: russh_sftp::client::error::Error) -> AppError {
    if let russh_sftp::client::error::Error::Status(status) = &error
        && status.status_code == russh_sftp::protocol::StatusCode::PermissionDenied
    {
        return AppError::with_detail(
            ErrorCode::PermissionDenied,
            format!(
                "远端拒绝授权本次操作，路径状态码: {:?}，说明: {}",
                status.status_code, status.error_message
            ),
        );
    }
    AppError::with_detail(ErrorCode::SftpFailed, error.to_string())
}

pub(crate) fn sftp_io_error(error: std::io::Error) -> AppError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        return AppError::with_detail(
            ErrorCode::PermissionDenied,
            format!("远端拒绝授权读写操作，原因: {error}"),
        );
    }
    AppError::with_detail(ErrorCode::SftpFailed, error.to_string())
}
