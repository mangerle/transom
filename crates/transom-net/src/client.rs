use crate::auth::AuthPrompter;
use crate::bridge::{AsyncChannelStream, ChannelReader, ChannelWriter};
use crate::error::NetError;
use crate::traffic::CountingStream;
use russh::client::{self, Handle, Msg};
use russh::keys::agent::client::AgentClient;
use russh::keys::{
    Algorithm, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate, load_secret_key,
};
use russh::{Channel, ChannelMsg, Disconnect, Preferred};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use transom_core::{
    AppError, ConnectionSecrets, ErrorCode, HostKeyInfo, HostKeyStatus, KnownHostsStore,
    OutboundProxy, PromptInfo, PromptKind, SessionOptions, SshConnection, SshHop, TerminalControl,
    TerminalControlHandle, TerminalIo, TerminalSize, TrafficCounter,
};
use zeroize::Zeroizing;

/// 传输流抽象：直连 TCP、跳板机通道与后续的出站代理都收敛到同一类型。
///
/// 为什么需要它：`russh::client::connect_stream` 只要求 `AsyncRead + AsyncWrite +
/// Send + Unpin + 'static`，用 trait object 把"这一跳的字节从哪来"从握手逻辑里
/// 解耦，裸 TCP 与隧道通道可以走同一段握手/认证代码。
trait AsyncStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> AsyncStream for T {}

#[derive(Debug)]
pub struct ClientError(pub AppError);

/// 远端转发建立到目标主机的 TCP 连接的最长等待时间。
///
/// 转发目标通常是内网邻近服务，正常连接远快于该上限；取 30 秒是为
/// 跨机房高延迟链路留余量，同时兜住"对端不可达导致任务永久泄漏"。
const FORWARD_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// 单次远端命令允许捕获的输出总量上限（stdout 与 stderr 合计）。
///
/// 设计原理（Why）：`exec_command_capture` 的调用方全是小输出的只读命令
/// （/proc 采样、ps、df、authorized_keys 追加），但远端是不可信输入源——
/// 被攻陷或配置异常的服务端可以让命令持续回放大数据，无上限的累积缓冲
/// 会把客户端内存吃光（OOM）。4 MiB 足以容纳数万行进程表，超出即按
/// 协议异常中止读取，不把异常输出交给上层解析。
const MAX_EXEC_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// 单次远端命令的兜底超时。
///
/// 调用方按业务需要叠加更短的超时（如统计采样 10 秒，取更小者生效）；
/// 这里只兜住"对端静默挂起导致通道永不返回"的协程泄漏。
const EXEC_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// 错误信息中命令文本的最大字符数（截断点回退到字符边界）。
const COMMAND_EXCERPT_BYTES: usize = 120;

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for ClientError {}

impl From<russh::Error> for ClientError {
    fn from(error: russh::Error) -> Self {
        Self(AppError::with_detail(
            ErrorCode::Protocol,
            error.to_string(),
        ))
    }
}

impl From<AppError> for ClientError {
    fn from(error: AppError) -> Self {
        Self(error)
    }
}

struct ClientHandler {
    host: String,
    port: u16,
    known_hosts: Arc<dyn KnownHostsStore>,
    remote_forwards: Arc<Mutex<HashMap<u16, (String, u16)>>>,
    /// TOFU 指纹确认走认证提示通道，与凭据索取共用同一条 UI 往返链路。
    prompter: Arc<dyn AuthPrompter>,
}

impl client::Handler for ClientHandler {
    type Error = ClientError;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let public_key = server_key.public_key();
        let fingerprint = public_key.fingerprint(HashAlg::Sha256).to_string();
        let key_type = format!("{:?}", public_key.algorithm());
        let info = HostKeyInfo::new(
            key_type.clone(),
            fingerprint.clone(),
            public_key.to_openssh().ok(),
        );

        match self.known_hosts.check(&self.host, self.port, &info)? {
            HostKeyStatus::Match => Ok(true),
            HostKeyStatus::Unknown => {
                // TOFU：首次连接必须由用户确认指纹后才记录信任，杜绝首连
                // 被 MITM 无感通过；无界面提示器按自身策略自动放行或失败。
                let accepted = self
                    .prompter
                    .confirm_host_key(&self.host, self.port, &key_type, &fingerprint)
                    .await
                    .map_err(ClientError)?;
                if !accepted {
                    return Err(ClientError(AppError::with_detail(
                        ErrorCode::HostKeyUnknown,
                        format!(
                            "用户拒绝了 {}:{} 的主机密钥指纹 {fingerprint}",
                            self.host, self.port
                        ),
                    )));
                }
                self.known_hosts.record(&self.host, self.port, &info)?;
                log::info!(
                    "用户确认后记录新主机密钥，主机: {}，端口: {}，指纹: {fingerprint}",
                    self.host,
                    self.port
                );
                Ok(true)
            }
            HostKeyStatus::Mismatch {
                previous_fingerprint,
            } => Err(ClientError(
                NetError::HostKey {
                    host: self.host.clone(),
                    port: self.port,
                    reason: format!(
                        "主机密钥与已记录指纹不一致，原指纹: {previous_fingerprint}，当前指纹: {fingerprint}"
                    ),
                }
                .into(),
            )),
        }
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let target = self
            .remote_forwards
            .lock()
            .ok()
            .and_then(|map| map.get(&(connected_port as u16)).cloned());
        reply.accept().await;
        let Some((host, port)) = target else {
            return Ok(());
        };
        let mut stream = channel.into_stream();
        tokio::spawn(async move {
            // 转发目标是任意外部地址：对端不可达或防火墙静默丢包时 connect 会
            // 无限挂起，任务与通道随之泄漏。只给连接阶段设限，建立后的双向
            // 转发不设限——长空闲本身就是隧道的正常形态。
            let connect = TcpStream::connect((host.as_str(), port));
            match tokio::time::timeout(FORWARD_CONNECT_TIMEOUT, connect).await {
                Ok(Ok(mut tcp)) => {
                    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut stream).await;
                }
                _ => {
                    let _ = stream.shutdown().await;
                }
            }
        });
        Ok(())
    }
}

