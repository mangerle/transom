use crate::auth::AuthPrompter;
use crate::bridge::{ChannelReader, ChannelWriter};
use crate::error::NetError;
use crate::traffic::CountingStream;
use russh::client::{self, Handle, Msg};
use russh::keys::agent::client::AgentClient;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate, load_secret_key};
use russh::{Channel, ChannelMsg, Disconnect};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use transom_core::{
    AppError, ConnectionSecrets, ErrorCode, HostKeyInfo, HostKeyStatus, KnownHostsStore,
    PromptInfo, PromptKind, SessionOptions, SshConnection, TerminalControl, TerminalControlHandle,
    TerminalIo, TerminalSize, TrafficCounter,
};
use zeroize::Zeroizing;

#[derive(Debug)]
pub struct ClientError(pub AppError);

/// 远端转发建立到目标主机的 TCP 连接的最长等待时间。
///
/// 转发目标通常是内网邻近服务，正常连接远快于该上限；取 30 秒是为
/// 跨机房高延迟链路留余量，同时兜住"对端不可达导致任务永久泄漏"。
const FORWARD_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

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
                tracing::info!(host = %self.host, port = self.port, %fingerprint, "用户确认后记录新主机密钥");
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
    /// # Errors
    /// 打开通道或发送 exec 请求失败时返回 `ChannelFailed`。
    pub async fn exec_command_capture(&self, command: &str) -> Result<CommandOutput, AppError> {
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
                ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, ext: 1 } => {
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
    }
}

pub struct SshSession {
    pub io: TerminalIo,
    pub join: JoinHandle<()>,
    pub remote: SshRemote,
    /// 整条 SSH 连接的传输层累计流量，供状态栏采样。
    pub traffic: Arc<TrafficCounter>,
}

pub async fn connect_ssh(
    connection: &SshConnection,
    size: TerminalSize,
    credentials: ConnectionSecrets,
    prompter: Arc<dyn AuthPrompter>,
    known_hosts: Arc<dyn KnownHostsStore>,
) -> Result<SshSession, AppError> {
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

    let config = client::Config {
        nodelay: true,
        keepalive_interval: keepalive_interval(&connection.options),
        keepalive_max: 3,
        ..Default::default()
    };

    let remote_forwards = Arc::new(Mutex::new(HashMap::new()));
    let handler = ClientHandler {
        host: connection.host.clone(),
        port: connection.port,
        known_hosts,
        remote_forwards: remote_forwards.clone(),
        // 同一个 Arc 稍后还要传给 authenticate 做凭据索取，这里只克隆引用
        prompter: prompter.clone(),
    };
    let connect_timeout = Duration::from_secs(connection.options.connect_timeout_secs.max(1));
    let host = connection.host.clone();
    let port = connection.port;
    let connect = async move {
        let socket = TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|error| NetError::Connect {
                host: host.clone(),
                port,
                source: error,
            })?;
        if let Err(error) = socket.set_nodelay(true) {
            tracing::debug!(%error, "设置 TCP_NODELAY 失败，将继续使用默认行为");
        }
        let traffic = Arc::new(TrafficCounter::default());
        let stream = CountingStream::new(socket, traffic.clone());
        let session = client::connect_stream(Arc::new(config), stream, handler)
            .await
            .map_err(map_connect_error)?;
        Ok::<_, AppError>((session, traffic))
    };
    let (mut session, traffic) = tokio::time::timeout(connect_timeout, connect)
        .await
        .map_err(|_| NetError::Handshake {
            host: connection.host.clone(),
            port: connection.port,
            reason: format!(
                "连接超时，已等待 {} 秒",
                connection.options.connect_timeout_secs.max(1)
            ),
        })??;

    if let Err(error) = authenticate(&mut session, connection, credentials, prompter).await {
        let _ = session
            .disconnect(Disconnect::ByApplication, "authentication failed", "en")
            .await;
        return Err(error);
    }

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
            tracing::warn!(
                user,
                attempt = attempt + 1,
                max = MAX_PASSWORD_ATTEMPTS,
                "密码被服务器拒绝，将在本次认证中重新询问"
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
    tracing::warn!(
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
    control_rx: flume::Receiver<TerminalControl>,
    output_tx: flume::Sender<Vec<u8>>,
) {
    enum Activity {
        Channel(Option<ChannelMsg>),
        Input(Result<Vec<u8>, flume::RecvError>),
        Control(Result<TerminalControl, flume::RecvError>),
    }

    loop {
        let activity = tokio::select! {
            message = channel.wait() => Activity::Channel(message),
            input = input_rx.recv_async() => Activity::Input(input),
            control = control_rx.recv_async() => Activity::Control(control),
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
            Activity::Control(Ok(TerminalControl::Resize(size))) => {
                if let Err(error) = channel
                    .window_change(size.cols as u32, size.rows as u32, 0, 0)
                    .await
                {
                    tracing::warn!(
                        cols = size.cols,
                        rows = size.rows,
                        %error,
                        "同步远端 PTY 窗口尺寸失败，远端 shell 将以旧尺寸排版，画面可能持续错位"
                    );
                }
            }
            Activity::Control(Ok(TerminalControl::Close)) => {
                let _ = channel.eof().await;
                let _ = channel.close().await;
                break;
            }
            Activity::Control(Err(_)) => break,
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
}
