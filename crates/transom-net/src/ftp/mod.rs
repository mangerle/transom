//! FTP/FTPS 客户端封装。
//!
//! 设计原理（Why）：
//! - 实现初衷：SFTP 面板的数据源抽象已经只依赖 `SftpOperation` 契约与
//!   `ConnectionId`，引入 FTP 只需在 `transom-net` 提供一个与 `SftpClient`
//!   方法集对齐的具体客户端，面板与协议契约层完全不用动；
//! - 核心优势：控制连接断线自愈（FTP 服务器普遍对空闲控制连接设限，几十秒
//!   无操作即断开）——操作失败且错误属于连接级故障时把流整体废弃，下一次
//!   操作自动重连重登录，调用方无感；传输类操作不在此列，它们由执行器为
//!   每个任务单独开连接（见 [`FtpClient::open_transfer_client`]），取消即
//!   整条丢弃，不存在"半截传输污染控制通道"的问题；
//! - 代价与局限：FTP 是弱语义协议——没有原子 rename 语义、没有统一权限
//!   模型、目录列表格式依赖服务端实现，能力面天然小于 SFTP；软链接目标、
//!   远端归档、远端检索与提权写入没有协议级等价物，由上层按"不支持"拒绝。
//!
//! 密码经钥匙串取出后存放在 [`zeroize::Zeroizing`] 中，丢弃时擦除内存。

mod directory;
mod listing;
mod stream;
mod text;
mod transfer;

use std::sync::Arc;

use tokio::sync::Mutex;
use transom_core::{AppError, ErrorCode, FtpConnection};
use zeroize::Zeroizing;

pub(crate) use stream::FtpStream;

/// 单个 FTP 会话的客户端；克隆共享同一条控制连接。
///
/// 控制连接包在 tokio 互斥锁里：FTP 控制通道一次只能跑一条命令，锁把并发
/// 调用串行化，与协议约束一致；锁内不含长时间数据传输（传输走独立连接），
/// 交互操作不会被吞吐任务长期占住锁。
#[derive(Clone)]
pub struct FtpClient {
    session: Arc<Mutex<FtpSession>>,
    config: FtpConnection,
    password: Zeroizing<String>,
}

/// 控制连接的可恢复状态：`None` 表示需要（重新）建立连接。
struct FtpSession {
    stream: Option<FtpStream>,
}

impl FtpClient {
    /// 建立并登录一个 FTP 会话。
    ///
    /// # Errors
    /// 拨号超时、TLS 握手失败或认证被拒绝时分别返回
    /// `ErrorCode::Timeout`/`ErrorCode::ConnectionFailed`/`ErrorCode::AuthFailed`。
    pub async fn open(config: &FtpConnection, password: Option<&str>) -> Result<Self, AppError> {
        let client = Self {
            session: Arc::new(Mutex::new(FtpSession { stream: None })),
            config: config.clone(),
            password: Zeroizing::new(password.unwrap_or_default().to_string()),
        };
        {
            let mut session = client.session.lock().await;
            client.reconnect_inner(&mut session).await?;
        }
        Ok(client)
    }

    /// 为传输任务建立一条独立的 FTP 连接。
    ///
    /// 设计原理（Why）：与 SFTP 侧预留第二传输通道同理——FTP 的控制通道一次
    /// 只能服务一个数据连接，且任务取消（tokio abort）要求"整条连接随之废弃"
    /// 才有干净的取消语义。每个传输任务独立拨号登录，取消时连接整体关闭，
    /// 交互连接不受任何影响；代价是每个任务多一次握手，对以吞吐为主的传输
    /// 场景可忽略。
    ///
    /// # Errors
    /// 同 [`FtpClient::open`]。
    pub async fn open_transfer_client(&self) -> Result<Self, AppError> {
        FtpClient::open(&self.config, Some(&self.password)).await
    }

    /// 尽力关闭控制连接；用于会话停机路径。
    pub async fn close(&self) {
        let mut session = self.session.lock().await;
        if let Some(stream) = session.stream.as_mut() {
            stream.quit().await;
        }
        session.stream = None;
    }

    /// 在健康连接上执行一次控制操作；连接级故障废弃当前流，等待下次重建。
    ///
    /// 生命周期说明：操作用 [`AsyncFnOnce`] 承接——异步闭包的 future 可以
    /// 借用其入参（`&mut FtpStream`），不必再做类型擦除装箱。
    async fn with_stream<T>(
        &self,
        operation: impl AsyncFnOnce(&mut FtpStream) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut session = self.session.lock().await;
        if session.stream.is_none() {
            self.reconnect_inner(&mut session).await?;
        }
        let Some(stream) = session.stream.as_mut() else {
            return Err(AppError::with_detail(
                ErrorCode::ConnectionFailed,
                "FTP 连接未能建立",
            ));
        };
        match operation(stream).await {
            Ok(value) => Ok(value),
            Err(error) => {
                // 连接级故障（拨号断开、响应错乱、控制超时）会让行协议错位，
                // 整条废弃最安全；语义性拒绝（550/502 等）是干净往返，连接可继续用。
                if matches!(error.code, ErrorCode::ConnectionFailed | ErrorCode::Timeout) {
                    session.stream = None;
                }
                Err(error)
            }
        }
    }

    /// 重建控制连接并完成登录初始化。
    async fn reconnect_inner(&self, session: &mut FtpSession) -> Result<(), AppError> {
        let mut stream = FtpStream::connect(&self.config).await?;
        stream
            .login_and_setup(&self.config.username, &self.password, self.config.passive)
            .await?;
        session.stream = Some(stream);
        Ok(())
    }

    /// 传输执行器专用：直接锁定会话状态，传输体在锁内顺序执行。
    ///
    /// 仅限独立传输连接使用；交互操作必须走 [`FtpClient::with_stream`]，
    /// 否则长传输会占住共享锁阻塞交互。
    async fn lock_session(&self) -> tokio::sync::MutexGuard<'_, FtpSession> {
        self.session.lock().await
    }

    /// 会话未连接时先建立连接；传输执行器在拿到会话锁后调用。
    async fn ensure_connected(&self, session: &mut FtpSession) -> Result<(), AppError> {
        if session.stream.is_none() {
            self.reconnect_inner(session).await?;
        }
        Ok(())
    }
}

/// 传输数据流的底层 IO 故障统一转为领域错误。
pub(super) fn transfer_io_error(error: std::io::Error) -> AppError {
    AppError::with_detail(ErrorCode::FtpFailed, format!("FTP 数据传输失败: {error}"))
}