/// 远端命令执行结果；`exit_status` 缺失表示服务端未回传退出码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub exit_status: Option<u32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone)]
pub struct SshRemote {
    /// russh 的 Handle 方法全部取 &self 且内部自带并发安全（消息发送端 +
    /// 回复分发器），包一层全局互斥只会把所有通道打开、exec、转发操作
    /// 无谓串行化，因此直接共享 Arc<Handle>。
    inner: Arc<Handle<ClientHandler>>,
    remote_forwards: Arc<Mutex<HashMap<u16, (String, u16)>>>,
    /// 跳板机链路上的会话句柄（最外层到最内层）。
    ///
    /// 这些句柄必须在主连接存续期间保活：目标节点的传输流是它们内部的
    /// `direct-tcpip` 通道，提前释放会让整条链路断开。断开主连接时按
    /// 由内向外的顺序一并关闭，避免残留 SSH 会话。
    jump_handles: Vec<Arc<Handle<ClientHandler>>>,
}

impl SshRemote {
    pub(crate) async fn open_session(&self) -> Result<Channel<Msg>, AppError> {
        self.inner
            .channel_open_session()
            .await
            .map_err(|error| AppError::with_detail(ErrorCode::ChannelFailed, error.to_string()))
    }

    pub(crate) async fn open_direct_tcpip(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Channel<Msg>, AppError> {
        self.inner
            .channel_open_direct_tcpip(host.to_string(), port as u32, "127.0.0.1", 0)
            .await
            .map_err(|error| AppError::with_detail(ErrorCode::ForwardFailed, error.to_string()))
    }

    pub(crate) async fn tcpip_forward(&self, host: &str, port: u16) -> Result<u32, AppError> {
        self.inner
            .tcpip_forward(host.to_string(), port as u32)
            .await
            .map_err(|error| AppError::with_detail(ErrorCode::ForwardFailed, error.to_string()))
    }

    pub(crate) async fn cancel_tcpip_forward(&self, host: &str, port: u16) -> Result<(), AppError> {
        self.inner
            .cancel_tcpip_forward(host.to_string(), port as u32)
            .await
            .map_err(|error| AppError::with_detail(ErrorCode::ForwardFailed, error.to_string()))
    }

    pub(crate) fn register_remote_target(&self, port: u16, host: String, target_port: u16) {
        if let Ok(mut map) = self.remote_forwards.lock() {
            map.insert(port, (host, target_port));
        }
    }

    pub(crate) fn unregister_remote_target(&self, port: u16) {
        if let Ok(mut map) = self.remote_forwards.lock() {
            map.remove(&port);
        }
    }

    /// 执行命令并返回标准输出，忽略退出码与错误输出。
    ///
    /// 适用于系统状态采样这类“失败即降级”的只读命令；需要判定成败时
    /// 应使用 [`Self::exec_command_capture`]。
    pub async fn exec_command(&self, command: &str) -> Result<String, AppError> {
        Ok(self.exec_command_capture(command).await?.stdout)
    }

    /// 执行命令并完整捕获标准输出、错误输出与远端退出码。
    ///
    /// 输出总量超过 [`MAX_EXEC_OUTPUT_BYTES`] 时中止读取并返回 `Protocol`
    /// 错误，整体等待超过 [`EXEC_COMMAND_TIMEOUT`] 时返回 `Timeout`；调用方
    /// 需要更短时限时自行叠加 `tokio::time::timeout` 即可（取更小者生效）。
    ///
    /// # Errors
    /// 打开通道或发送 exec 请求失败时返回 `ChannelFailed`；对端回放超出
    /// 上限的输出时返回 `Protocol`；挂起超过兜底时限时返回 `Timeout`。
    pub async fn exec_command_capture(&self, command: &str) -> Result<CommandOutput, AppError> {
        match tokio::time::timeout(EXEC_COMMAND_TIMEOUT, self.exec_command_collect(command)).await {
            Ok(result) => result,
            Err(_) => Err(AppError::with_detail(
                ErrorCode::Timeout,
                format!(
                    "远端命令执行超时，已等待 {} 秒，命令: {}",
                    EXEC_COMMAND_TIMEOUT.as_secs(),
                    command_excerpt(command)
                ),
            )),
        }
    }

    /// 读取循环本体：逐块累积输出，累计超限即关闭通道并报错。
    async fn exec_command_collect(&self, command: &str) -> Result<CommandOutput, AppError> {
        let mut channel = self.open_session().await?;
        channel
            .exec(true, command)
            .await
            .map_err(|error| AppError::with_detail(ErrorCode::ChannelFailed, error.to_string()))?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => {
                    if exec_output_exceeds(&stdout, &stderr, data.len()) {
                        let _ = channel.close().await;
                        return Err(exec_output_limit_error(command));
                    }
                    stdout.extend_from_slice(&data);
                }
                ChannelMsg::ExtendedData { data, ext: 1 } => {
                    if exec_output_exceeds(&stdout, &stderr, data.len()) {
                        let _ = channel.close().await;
                        return Err(exec_output_limit_error(command));
                    }
                    stderr.extend_from_slice(&data);
                }
                ChannelMsg::ExitStatus {
                    exit_status: status,
                } => exit_status = Some(status),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok(CommandOutput {
            exit_status,
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
        })
    }

    pub async fn disconnect(&self, description: &str) {
        let _ = self
            .inner
            .disconnect(Disconnect::ByApplication, description, "en")
            .await;
        // 由内向外关闭跳板机：最内层离目标最近，先释放它再逐级退出外层会话。
        for handle in self.jump_handles.iter().rev() {
            let _ = handle
                .disconnect(Disconnect::ByApplication, description, "en")
                .await;
        }
    }
}

/// 本次追加后输出总量是否超过上限（stdout 与 stderr 合计）。
///
/// 抽成纯函数供单测钉住边界：正好等于上限放行，超一字节即拒绝。
fn exec_output_exceeds(stdout: &[u8], stderr: &[u8], incoming: usize) -> bool {
    stdout
        .len()
        .saturating_add(stderr.len())
        .saturating_add(incoming)
        > MAX_EXEC_OUTPUT_BYTES
}

/// 输出超限的统一错误描述。
fn exec_output_limit_error(command: &str) -> AppError {
    AppError::with_detail(
        ErrorCode::Protocol,
        format!(
            "远端命令输出超过 {} 字节上限，已中止读取，命令: {}",
            MAX_EXEC_OUTPUT_BYTES,
            command_excerpt(command)
        ),
    )
}

/// 取命令文本的排障摘要：过长时按字符边界截断。
///
/// 为什么截断：错误信息会进日志，命令可能是整段部署脚本（含公钥），
/// 不截断会把日志行撑到不可读；截断点回退到字符边界，避免在多字节
/// 字符中间切开产生乱码。
fn command_excerpt(command: &str) -> String {
    if command.len() <= COMMAND_EXCERPT_BYTES {
        return command.to_string();
    }
    let mut end = COMMAND_EXCERPT_BYTES;
    while !command.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &command[..end])
}

