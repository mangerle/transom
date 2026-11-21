//! FTP 控制流的多态封装与错误归类。
//!
//! 设计原理（Why）：suppaftp 的明文与 TLS 流是两个不同的具体类型
//! （`ImplAsyncFtpStream<AsyncNoTlsStream>` 与 `ImplAsyncFtpStream<AsyncRustlsStream>`），
//! 泛型参数会顺着 `TransferStream<T>` 泄漏到所有操作签名上。用枚举在边界处
//! 收敛为单一类型，配合宏代理转发方法，调用方完全感知不到两种后端的差异；
//! 代价是每个方法都要写两份转发分支（宏代付），类型安全不受影响。
//!
//! 错误处理统一走 [`control_call`]：先套控制面超时，再把 suppaftp 原始错误
//! 归类为领域错误。不能复用 `timeout::with_timeout`，因为它把原始错误包进
//! 中文描述字符串后就丢弃了类型，无法再按服务端状态码区分认证失败、不支持
//! 与一般失败。

use std::future::Future;
use std::sync::Arc;

use rustls_platform_verifier::BuilderVerifierExt;
use suppaftp::FtpError;
use suppaftp::Status;
use suppaftp::tokio::{
    AsyncFtpStream, AsyncRustlsConnector, AsyncRustlsFtpStream, ImplAsyncFtpStream, TokioTlsStream,
};
use suppaftp::types::{FileType, Mode, Response};
use tokio_rustls::TlsConnector;
use transom_core::{AppError, ErrorCode};

/// 控制命令的单次往返上限：覆盖慢链路上的 AUTH/PASV/LIST 往返，不约束数据
/// 传输（大文件不能设整体时限，见 `transfer.rs`）。
pub(crate) const CONTROL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// 拨号（含 DNS 与 TCP 握手）与 TLS 升级的上限，与 SSH 会话握手口径一致。
pub(crate) const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// 单次控制面调用：控制面超时 + 按 FTP 语义归类领域错误。
///
/// 返回装箱 future：明文与 TLS 两个后端分支产生的 future 具体类型不同，
/// 装箱后宏代理的 match 两臂才能收敛为同一种返回类型；控制命令本身是
/// 网络往返，装箱开销可忽略。
fn control_call<'a, T, F>(
    action: &'a str,
    future: F,
) -> std::pin::Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>
where
    T: Send + 'a,
    F: Future<Output = suppaftp::FtpResult<T>> + Send + 'a,
{
    Box::pin(async move {
        match tokio::time::timeout(CONTROL_TIMEOUT, future).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(classify_ftp_error(&error, action).0),
            Err(_) => Err(AppError::with_detail(
                ErrorCode::Timeout,
                format!("{action}超时，等待上限: {} 秒", CONTROL_TIMEOUT.as_secs()),
            )),
        }
    })
}

/// 拨号与 TLS 握手的统一超时包装；产物固定为 `ImplAsyncFtpStream<T>`。
async fn dial_control<T, F>(action: &str, future: F) -> Result<ImplAsyncFtpStream<T>, AppError>
where
    T: TokioTlsStream + Send,
    F: Future<Output = suppaftp::FtpResult<ImplAsyncFtpStream<T>>>,
{
    match tokio::time::timeout(CONNECT_TIMEOUT, future).await {
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(error)) => Err(classify_ftp_error(&error, action).0),
        Err(_) => Err(AppError::with_detail(
            ErrorCode::Timeout,
            format!("{action}超时，等待上限: {} 秒", CONNECT_TIMEOUT.as_secs()),
        )),
    }
}

/// 已建立的 FTP 控制通道：明文或显式 TLS 两种后端的统一封装。
pub(crate) enum FtpStream {
    Plain(AsyncFtpStream),
    Tls(AsyncRustlsFtpStream),
}

/// 代理转发一条控制命令：两个后端分支各自构造 future 并就地 await，
/// 让 match 两臂统一收敛到 `Result<T, AppError>`。
macro_rules! delegate_control {
    ($self:expr, $action:expr, $stream:ident => $($body:tt)*) => {
        match $self {
            FtpStream::Plain($stream) => control_call($action, $($body)*).await,
            FtpStream::Tls($stream) => control_call($action, $($body)*).await,
        }
    };
}

impl FtpStream {
    /// 建立控制连接；显式 TLS 模式在认证前完成 `AUTH TLS` 升级。
    pub(crate) async fn connect(config: &transom_core::FtpConnection) -> Result<Self, AppError> {
        let address = format!("{}:{}", config.host, config.port);
        match config.security {
            transom_core::FtpSecurity::Plain => {
                let stream = dial_control(
                    &format!("FTP 连接建立，主机: {}", config.host),
                    AsyncFtpStream::connect(&address),
                )
                .await?;
                Ok(FtpStream::Plain(stream))
            }
            transom_core::FtpSecurity::ExplicitTls => {
                // 显式 TLS：连接必须以 TLS 流类型建立，再由 into_secure 原地
                // 升级；AUTH TLS 之后 USER/PASS 都发生在加密通道之上。
                let plain = dial_control(
                    &format!("FTP 连接建立，主机: {}", config.host),
                    AsyncRustlsFtpStream::connect(&address),
                )
                .await?;
                let secure = dial_control(
                    &format!("FTPS 加密握手，主机: {}", config.host),
                    plain.into_secure(AsyncRustlsConnector::from(ftps_connector()?), &config.host),
                )
                .await?;
                Ok(FtpStream::Tls(secure))
            }
        }
    }

    /// 登录并完成会话初始化：显式声明二进制传输模式（服务端默认 ASCII，
    /// 不切换会让二进制文件在传输中被 CRLF 转换损坏）、按配置选择被动模式，
    /// 最后返回服务端上报的登录主目录。
    pub(crate) async fn login_and_setup(
        &mut self,
        username: &str,
        password: &str,
        passive: bool,
    ) -> Result<String, AppError> {
        delegate_control!(self, "FTP 登录", stream => stream.login(username, password))?;
        delegate_control!(
            self,
            "FTP 传输模式切换",
            stream => stream.transfer_type(FileType::Binary)
        )?;
        if !passive {
            match self {
                FtpStream::Plain(stream) => stream.set_mode(Mode::Active),
                FtpStream::Tls(stream) => stream.set_mode(Mode::Active),
            }
        }
        delegate_control!(self, "FTP 主目录查询", stream => stream.pwd())
    }

    /// 尽力关闭会话：QUIT 失败或挂起都直接放弃，底层 TCP 由 Drop 兜底释放。
    pub(crate) async fn quit(&mut self) {
        let _ = delegate_control!(self, "FTP 会话关闭", stream => stream.quit());
    }

    /// 目录探测：CWD 成功即证明路径存在且为目录；同时改变服务端当前目录，
    /// 客户端全部使用绝对路径，不受该副作用影响。
    pub(crate) async fn cwd(&mut self, path: &str) -> Result<(), AppError> {
        delegate_control!(self, "FTP 目录切换", stream => stream.cwd(path))
    }

    pub(crate) async fn mkdir(&mut self, path: &str) -> Result<(), AppError> {
        delegate_control!(self, "FTP 创建目录", stream => stream.mkdir(path))
    }

    pub(crate) async fn rename(&mut self, from: &str, to: &str) -> Result<(), AppError> {
        delegate_control!(self, "FTP 重命名", stream => stream.rename(from, to))
    }

    pub(crate) async fn rm(&mut self, path: &str) -> Result<(), AppError> {
        delegate_control!(self, "FTP 删除文件", stream => stream.rm(path))
    }

    pub(crate) async fn rmdir(&mut self, path: &str) -> Result<(), AppError> {
        delegate_control!(self, "FTP 删除目录", stream => stream.rmdir(path))
    }

    pub(crate) async fn size(&mut self, path: &str) -> Result<u64, AppError> {
        delegate_control!(
            self,
            "FTP 文件大小查询",
            stream => async { stream.size(path).await.map(|value| value as u64) }
        )
    }

    pub(crate) async fn mlsd(&mut self, path: Option<&str>) -> Result<Vec<String>, AppError> {
        delegate_control!(self, "FTP 机器可读目录列表", stream => stream.mlsd(path))
    }

    pub(crate) async fn list(&mut self, path: Option<&str>) -> Result<Vec<String>, AppError> {
        delegate_control!(self, "FTP 目录列表", stream => stream.list(path))
    }