pub struct SshSession {
    pub io: TerminalIo,
    pub join: JoinHandle<()>,
    pub remote: SshRemote,
    /// 整条 SSH 连接的传输层累计流量，供状态栏采样。
    pub traffic: Arc<TrafficCounter>,
}

/// SSH 建连参数。
///
/// 参数超过五个后收敛为结构体：跳板链、凭据、提示器与 known_hosts 都是
/// "本次连接一次性"的输入，平铺会让调用点难以核对顺序与遗漏。
pub struct SshConnectParams<'a> {
    pub connection: &'a SshConnection,
    /// 已解析的跳板链，顺序为最外层到最内层；空表示直连。
    pub jumps: &'a [SshHop],
    pub size: TerminalSize,
    pub credentials: ConnectionSecrets,
    pub prompter: Arc<dyn AuthPrompter>,
    pub known_hosts: Arc<dyn KnownHostsStore>,
    /// 会话实际生效的出站代理（已按"会话优先于全局"解析）。
    pub proxy: OutboundProxy,
}

pub async fn connect_ssh(params: SshConnectParams<'_>) -> Result<SshSession, AppError> {
    let SshConnectParams {
        connection,
        jumps,
        size,
        credentials,
        prompter,
        known_hosts,
        proxy,
    } = params;
    if connection.host.trim().is_empty() {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "主机地址为空",
        ));
    }
    if connection.username.trim().is_empty() {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "用户名为空",
        ));
    }

    let remote_forwards = Arc::new(Mutex::new(HashMap::new()));
    let (session, traffic, jump_handles) = establish_chain(ChainParams {
        target: connection,
        jumps,
        credentials,
        prompter: prompter.clone(),
        known_hosts,
        remote_forwards: remote_forwards.clone(),
        proxy,
    })
    .await?;

    let channel = session
        .channel_open_session()
        .await
        .map_err(|error| AppError::with_detail(ErrorCode::ChannelFailed, error.to_string()))?;

    channel
        .request_pty(
            false,
            "xterm-256color",
            size.cols as u32,
            size.rows as u32,
            0,
            0,
            &[],
        )
        .await
        .map_err(|error| AppError::with_detail(ErrorCode::ChannelFailed, error.to_string()))?;
    let _ = channel.set_env(false, "COLORTERM", "truecolor").await;
    channel
        .request_shell(false)
        .await
        .map_err(|error| AppError::with_detail(ErrorCode::ChannelFailed, error.to_string()))?;

    let (input_tx, input_rx) = flume::bounded::<Vec<u8>>(64);
    let (output_tx, output_rx) = flume::bounded::<Vec<u8>>(128);
    let (control, control_rx) = TerminalControlHandle::channel(16);
    let remote = SshRemote {
        inner: Arc::new(session),
        remote_forwards,
        jump_handles,
    };

    let join = tokio::spawn(run_terminal_actor(
        remote.clone(),
        channel,
        input_rx,
        control_rx,
        output_tx,
    ));

    let io = TerminalIo::new(
        Box::new(ChannelReader::new(output_rx)),
        Box::new(ChannelWriter::new(input_tx)),
        control,
    );

    Ok(SshSession {
        io,
        join,
        remote,
        traffic,
    })
}

/// 建连链路的完整输入：跳板链、目标、凭据与各跳共享的认证设施。
struct ChainParams<'a> {
    target: &'a SshConnection,
    jumps: &'a [SshHop],
    credentials: ConnectionSecrets,
    prompter: Arc<dyn AuthPrompter>,
    known_hosts: Arc<dyn KnownHostsStore>,
    /// 目标会话的远端转发登记表；与 `ClientHandler` 共享同一个 Arc。
    remote_forwards: Arc<Mutex<HashMap<u16, (String, u16)>>>,
    /// 会话实际生效的出站代理；只作用于最外层物理连接。
    proxy: OutboundProxy,
}

/// 单跳握手与认证的输入。
struct HandshakeParams<'a> {
    connection: &'a SshConnection,
    credentials: ConnectionSecrets,
    prompter: Arc<dyn AuthPrompter>,
    known_hosts: Arc<dyn KnownHostsStore>,
    config: Arc<client::Config>,
    remote_forwards: Arc<Mutex<HashMap<u16, (String, u16)>>>,
}

/// 逐跳建链过程中的累积状态。
struct ChainBuild<'a> {
    params: ChainParams<'a>,
    config: Arc<client::Config>,
    traffic: Arc<TrafficCounter>,
}

/// 协商偏好：保留 russh 的现代默认值，但不接受 SHA1 的 `ssh-rsa` 主机签名。
///
/// 设计原理（Why）：
/// - russh 默认列表把 `Rsa { hash: None }` 放在末尾，遇到只提供旧签名的服务端
///   会降级到 SHA-1；OpenSSH 8.0+ 早已支持 `rsa-sha2-256/512`，没有保留 SHA-1
///   的必要。RSA 主机密钥仍可走 SHA2 变体，不会因为这次收紧而失效。
/// - 只过滤主机密钥，保留 AES-CTR：嵌入式设备与交换机管理口的 SSH 实现常缺
///   AES-GCM，而 CTR + EtM MAC 并未被实际攻破；为了兼容真实运维靶机，不把
///   “只留 AEAD 套件”当成安全收益去破坏可用性。
/// - 代价与局限：只支持 `ssh-rsa` SHA1 的远古服务端将无法连接；这是有意为之，
///   错误会由 russh 协商阶段报出，便于用户升级设备而不是静默使用弱签名。
fn ssh_preferred() -> Preferred {
    let mut preferred = Preferred::DEFAULT;
    preferred.key = Cow::Owned(
        preferred
            .key
            .iter()
            .filter(|algorithm| !matches!(algorithm, Algorithm::Rsa { hash: None }))
            .cloned()
            .collect(),
    );
    preferred
}

/// 建立"TCP（或代理）-> 各跳板机 SSH -> 目标 SSH"的完整链路。
///
/// 每一跳都复用手头的 `prompter` 与 `known_hosts`：跳板机同样走 TOFU 指纹
/// 校验与按需索取凭据，安全口径与直连一致。中途失败会关闭已经建立的跳板
/// 会话，不把半条链路留在后台。
async fn establish_chain(
    params: ChainParams<'_>,
) -> Result<
    (
        Handle<ClientHandler>,
        Arc<TrafficCounter>,
        Vec<Arc<Handle<ClientHandler>>>,
    ),
    AppError,
> {
    // 下载方向的吞吐上限约为「通道窗口 / RTT」。russh 默认 2 MiB 与 OpenSSH
    // 一致，在百毫秒级链路上会把单通道压到约 20 MB/s；提高到 16 MiB 后，
    // 与 SFTP 侧单句柄 64 个约 255 KiB 的预取请求正好匹配。窗口是通道级配置，
    // 终端、SFTP 与端口转发共用同一个值；它只是流控额度，不会按窗口大小
    // 预分配内存。
    let config = Arc::new(client::Config {
        nodelay: true,
        window_size: 16 * 1024 * 1024,
        keepalive_interval: keepalive_interval(&params.target.options),
        keepalive_max: 3,
        preferred: ssh_preferred(),
        ..Default::default()
    });
    // 只统计目标 SSH 会话的密文吞吐：外层跳板机的密文包含整条隧道，
    // 两层都计会成倍虚高，状态栏显示的"本连接流量"应与直连时同口径。
    let traffic = Arc::new(TrafficCounter::default());
    let (result, jump_handles) = build_chain(ChainBuild {
        params,
        config,
        traffic: traffic.clone(),
    })
    .await;
    match result {
        Ok(session) => Ok((session, traffic, jump_handles)),
        Err(error) => {
            disconnect_handles(&jump_handles, "跳板链建立失败").await;
            Err(error)
        }
    }
}

/// 依次完成各跳握手并打通下一跳的 `direct-tcpip` 通道。
///
/// 无论成功与否都返回已建立的跳板句柄，交由调用方统一清理：中途失败时
/// 已认证的跳板会话不能留在后台。
async fn build_chain(
    build: ChainBuild<'_>,
) -> (
    Result<Handle<ClientHandler>, AppError>,
    Vec<Arc<Handle<ClientHandler>>>,
) {
    let ChainBuild {
        params:
            ChainParams {
                target,
                jumps,
                credentials,
                prompter,
                known_hosts,
                remote_forwards,
                proxy,
            },
        config,
        traffic,
    } = build;
    let mut jump_handles = Vec::with_capacity(jumps.len());
    // 第一段物理 TCP 永远连最外层跳板机；没有跳板时才直连目标。
    let dial_target = jumps.first().map_or(target, |hop| &hop.connection);
    let outer_timeout = dial_target.options.connect_timeout_secs.max(1);
    let transport = match tokio::time::timeout(
        Duration::from_secs(outer_timeout),
        open_transport(dial_target, &proxy),
    )
    .await
    {
        Ok(Ok(transport)) => transport,
        Ok(Err(error)) => return (Err(error), jump_handles),
        Err(_) => {
            let error = NetError::Connect {
                host: dial_target.host.clone(),
                port: dial_target.port,
                source: std::io::Error::new(std::io::ErrorKind::TimedOut, "连接阶段超时"),
            };
            return (Err(error.into()), jump_handles);
        }
    };
    let mut transport: Box<dyn AsyncStream> = Box::new(transport);

    for (index, hop) in jumps.iter().enumerate() {
        let handle = match timed_handshake(
            transport,
            HandshakeParams {
                connection: &hop.connection,
                credentials: hop.secrets.clone(),
                prompter: prompter.clone(),
                known_hosts: known_hosts.clone(),
                config: config.clone(),
                remote_forwards: Arc::new(Mutex::new(HashMap::new())),
            },
        )
        .await
        {
            Ok(handle) => handle,
            Err(error) => {
                return (
                    Err(AppError::with_detail(
                        error.code,
                        format!(
                            "经跳板机 {}:{} 建连失败: {}",
                            hop.connection.host,
                            hop.connection.port,
                            error.detail.as_deref().unwrap_or("未知原因")
                        ),
                    )),
                    jump_handles,
                );
            }
        };
        let (next_host, next_port) = jumps
            .get(index + 1)
            .map_or((&target.host, target.port), |next| {
                (&next.connection.host, next.connection.port)
            });
        let channel = handle
            .channel_open_direct_tcpip(next_host.clone(), next_port as u32, "127.0.0.1", 0)
            .await;
        let channel = match channel {
            Ok(channel) => channel,
            Err(error) => {
                // 当前这一跳可能已经完成认证；先登记再统一由外层清理。
                jump_handles.push(Arc::new(handle));
                let error = NetError::Channel {
                    reason: format!(
                        "经跳板机 {}:{} 打开到 {next_host}:{next_port} 的直连通道失败: {error}",
                        hop.connection.host, hop.connection.port
                    ),
                };
                return (Err(error.into()), jump_handles);
            }
        };
        log::debug!(
            "跳板机通道已建立，跳板机: {}:{}，下一跳: {next_host}:{next_port}",
            hop.connection.host,
            hop.connection.port
        );
        transport = Box::new(AsyncChannelStream::new(channel));
        jump_handles.push(Arc::new(handle));
    }

    let final_transport = CountingStream::new(transport, traffic);
    let result = timed_handshake(
        final_transport,
        HandshakeParams {
            connection: target,
            credentials,
            prompter,
            known_hosts,
            config,
            remote_forwards,
        },
    )
    .await;
    (result, jump_handles)
}