    pub(crate) async fn mlst(&mut self, path: &str) -> Result<String, AppError> {
        delegate_control!(self, "FTP 单条目属性查询", stream => stream.mlst(Some(path)))
    }

    pub(crate) async fn site(&mut self, command: &str) -> Result<(), AppError> {
        delegate_control!(
            self,
            "FTP SITE 命令",
            stream => async { stream.site(command).await.map(|_| ()) }
        )
    }

    /// 查询远端文件修改时间（MDTM），按服务端惯例视为 UTC 换算成 Unix 秒。
    pub(crate) async fn mdtm(&mut self, path: &str) -> Result<Option<u32>, AppError> {
        delegate_control!(self, "FTP 修改时间查询", stream => async {
            let datetime = stream.mdtm(path).await?;
            Ok(Some(crate::ftp::listing::civil_to_unix_seconds(
                datetime.year(),
                datetime.month(),
                datetime.day(),
                datetime.hour(),
                datetime.minute(),
                datetime.second(),
            )))
        })
    }

    /// 开启下载流：先按偏移发送 REST，再发起 RETR 返回数据流。
    pub(crate) async fn download_stream(
        &mut self,
        path: &str,
        offset: u64,
    ) -> Result<FtpTransferStream, AppError> {
        let offset = resume_offset(offset, path)?;
        match self {
            FtpStream::Plain(stream) => {
                let transfer = control_call("FTP 打开下载流", async {
                    stream.resume_transfer(offset).await?;
                    stream.retr_as_stream(path).await
                })
                .await?;
                Ok(FtpTransferStream::Plain(transfer))
            }
            FtpStream::Tls(stream) => {
                let transfer = control_call("FTP 打开下载流", async {
                    stream.resume_transfer(offset).await?;
                    stream.retr_as_stream(path).await
                })
                .await?;
                Ok(FtpTransferStream::Tls(transfer))
            }
        }
    }

    /// 开启上传流：先按偏移发送 REST，再发起 STOR 返回可写数据流。
    ///
    /// REST + STOR 按 RFC 3659 从指定偏移覆写；个别服务器不支持 REST 写侧时
    /// 会拒绝该次传输，错误按语义返回，由调用方退回整体覆盖。
    pub(crate) async fn upload_stream(
        &mut self,
        path: &str,
        offset: u64,
    ) -> Result<FtpTransferStream, AppError> {
        let offset = resume_offset(offset, path)?;
        match self {
            FtpStream::Plain(stream) => {
                let transfer = control_call("FTP 打开上传流", async {
                    stream.resume_transfer(offset).await?;
                    stream.put_with_stream(path).await
                })
                .await?;
                Ok(FtpTransferStream::Plain(transfer))
            }
            FtpStream::Tls(stream) => {
                let transfer = control_call("FTP 打开上传流", async {
                    stream.resume_transfer(offset).await?;
                    stream.put_with_stream(path).await
                })
                .await?;
                Ok(FtpTransferStream::Tls(transfer))
            }
        }
    }
}

/// 校验并收敛续传偏移：FTP 协议用十进制字符串携带偏移，平台指针宽度是上限。
fn resume_offset(offset: u64, path: &str) -> Result<usize, AppError> {
    usize::try_from(offset).map_err(|_| {
        AppError::with_detail(
            ErrorCode::FtpFailed,
            format!("FTP 续传偏移超出平台上限，路径: {path}"),
        )
    })
}

/// 进行中的 FTP 数据传输流：读写与收尾的统一封装。
pub(crate) enum FtpTransferStream {
    Plain(suppaftp::tokio::TransferStream<suppaftp::tokio::AsyncNoTlsStream>),
    Tls(suppaftp::tokio::TransferStream<suppaftp::tokio::AsyncRustlsStream>),
}

impl FtpTransferStream {
    /// 结束传输并等待服务端确认；消费传输流本身（suppaftp 的 finish 以
    /// 值接收），必须在该连接的下一条命令之前调用，否则控制通道会残留
    /// 一条未消费的应答。
    pub(crate) async fn finish(self) -> Result<(), AppError> {
        match self {
            FtpTransferStream::Plain(stream) => control_call("FTP 传输收尾", stream.finish()).await,
            FtpTransferStream::Tls(stream) => control_call("FTP 传输收尾", stream.finish()).await,
        }
    }
}