/// 打开目标地址的物理连接；直连或经 SOCKS5/HTTP CONNECT 代理隧道。
///
/// 代理只作用于最外层物理连接：内层跳板机都跑在同一条隧道里。
async fn open_transport(
    connection: &SshConnection,
    proxy: &OutboundProxy,
) -> Result<TcpStream, AppError> {
    let socket = crate::proxy::connect_via_proxy(&connection.host, connection.port, proxy).await?;
    if let Err(error) = socket.set_nodelay(true) {
        log::debug!("设置 TCP_NODELAY 失败，将继续使用默认行为，原因: {error}");
    }
    Ok(socket)
}

/// 在给定传输流上完成一次 SSH 握手与认证，超时按该节点自身的配置。
async fn timed_handshake<S>(
    transport: S,
    params: HandshakeParams<'_>,
) -> Result<Handle<ClientHandler>, AppError>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let timeout_secs = params.connection.options.connect_timeout_secs.max(1);
    let host = params.connection.host.clone();
    let port = params.connection.port;
    tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        handshake(transport, params),
    )
    .await
    .map_err(|_| {
        AppError::from(NetError::Handshake {
            host,
            port,
            reason: format!("连接超时，已等待 {timeout_secs} 秒"),
        })
    })?
}

async fn handshake<S>(
    transport: S,
    params: HandshakeParams<'_>,
) -> Result<Handle<ClientHandler>, AppError>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let HandshakeParams {
        connection,
        credentials,
        prompter,
        known_hosts,
        config,
        remote_forwards,
    } = params;
    let handler = ClientHandler {
        host: connection.host.clone(),
        port: connection.port,
        known_hosts,
        remote_forwards,
        // 同一个 Arc 稍后还要传给 authenticate 做凭据索取，这里只克隆引用
        prompter: prompter.clone(),
    };
    let mut session = client::connect_stream(config, transport, handler)
        .await
        .map_err(map_connect_error)?;
    authenticate(&mut session, connection, credentials, prompter).await?;
    Ok(session)
}

async fn disconnect_handles(handles: &[Arc<Handle<ClientHandler>>], reason: &str) {
    for handle in handles.iter().rev() {
        let _ = handle
            .disconnect(Disconnect::ByApplication, reason, "en")
            .await;
    }
}

fn keepalive_interval(options: &SessionOptions) -> Option<Duration> {
    (options.keepalive_secs > 0).then(|| Duration::from_secs(options.keepalive_secs))
}

fn map_connect_error(error: ClientError) -> AppError {
    if matches!(
        error.0.code,
        ErrorCode::HostKeyMismatch | ErrorCode::HostKeyUnknown
    ) {
        error.0
    } else {
        AppError::with_detail(ErrorCode::ConnectionFailed, error.0.to_string())
    }
}

async fn authenticate(
    session: &mut Handle<ClientHandler>,
    connection: &SshConnection,
    mut credentials: ConnectionSecrets,
    prompter: Arc<dyn AuthPrompter>,
) -> Result<(), AppError> {
    let user = connection.username.clone();
    let authenticated = match &connection.auth {
        transom_core::AuthMethod::Password { .. } => {
            password_flow(
                session,
                &user,
                prompter.as_ref(),
                credentials.password.take(),
            )
            .await?
        }
        transom_core::AuthMethod::PrivateKey { .. } => {
            authenticate_private_key(session, connection, &mut credentials, prompter.as_ref())
                .await?
        }
        transom_core::AuthMethod::Agent => authenticate_agent(session, &user).await?,
        _ => {
            return Err(NetError::Auth {
                reason: "不支持的认证方式".to_string(),
            }
            .into());
        }
    };

    if authenticated {
        return Ok(());
    }

    if connection.options.allow_keyboard_interactive
        && authenticate_keyboard_interactive(session, &user, prompter).await?
    {
        return Ok(());
    }

    Err(NetError::Auth {
        reason: "服务器拒绝了全部认证方式".to_string(),
    }
    .into())
}

/// 密码认证的尝试上限，对齐 OpenSSH 给用户的三次机会。
const MAX_PASSWORD_ATTEMPTS: usize = 3;

/// 密码认证流程：先用手上的凭据试，被服务器拒绝后重新索取，最多三次。
///
/// 为什么要在同一次认证里重试：服务器拒绝密码不返回任何原因文本，早期实现一次即
/// 判死，打错一个字符就得回标签页点"重新连接"。手上的凭据可能来自钥匙串，也可能
/// 为空（本机从未保存），为空时第一次就问用户。
async fn password_flow(
    session: &mut Handle<ClientHandler>,
    user: &str,
    prompter: &dyn AuthPrompter,
    stored: Option<Zeroizing<String>>,
) -> Result<bool, AppError> {
    let mut candidate = stored;
    for attempt in 0..MAX_PASSWORD_ATTEMPTS {
        let password = match candidate.take() {
            Some(password) => password,
            None => request_secret(prompter, PromptKind::Password).await?,
        };
        let accepted = session
            .authenticate_password(user.to_string(), password.to_string())
            .await
            .map_err(auth_error)?
            .success();
        if accepted {
            return Ok(true);
        }
        if attempt + 1 < MAX_PASSWORD_ATTEMPTS {
            log::warn!(
                "密码被服务器拒绝，将在本次认证中重新询问，用户: {user}，本次第 {} 次，上限 {} 次",
                attempt + 1,
                MAX_PASSWORD_ATTEMPTS
            );
        }
    }
    Ok(false)
}

/// 借用 `Option<Zeroizing<String>>` 里的字符串引用，避免为一次性参数克隆口令。
/// 用私钥完成认证，可选附带企业 CA 签发的 OpenSSH 用户证书。
///
/// 从 `authenticate` 里拆出来：这条路径上有"口令索取—证书加载—回退裸公钥"三段，
/// 塞进 match 分支既超出函数长度约束，也让人看不清回退次序。
async fn authenticate_private_key(
    session: &mut Handle<ClientHandler>,
    connection: &SshConnection,
    credentials: &mut ConnectionSecrets,
    prompter: &dyn AuthPrompter,
) -> Result<bool, AppError> {
    let transom_core::AuthMethod::PrivateKey {
        path, cert_path, ..
    } = &connection.auth
    else {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "内部错误：认证方式不是私钥却进入了私钥分支",
        ));
    };
    let user = connection.username.clone();
    let key = match load_secret_key(path, credential_ref(&credentials.key_passphrase)) {
        Ok(key) => key,
        // 未加密私钥用 None 必然加载成功，因此失败只可能是"私钥带口令而本机没存口令"
        // 或口令本身错误。前者问一次就能继续，后者会在用户输入错口令后走同样的分支报错。
        Err(first_error) => match &credentials.key_passphrase {
            Some(_) => return Err(load_key_error(path, &first_error)),
            None => {
                let passphrase = request_secret(prompter, PromptKind::Passphrase).await?;
                load_secret_key(path, Some(passphrase.as_str()))
                    .map_err(|error| load_key_error(path, &error))?
            }
        },
    };
    let hash_alg = session
        .best_supported_rsa_hash()
        .await
        .map_err(auth_error)?
        .flatten();
    let key = Arc::new(key);
    let Some(cert_path) = cert_path else {
        return pubkey_auth(session, &user, key, hash_alg).await;
    };
    let certificate = russh::keys::load_openssh_certificate(cert_path).map_err(|error| {
        AppError::with_detail(
            ErrorCode::AuthFailed,
            format!(
                "加载 SSH 用户证书失败，路径: {}，原因: {error}",
                cert_path.display()
            ),
        )
    })?;
    let via_certificate = session
        .authenticate_openssh_cert(user.clone(), Arc::clone(&key), certificate)
        .await
        .map_err(auth_error)?;
    if via_certificate.success() {
        return Ok(true);
    }
    // 退回裸公钥认证：服务器没配 trusteduserca、证书过期或 principal 不匹配时，同一把私钥
    // 往往仍在其 authorized_keys 里；只发证书会让这些机器直接连不上。回退不是静默掩盖问题，
    // 证书被拒已经按 warn 记录，路径与主机都在日志里。
    log::warn!(
        "证书认证被服务器拒绝，回退到公钥认证，主机: {}，证书: {}",
        connection.host,
        cert_path.display()
    );
    pubkey_auth(session, &user, key, hash_alg).await
}

/// 裸公钥认证一步。
async fn pubkey_auth(
    session: &mut Handle<ClientHandler>,
    user: &str,
    key: Arc<russh::keys::PrivateKey>,
    hash_alg: Option<HashAlg>,
) -> Result<bool, AppError> {
    session
        .authenticate_publickey(user.to_owned(), PrivateKeyWithHashAlg::new(key, hash_alg))
        .await
        .map_err(auth_error)
        .map(|result| result.success())
}

fn credential_ref(value: &Option<Zeroizing<String>>) -> Option<&str> {
    value.as_ref().map(|secret| secret.as_str())
}

/// 私钥加载失败的统一错误描述。
fn load_key_error(path: &std::path::Path, error: &russh::keys::Error) -> AppError {
    NetError::Auth {
        reason: format!("加载私钥失败 {}: {error}", path.display()),
    }
    .into()
}

/// 认证过程中向调用方索取一个机密输入（登录密码或私钥口令）。
///
/// 复用 `AuthPrompter` 而不是新开一条通道：控制器侧的 `UiPrompter` 已经实现了
/// "发事件 -> 等 responder"的完整往返，这里只是把提示语义换成我们自己生成的类别。
async fn request_secret(
    prompter: &dyn AuthPrompter,
    kind: PromptKind,
) -> Result<Zeroizing<String>, AppError> {
    let answers = prompter
        .keyboard_interactive("", "", &[PromptInfo::secret(kind)])
        .await?;
    let answer = answers
        .first()
        .filter(|answer| !answer.is_empty())
        .ok_or_else(|| NetError::Cancelled {
            reason: "凭据输入为空".to_string(),
        })?;
    Ok(Zeroizing::new(answer.clone()))
}

fn auth_error(error: russh::Error) -> AppError {
    AppError::with_detail(ErrorCode::AuthFailed, error.to_string())
}

/// ssh-agent 环节错误统一描述：连接、列举身份与逐把签名三类失败共用。
fn agent_error(error: impl std::fmt::Display) -> NetError {
    NetError::Auth {
        reason: format!("SSH 代理（ssh-agent）认证失败: {error}"),
    }
}