impl tokio::io::AsyncRead for FtpTransferStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            FtpTransferStream::Plain(stream) => std::pin::pin!(stream).poll_read(cx, buf),
            FtpTransferStream::Tls(stream) => std::pin::pin!(stream).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for FtpTransferStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            FtpTransferStream::Plain(stream) => std::pin::pin!(stream).poll_write(cx, buf),
            FtpTransferStream::Tls(stream) => std::pin::pin!(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            FtpTransferStream::Plain(stream) => std::pin::pin!(stream).poll_flush(cx),
            FtpTransferStream::Tls(stream) => std::pin::pin!(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            FtpTransferStream::Plain(stream) => std::pin::pin!(stream).poll_shutdown(cx),
            FtpTransferStream::Tls(stream) => std::pin::pin!(stream).poll_shutdown(cx),
        }
    }
}

/// 构造 FTPS 的 TLS 连接器：证书校验走操作系统平台验证器，与 reqwest 的
/// HTTPS 行为一致；显式声明 aws-lc-rs 加密提供者，不依赖进程级隐式默认值。
fn ftps_connector() -> Result<TlsConnector, AppError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::Configuration,
                format!("FTPS 协议版本初始化失败: {error}"),
            )
        })?
        .with_platform_verifier()
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::Configuration,
                format!("FTPS 平台证书校验器初始化失败: {error}"),
            )
        })?
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

/// 把 suppaftp 错误归类为领域错误，并标记该错误是否已污染控制通道。
///
/// 设计原理（Why）：FTP 控制连接是带状态的行协议，连接级故障（IO 断开、
/// 响应错乱）会让后续命令读到错位的应答，必须整条废弃重连；而服务端的
/// 语义性拒绝（550 无此文件、502 不支持）是干净的往返，连接仍可继续使用。
/// 两类错误在归类的同一步里区分开，调用方按错误码即可决定是否重建连接。
pub(crate) fn classify_ftp_error(error: &FtpError, action: &str) -> (AppError, bool) {
    let poisoned = match error {
        FtpError::ConnectionError(_) | FtpError::BadResponse | FtpError::SecureError(_) => true,
        FtpError::UnexpectedResponse(Response { status, .. }) => matches!(
            status,
            Status::NotAvailable
                | Status::CannotOpenDataConnection
                | Status::TransferAborted
                | Status::ActionAborted
        ),
        _ => false,
    };
    let code = match error {
        FtpError::InvalidAddress(_) => ErrorCode::Configuration,
        FtpError::UnexpectedResponse(Response { status, .. }) => match status {
            Status::NotLoggedIn
            | Status::InvalidCredentials
            | Status::NeedPassword
            | Status::StoringNeedAccount => ErrorCode::AuthFailed,
            Status::BadCommand | Status::NotImplemented | Status::NotImplementedParameter => {
                ErrorCode::NotSupported
            }
            // 4xx 暂态与 5xx 语义拒绝分开：前者伴随连接不可用，统一归
            // ConnectionFailed，让调用方按错误码判定"必须重建连接"。
            _ if poisoned => ErrorCode::ConnectionFailed,
            _ => ErrorCode::FtpFailed,
        },
        FtpError::ConnectionError(io) if io.kind() == std::io::ErrorKind::TimedOut => {
            ErrorCode::Timeout
        }
        // 连接级故障（IO 断开、响应错乱、TLS 失败）统一归 ConnectionFailed：
        // 这是"控制连接已污染、必须整条废弃重连"的语义信号，调用方按错误码
        // 即可完成自愈分流，不再依赖额外的布尔标志穿透各层。
        _ if poisoned => ErrorCode::ConnectionFailed,
        _ => ErrorCode::FtpFailed,
    };
    let detail = match error {
        FtpError::UnexpectedResponse(response) => {
            let body = String::from_utf8_lossy(&response.body);
            format!(
                "{action}被服务端拒绝（{}），响应: {body}",
                response.status as u16
            )
        }
        other => format!("{action}失败，原因: {other}"),
    };
    (AppError::with_detail(code, detail), poisoned)
}