async fn authenticate_agent(
    session: &mut Handle<ClientHandler>,
    user: &str,
) -> Result<bool, AppError> {
    #[cfg(unix)]
    {
        let mut agent = AgentClient::connect_env().await.map_err(agent_error)?;
        let identities = agent.request_identities().await.map_err(agent_error)?;
        for identity in identities {
            let public_key = identity.public_key().into_owned();
            let result = session
                .authenticate_publickey_with(user.to_owned(), public_key, None, &mut agent)
                .await
                .map_err(agent_error)?;
            if result.success() {
                return Ok(true);
            }
        }
        Ok(false)
    }
    #[cfg(not(unix))]
    {
        let _ = (session, user);
        Err(NetError::Auth {
            reason: "当前平台暂不支持 ssh-agent 认证".to_string(),
        }
        .into())
    }
}

async fn authenticate_keyboard_interactive(
    session: &mut Handle<ClientHandler>,
    user: &str,
    prompter: Arc<dyn AuthPrompter>,
) -> Result<bool, AppError> {
    use russh::client::KeyboardInteractiveAuthResponse;

    let mut response = session
        .authenticate_keyboard_interactive_start(user.to_owned(), None)
        .await
        .map_err(auth_error)?;

    for _ in 0..8 {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let prompt_info: Vec<PromptInfo> = prompts
                    .iter()
                    .map(|prompt| PromptInfo {
                        prompt: prompt.prompt.clone(),
                        echo: prompt.echo,
                        kind: PromptKind::Text,
                    })
                    .collect();
                let answers = prompter
                    .keyboard_interactive(&name, &instructions, &prompt_info)
                    .await?;
                if answers.len() != prompt_info.len() {
                    return Err(AppError::with_detail(
                        ErrorCode::KeyboardInteractiveFailed,
                        "prompt response count mismatch",
                    ));
                }
                response = session
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(auth_error)?;
            }
        }
    }

    Err(AppError::with_detail(
        ErrorCode::KeyboardInteractiveFailed,
        "too many keyboard-interactive rounds",
    ))
}

async fn run_terminal_actor(
    remote: SshRemote,
    mut channel: Channel<Msg>,
    input_rx: flume::Receiver<Vec<u8>>,
    mut control_rx: tokio::sync::mpsc::Receiver<TerminalControl>,
    output_tx: flume::Sender<Vec<u8>>,
) {
    enum Activity {
        Channel(Option<ChannelMsg>),
        Input(Result<Vec<u8>, flume::RecvError>),
        Control(Option<TerminalControl>),
    }

    loop {
        let activity = tokio::select! {
            message = channel.wait() => Activity::Channel(message),
            input = input_rx.recv_async() => Activity::Input(input),
            control = control_rx.recv() => Activity::Control(control),
        };

        match activity {
            Activity::Channel(Some(ChannelMsg::Data { data })) => {
                if output_tx.send_async(data.to_vec()).await.is_err() {
                    break;
                }
            }
            Activity::Channel(Some(ChannelMsg::ExtendedData { data, .. })) => {
                if output_tx.send_async(data.to_vec()).await.is_err() {
                    break;
                }
            }
            Activity::Channel(Some(ChannelMsg::Eof))
            | Activity::Channel(Some(ChannelMsg::Close)) => {
                break;
            }
            Activity::Channel(Some(ChannelMsg::ExitStatus { .. }))
            | Activity::Channel(Some(ChannelMsg::ExitSignal { .. }))
            | Activity::Channel(None) => break,
            Activity::Channel(Some(_)) => {}
            Activity::Input(Ok(bytes)) => {
                if channel.data_bytes(bytes).await.is_err() {
                    break;
                }
            }
            Activity::Input(Err(_)) => break,
            Activity::Control(Some(TerminalControl::Resize(size))) => {
                if let Err(error) = channel
                    .window_change(size.cols as u32, size.rows as u32, 0, 0)
                    .await
                {
                    log::warn!(
                        "同步远端 PTY 窗口尺寸失败，远端 shell 将以旧尺寸排版，画面可能持续错位，列数: {}，行数: {}，原因: {error}",
                        size.cols,
                        size.rows
                    );
                }
            }
            Activity::Control(Some(TerminalControl::Close)) => {
                let _ = channel.eof().await;
                let _ = channel.close().await;
                break;
            }
            Activity::Control(None) => break,
        }
    }

    remote.disconnect("terminal closed").await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use russh::client::Handler;
    use russh::keys::key::safe_rng;
    use russh::keys::{Algorithm, PrivateKey};

    /// 测试替身：按预设答案回应认证索取，`Err` 分支模拟用户取消；
    /// `host_key` 决定 TOFU 指纹确认的答复，模拟用户信任或拒绝。
    struct ScriptedPrompter {
        answers: Result<Vec<String>, ()>,
        host_key: bool,
    }

    #[async_trait::async_trait]
    impl AuthPrompter for ScriptedPrompter {
        async fn confirm_host_key(
            &self,
            _host: &str,
            _port: u16,
            _key_type: &str,
            _fingerprint: &str,
        ) -> Result<bool, AppError> {
            Ok(self.host_key)
        }

        async fn keyboard_interactive(
            &self,
            _name: &str,
            _instructions: &str,
            prompts: &[PromptInfo],
        ) -> Result<Vec<String>, AppError> {
            self.answers.clone().map_err(|_| {
                AppError::with_detail(
                    ErrorCode::Cancelled,
                    format!("模拟用户取消认证，待回答提示数: {}", prompts.len()),
                )
            })
        }
    }

    struct MemoryKnownHosts {
        inner: Mutex<Vec<(String, u16, String)>>,
    }

    impl KnownHostsStore for MemoryKnownHosts {
        fn check(
            &self,
            host: &str,
            port: u16,
            info: &HostKeyInfo,
        ) -> Result<HostKeyStatus, AppError> {
            let inner = self.inner.lock();
            match inner
                .iter()
                .find(|(saved_host, saved_port, _)| saved_host == host && *saved_port == port)
            {
                Some((_, _, fingerprint)) if fingerprint == &info.fingerprint => {
                    Ok(HostKeyStatus::Match)
                }
                Some((_, _, fingerprint)) => Ok(HostKeyStatus::Mismatch {
                    previous_fingerprint: fingerprint.clone(),
                }),
                None => Ok(HostKeyStatus::Unknown),
            }
        }

        fn record(&self, host: &str, port: u16, info: &HostKeyInfo) -> Result<(), AppError> {
            self.inner
                .lock()
                .push((host.to_owned(), port, info.fingerprint.clone()));
            Ok(())
        }

        fn remove(&self, _host: &str, _port: u16) -> Result<(), AppError> {
            Ok(())
        }

        fn list(&self) -> Result<Vec<transom_core::HostKeyRecord>, AppError> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn handler_records_unknown_and_rejects_changed_keys() {
        let key = PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
        let public = key.public_key();
        let server_key = PublicKeyOrCertificate::from(public.clone());

        let known_hosts = Arc::new(MemoryKnownHosts {
            inner: Mutex::new(Vec::new()),
        });
        let mut handler = ClientHandler {
            host: "example.com".into(),
            port: 22,
            known_hosts: known_hosts.clone(),
            remote_forwards: Arc::new(std::sync::Mutex::new(HashMap::new())),
            prompter: Arc::new(ScriptedPrompter {
                answers: Ok(Vec::new()),
                host_key: true,
            }),
        };

        assert!(handler.check_server_key(&server_key).await.unwrap());
        assert!(handler.check_server_key(&server_key).await.unwrap());
        assert_eq!(known_hosts.inner.lock().len(), 1);

        let other = PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
        let other_key = PublicKeyOrCertificate::from(other.public_key().clone());
        assert!(handler.check_server_key(&other_key).await.is_err());
    }

    /// TOFU 拒绝路径：用户不确认指纹时连接必须失败，且不得写入 known_hosts。
    #[tokio::test]
    async fn handler_rejected_host_key_is_not_recorded() {
        let key = PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
        let server_key = PublicKeyOrCertificate::from(key.public_key().clone());
        let known_hosts = Arc::new(MemoryKnownHosts {
            inner: Mutex::new(Vec::new()),
        });
        let mut handler = ClientHandler {
            host: "example.com".into(),
            port: 22,
            known_hosts: known_hosts.clone(),
            remote_forwards: Arc::new(std::sync::Mutex::new(HashMap::new())),
            prompter: Arc::new(ScriptedPrompter {
                answers: Ok(Vec::new()),
                host_key: false,
            }),
        };
        assert!(handler.check_server_key(&server_key).await.is_err());
        assert!(
            known_hosts.inner.lock().is_empty(),
            "拒绝指纹时绝不能记录信任"
        );
    }

    #[tokio::test]
    async fn request_secret_returns_the_prompted_answer() {
        let prompter = ScriptedPrompter {
            answers: Ok(vec!["s3cret".to_string()]),
            host_key: true,
        };
        let secret = request_secret(&prompter, PromptKind::Password)
            .await
            .expect("应当取回用户输入的凭据");
        assert_eq!(&*secret, "s3cret");
    }

    #[tokio::test]
    async fn request_secret_fails_on_empty_or_cancelled_input() {
        let blank = ScriptedPrompter {
            answers: Ok(vec![String::new()]),
            host_key: true,
        };
        assert!(
            request_secret(&blank, PromptKind::Passphrase)
                .await
                .is_err()
        );

        let cancelled = ScriptedPrompter {
            answers: Err(()),
            host_key: true,
        };
        assert!(
            request_secret(&cancelled, PromptKind::Password)
                .await
                .is_err()
        );
    }

    #[test]
    fn ssh_preferred_drops_sha1_rsa_host_keys_but_keeps_modern_algorithms() {
        let preferred = ssh_preferred();
        assert!(
            !preferred
                .key
                .iter()
                .any(|algorithm| matches!(algorithm, Algorithm::Rsa { hash: None })),
            "不能接受 SHA1 的 ssh-rsa 主机签名"
        );
        assert!(
            preferred.key.contains(&Algorithm::Ed25519),
            "Ed25519 主机密钥必须保留"
        );
        assert!(
            preferred.key.contains(&Algorithm::Rsa {
                hash: Some(HashAlg::Sha256)
            }),
            "RSA 主机密钥仍应可走 SHA2 签名"
        );
        assert!(
            !preferred.cipher.is_empty(),
            "过滤主机密钥不应连带清空加密套件列表"
        );
    }

    /// 输出上限的边界：stdout 与 stderr 合计恰好等于上限放行，超一字节即拒绝。
    #[test]
    fn exec_output_limit_counts_stdout_and_stderr_together() {
        let half = MAX_EXEC_OUTPUT_BYTES / 2;
        assert!(
            !exec_output_exceeds(&vec![0u8; MAX_EXEC_OUTPUT_BYTES], &[], 0),
            "恰好等于上限不应判定超限"
        );
        assert!(
            exec_output_exceeds(&vec![0u8; MAX_EXEC_OUTPUT_BYTES], &[], 1),
            "超出上限一字节必须拒绝"
        );
        assert!(
            !exec_output_exceeds(&vec![0u8; half], &vec![0u8; half], 0),
            "stdout 与 stderr 各半、合计等于上限不应判定超限"
        );
        assert!(
            exec_output_exceeds(&vec![0u8; half], &vec![0u8; half], 1),
            "合计超出上限一字节必须拒绝"
        );
    }

    /// 命令摘要按字符边界截断：短命令原样返回，长命令带省略标记且不切开多字节字符。
    #[test]
    fn command_excerpt_truncates_on_char_boundary() {
        assert_eq!(command_excerpt("kill -9 1234"), "kill -9 1234");
        let long = "公钥部署命令".repeat(40);
        let excerpt = command_excerpt(&long);
        assert!(
            excerpt.len() <= COMMAND_EXCERPT_BYTES + 3,
            "截断后长度不得超过上限加省略标记"
        );
        assert!(excerpt.ends_with("..."), "截断必须带省略标记");
        assert!(excerpt.starts_with("公钥部署命令"), "截断不得破坏首段文本");
    }
}
