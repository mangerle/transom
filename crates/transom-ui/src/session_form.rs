use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;
use std::collections::HashSet;
use std::path::PathBuf;
use transom_core::{
    AuthMethod, BlockViewOverride, Connection, IdentityId, OutboundProxy, ProxyAuth,
    ProxyJumpConfig, SerialConnection, SerialDataBits, SerialFlowControl, SerialParity,
    SerialPortDescriptor, SerialStopBits, Session, SessionColor, SessionIcon, SessionId,
    SshConnection, StartupCommands, TelnetConnection, TerminalProfile, TextEncoding,
};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthChoice {
    Password,
    PrivateKey,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionChoice {
    Ssh,
    Serial,
    Telnet,
}

/// 会话表单里的出站代理档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxyChoice {
    /// 跟随全局偏好设置（会话不落配置）。
    FollowGlobal,
    Direct,
    Socks5,
    Http,
}

/// 会话表单的构造输入。
///
/// 表单需要的目录快照（凭据模板、跳板候选、文件夹）与全局代理说明一起
/// 传入，保证打开表单后不因目录变化而悄悄换选项。
pub(crate) struct SessionFormInit {
    pub base: Option<Session>,
    pub folder_path: Option<String>,
    pub identities: Vec<IdentityOption>,
    pub jump_options: Vec<JumpOption>,
    pub folder_options: Vec<String>,
    /// 全局出站代理，仅用于"跟随全局"的说明文字。
    pub global_proxy: OutboundProxy,
    /// 语义块模式覆盖。
    pub block_view: BlockViewOverride,
}

pub(crate) struct SessionForm {
    pub base: Option<Session>,
    pub name: Entity<InputState>,
    /// 以 `/` 分隔的文件夹路径，保存时解析为稳定的文件夹节点。
    pub folder_path: Entity<InputState>,
    /// 可选的文件夹路径清单，供下拉快速选择。
    pub folder_options: Vec<String>,
    pub connection: ConnectionChoice,
    pub host: Entity<InputState>,
    pub port: Entity<InputState>,
    pub username: Entity<InputState>,
    pub password: Entity<InputState>,
    pub key_path: Entity<InputState>,
    /// CA 签发的用户证书路径（`*-cert.pub`），仅密钥认证有意义。
    pub cert_path: Entity<InputState>,
    pub passphrase: Entity<InputState>,
    pub serial_port: Entity<InputState>,
    pub serial_baud: Entity<InputState>,
    pub serial_data_bits: SerialDataBits,
    pub serial_parity: SerialParity,
    pub serial_stop_bits: SerialStopBits,
    pub serial_flow_control: SerialFlowControl,
    pub serial_ports: Vec<SerialPortDescriptor>,
    pub serial_ports_error: Option<String>,
    /// Telnet 字符编码；远端为老设备或 BBS 时选择 GBK/Big5。
    pub telnet_encoding: TextEncoding,
    pub startup_commands: Entity<TextareaState>,
    /// SSH 连接超时（秒）；留空或非法值回退会话存量/默认。
    pub connect_timeout: Entity<InputState>,
    /// SSH keepalive 间隔（秒），0 表示禁用。
    pub keepalive: Entity<InputState>,
    pub auth: AuthChoice,
    pub save_password: bool,
    pub save_passphrase: bool,
    /// 意外断线后自动重连（见 AppShell::schedule_auto_reconnect）。
    pub auto_reconnect: bool,
    /// 关机主机的网卡 MAC，供 Wake-on-LAN 使用；留空表示不启用唤醒。
    pub wol_mac: Entity<InputState>,
    /// 可选的凭据模板清单（打开表单时从目录快照而来）。
    pub identities: Vec<IdentityOption>,
    /// 当前选用的凭据模板；`None` 表示用本会话自己的用户名与认证方式。
    pub identity_choice: Option<IdentityId>,
    /// 可选的跳板机会话清单（仅 SSH，已排除自身与会成环的候选）。
    pub jump_options: Vec<JumpOption>,
    /// 当前选用的跳板机；`None` 表示直连。
    pub jump_choice: Option<SessionId>,
    /// 出站代理档位；跟随全局时会话不落代理配置。
    pub proxy_choice: ProxyChoice,
    pub proxy_host: Entity<InputState>,
    pub proxy_port: Entity<InputState>,
    pub proxy_username: Entity<InputState>,
    pub proxy_password: Entity<InputState>,
    /// 全局代理快照，用于"跟随全局"的说明文字。
    pub global_proxy: OutboundProxy,
    /// 语义块模式覆盖：跟随全局/强制开启/强制关闭。
    pub block_view: BlockViewOverride,
    /// 会话专属图标；`Auto` 表示按连接类型自动取图标。
    pub icon: SessionIcon,
    /// 会话环境色；`None` 表示不染色。
    pub color: SessionColor,
    pub confirm_delete: bool,
    pub error: Option<String>,
}

/// 会话表单里的一个凭据模板选项。
///
/// 带上用户名是因为选中模板后要用它填进连接参数：表单无法访问目录，
/// 让调用方把需要的两个字段一起给全，比在表单里再查一次目录更直接。
pub(crate) struct IdentityOption {
    pub id: IdentityId,
    pub name: String,
    pub username: String,
}

/// 会话表单里的一个跳板机选项。
///
/// 带上主机、端口与自身的跳板引用，是为了在表单里就地渲染完整拓扑链
/// （`本机 -> 堡垒机 -> 网关 -> 目标`）；表单不回查目录。
#[derive(Clone)]
pub(crate) struct JumpOption {
    pub id: SessionId,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub jump_session_id: Option<SessionId>,
}

pub(crate) struct BuiltSession {
    pub session: Session,
    pub folder_path: Option<String>,
    pub password: Option<Zeroizing<String>>,
    pub key_passphrase: Option<Zeroizing<String>>,
}

/// SSH 连接构建结果，包含需要单独写入密钥环的敏感值。
struct SshConnectionBuild {
    connection: Connection,
    password: Option<Zeroizing<String>>,
    key_passphrase: Option<Zeroizing<String>>,
}

/// 表单初始值聚合，避免构造函数平铺过多参数。
struct SessionFormSeed {
    base: Option<Session>,
    connection: ConnectionChoice,
    auth: AuthChoice,
    save_password: bool,
    save_passphrase: bool,
    key_path: String,
    cert_path: String,
    folder_path: String,
    startup_commands: String,
    connect_timeout: String,
    keepalive: String,
    serial_port: String,
    serial_baud: String,
    serial_data_bits: SerialDataBits,
    serial_parity: SerialParity,
    serial_stop_bits: SerialStopBits,
    serial_flow_control: SerialFlowControl,
    telnet_encoding: TextEncoding,
    /// 可选凭据模板清单；表单只认这份快照，不回查目录。
    identities: Vec<IdentityOption>,
    /// 可选跳板机清单；同样只认快照。
    jump_options: Vec<JumpOption>,
    folder_options: Vec<String>,
    /// 全局代理快照。
    global_proxy: OutboundProxy,
    /// 语义块模式覆盖。
    block_view: BlockViewOverride,
}

impl SessionForm {
    pub fn new(window: &mut Window, cx: &mut Context<impl Sized>, init: SessionFormInit) -> Self {
        let SessionFormInit {
            base,
            folder_path,
            identities,
            jump_options,
            folder_options,
            global_proxy,
            block_view,
        } = init;
        let connection = match base.as_ref().map(|session| &session.connection) {
            Some(Connection::Serial(_)) => ConnectionChoice::Serial,
            Some(Connection::Telnet(_)) => ConnectionChoice::Telnet,
            _ => ConnectionChoice::Ssh,
        };
        let serial = base.as_ref().and_then(|session| match &session.connection {
            Connection::Serial(serial) => Some(serial),
            _ => None,
        });
        let telnet = base.as_ref().and_then(|session| match &session.connection {
            Connection::Telnet(telnet) => Some(telnet),
            _ => None,
        });
        let ssh = base.as_ref().and_then(|session| match &session.connection {
            Connection::Ssh(ssh) => Some(ssh),
            _ => None,
        });
        let (auth, save_password, save_passphrase, key_path, cert_path) =
            match ssh.map(|ssh| &ssh.auth) {
                Some(AuthMethod::Password { save }) => (
                    AuthChoice::Password,
                    *save,
                    false,
                    String::new(),
                    String::new(),
                ),
                Some(AuthMethod::PrivateKey {
                    path,
                    cert_path,
                    save_passphrase,
                }) => (
                    AuthChoice::PrivateKey,
                    false,
                    *save_passphrase,
                    path.display().to_string(),
                    cert_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_default(),
                ),
                Some(AuthMethod::Agent) => (
                    AuthChoice::Agent,
                    false,
                    false,
                    String::new(),
                    String::new(),
                ),
                _ => (
                    AuthChoice::Password,
                    true,
                    false,
                    String::new(),
                    String::new(),
                ),
            };
        let startup_commands = base
            .as_ref()
            .map(|session| session.startup.to_multiline())
            .unwrap_or_default();
        let (connect_timeout, keepalive) = base
            .as_ref()
            .and_then(|session| match &session.connection {
                Connection::Ssh(ssh) => {
                    Some((ssh.options.connect_timeout_secs, ssh.options.keepalive_secs))
                }
                Connection::Telnet(telnet) => Some((
                    telnet.options.connect_timeout_secs,
                    telnet.options.keepalive_secs,
                )),
                _ => None,
            })
            .map(|(timeout, keepalive)| (timeout.to_string(), keepalive.to_string()))
            .unwrap_or_else(|| ("15".to_string(), "30".to_string()));
        let telnet_encoding = telnet
            .map(|telnet| telnet.encoding)
            .unwrap_or(TextEncoding::Utf8);
        let serial_port = serial.map(|serial| serial.port.clone()).unwrap_or_default();
        let serial_baud = serial
            .map(|serial| serial.baud_rate)
            .unwrap_or(115_200)
            .to_string();
        let serial_data_bits = serial
            .map(|serial| serial.data_bits)
            .unwrap_or(SerialDataBits::Eight);
        let serial_parity = serial
            .map(|serial| serial.parity)
            .unwrap_or(SerialParity::None);
        let serial_stop_bits = serial
            .map(|serial| serial.stop_bits)
            .unwrap_or(SerialStopBits::One);
        let serial_flow_control = serial
            .map(|serial| serial.flow_control)
            .unwrap_or(SerialFlowControl::None);
        Self::build(
            window,
            cx,
            SessionFormSeed {
                base,
                connection,
                auth,
                save_password: save_password && connection == ConnectionChoice::Ssh,
                save_passphrase,
                key_path,
                cert_path,
                folder_path: folder_path.unwrap_or_default(),
                startup_commands,
                connect_timeout,
                keepalive,
                serial_port,
                serial_baud,
                serial_data_bits,
                serial_parity,
                serial_stop_bits,
                serial_flow_control,
                telnet_encoding,
                identities,
                jump_options,
                folder_options,
                global_proxy,
                block_view,
            },
        )
    }

    /// 当前跳板链的拓扑展示文本：`本机 -> 外层 -> 内层 -> 目标`。
    ///
    /// 只做展示，不参与连接：真正的链路解析与成环校验在目录侧完成，
    /// 这里按表单快照把用户即将保存的路线画出来，避免选错跳板而不自知。
    pub(crate) fn proxy_topology(&self, target: &str) -> String {
        let local = rust_i18n::t!("form.proxy_jump_local").to_string();
        let Some(choice) = self.jump_choice else {
            return format!("{local} -> {target}");
        };
        let mut names = Vec::with_capacity(4);
        let mut visited = HashSet::with_capacity(4);
        let mut cursor = Some(choice);
        while let Some(id) = cursor {
            if !visited.insert(id) {
                break;
            }
            let Some(option) = self.jump_options.iter().find(|option| option.id == id) else {
                break;
            };
            names.push(option.name.clone());
            cursor = option.jump_session_id;
        }
        names.reverse();
        if names.is_empty() {
            format!("{local} -> {target}")
        } else {
            format!("{local} -> {} -> {target}", names.join(" -> "))
        }
    }

    pub(crate) fn apply_folder_selection(
        &mut self,
        path: &str,
        defaults: &transom_core::FolderInheritedDefaults,
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
    ) {
        self.folder_path
            .update(cx, |state, cx| state.set_value(path, window, cx));
        if let Some(port) = defaults.port {
            let current = self.port.read(cx).value();
            if current.is_empty() || current == "22" {
                self.port.update(cx, |state, cx| {
                    state.set_value(port.to_string(), window, cx)
                });
            }
        }
        if let Some(username) = &defaults.username {
            let current = self.username.read(cx).value();
            if current.is_empty() || current == default_username() {
                self.username
                    .update(cx, |state, cx| state.set_value(username, window, cx));
            }
        }
        if let Some(key_path) = &defaults.key_path {
            let current = self.key_path.read(cx).value();
            if current.is_empty() {
                self.auth = AuthChoice::PrivateKey;
                self.key_path.update(cx, |state, cx| {
                    state.set_value(key_path.display().to_string(), window, cx)
                });
            }
        }
        cx.notify();
    }

    fn build(window: &mut Window, cx: &mut Context<impl Sized>, seed: SessionFormSeed) -> Self {
        let SessionFormSeed {
            base,
            connection,
            auth,
            save_password,
            save_passphrase,
            key_path,
            cert_path,
            folder_path,
            startup_commands,
            connect_timeout,
            keepalive,
            serial_port,
            serial_baud,
            serial_data_bits,
            serial_parity,
            serial_stop_bits,
            serial_flow_control,
            telnet_encoding,
            identities,
            jump_options,
            folder_options,
            global_proxy,
            block_view,
        } = seed;
        let ssh = base.as_ref().and_then(|session| match &session.connection {
            Connection::Ssh(ssh) => Some(ssh),
            _ => None,
        });
        let telnet = base.as_ref().and_then(|session| match &session.connection {
            Connection::Telnet(telnet) => Some(telnet),
            _ => None,
        });
        let name = base
            .as_ref()
            .map(|session| session.name.clone())
            .unwrap_or_default();
        let host = ssh
            .map(|ssh| ssh.host.clone())
            .or_else(|| telnet.map(|telnet| telnet.host.clone()))
            .unwrap_or_default();
        let port = ssh
            .map(|ssh| ssh.port)
            .or_else(|| telnet.map(|telnet| telnet.port))
            .unwrap_or(match connection {
                ConnectionChoice::Telnet => 23,
                _ => 22,
            })
            .to_string();
        let username = ssh
            .map(|ssh| ssh.username.clone())
            .unwrap_or_else(default_username);
        // Self 字面量会移走 base，需要读的字段先取出来。
        let identity_choice = base.as_ref().and_then(|session| session.identity_id);
        let jump_choice = ssh
            .and_then(|ssh| ssh.proxy_jump.as_ref())
            .map(|jump| jump.jump_session_id);
        let (
            proxy_choice,
            proxy_host_text,
            proxy_port_text,
            proxy_username_text,
            proxy_password_text,
        ) = match base.as_ref().and_then(|session| session.proxy.as_ref()) {
            None => (
                ProxyChoice::FollowGlobal,
                String::new(),
                "1080".to_string(),
                String::new(),
                String::new(),
            ),
            Some(OutboundProxy::Direct) => (
                ProxyChoice::Direct,
                String::new(),
                "1080".to_string(),
                String::new(),
                String::new(),
            ),
            Some(OutboundProxy::Socks5 { host, port, auth }) => (
                ProxyChoice::Socks5,
                host.clone(),
                port.to_string(),
                auth.as_ref()
                    .map_or_else(String::new, |a| a.username.clone()),
                auth.as_ref()
                    .map_or_else(String::new, |a| a.password.clone()),
            ),
            Some(OutboundProxy::Http { host, port, auth }) => (
                ProxyChoice::Http,
                host.clone(),
                port.to_string(),
                auth.as_ref()
                    .map_or_else(String::new, |a| a.username.clone()),
                auth.as_ref()
                    .map_or_else(String::new, |a| a.password.clone()),
            ),
        };
        let icon = base
            .as_ref()
            .map(|session| session.icon)
            .unwrap_or_default();
        let color = base
            .as_ref()
            .map(|session| session.color)
            .unwrap_or_default();
        let auto_reconnect = ssh.is_some_and(|ssh| ssh.options.auto_reconnect)
            || telnet.is_some_and(|telnet| telnet.options.auto_reconnect);
        // 只有 SSH 会话有 MAC 字段（见 HostAction::wake_for），回填同口径。
        let wol_mac = ssh
            .and_then(|ssh| ssh.options.wol_mac.clone())
            .unwrap_or_default();

        Self {
            base,
            name: text_input(window, cx, &name, &rust_i18n::t!("form.name_placeholder")),
            folder_path: text_input(
                window,
                cx,
                &folder_path,
                &rust_i18n::t!("form.folder_placeholder"),
            ),
            folder_options,
            connection,
            host: text_input(window, cx, &host, &rust_i18n::t!("form.host_placeholder")),
            port: text_input(window, cx, &port, ""),
            username: text_input(
                window,
                cx,
                &username,
                &rust_i18n::t!("form.username_placeholder"),
            ),
            password: password_input(window, cx, &rust_i18n::t!("form.password_placeholder")),
            key_path: text_input(
                window,
                cx,
                &key_path,
                &rust_i18n::t!("form.key_placeholder"),
            ),
            cert_path: text_input(
                window,
                cx,
                &cert_path,
                &rust_i18n::t!("form.cert_placeholder"),
            ),
            passphrase: password_input(window, cx, &rust_i18n::t!("form.passphrase_placeholder")),
            serial_port: text_input(
                window,
                cx,
                &serial_port,
                &rust_i18n::t!("form.serial_port_placeholder"),
            ),
            serial_baud: text_input(window, cx, &serial_baud, "115200"),
            serial_data_bits,
            serial_parity,
            serial_stop_bits,
            serial_flow_control,
            serial_ports: Vec::new(),
            serial_ports_error: None,
            telnet_encoding,
            startup_commands: cx
                .new(|cx| TextareaState::new(window, cx).default_value(startup_commands)),
            connect_timeout: text_input(window, cx, &connect_timeout, "15"),
            keepalive: text_input(window, cx, &keepalive, "30"),
            auth,
            save_password,
            save_passphrase,
            auto_reconnect,
            wol_mac: text_input(
                window,
                cx,
                &wol_mac,
                &rust_i18n::t!("form.wol_mac_placeholder"),
            ),
            identities,
            identity_choice,
            jump_options,
            jump_choice,
            proxy_choice,
            proxy_host: text_input(window, cx, &proxy_host_text, "127.0.0.1"),
            proxy_port: text_input(window, cx, &proxy_port_text, "1080"),
            proxy_username: text_input(
                window,
                cx,
                &proxy_username_text,
                &rust_i18n::t!("form.proxy_username_placeholder"),
            ),
            proxy_password: masked_input(
                window,
                cx,
                &proxy_password_text,
                &rust_i18n::t!("form.proxy_password_placeholder"),
            ),
            global_proxy,
            block_view,
            icon,
            color,
            confirm_delete: false,
            error: None,
        }
    }

    pub fn build_session(&self, cx: &mut Context<impl Sized>) -> Result<BuiltSession, String> {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return Err(rust_i18n::t!("form.error_name").to_string());
        }
        let folder_path = {
            let path = self.folder_path.read(cx).value().trim().to_string();
            (!path.is_empty()).then_some(path)
        };
        let startup = StartupCommands::from_multiline(&self.startup_commands.read(cx).value());
        let (connection, password, key_passphrase) = match self.connection {
            ConnectionChoice::Ssh => {
                let built = self.build_ssh_connection(cx)?;
                (built.connection, built.password, built.key_passphrase)
            }
            ConnectionChoice::Serial => (
                Connection::Serial(self.build_serial_connection(cx)?),
                None,
                None,
            ),
            ConnectionChoice::Telnet => (
                Connection::Telnet(self.build_telnet_connection(cx)?),
                None,
                None,
            ),
        };

        let mut session = self
            .base
            .clone()
            .unwrap_or_else(|| Session::new(name.clone(), connection.clone()));
        session.name = name;
        session.startup = startup;
        session.connection = connection;
        session.identity_id = match self.connection {
            // 只有 SSH 有“用户名 + 认证方式”这组身份概念，其它类型忽略模板。
            ConnectionChoice::Ssh => self.identity_choice,
            _ => None,
        };
        // 出站代理对网络型连接有意义；串口与本地终端没有网络出口。
        session.proxy = match self.connection {
            ConnectionChoice::Ssh | ConnectionChoice::Telnet => self.build_proxy(cx)?,
            ConnectionChoice::Serial => None,
        };
        session.icon = self.icon;
        session.color = self.color;
        session.block_view = self.block_view;
        if self.base.is_none() {
            session.terminal = TerminalProfile::default();
        }
        Ok(BuiltSession {
            session,
            folder_path,
            password,
            key_passphrase,
        })
    }

    /// 组装会话级出站代理：`None` 表示跟随全局。
    fn build_proxy(&self, cx: &mut Context<impl Sized>) -> Result<Option<OutboundProxy>, String> {
        let host = self.proxy_host.read(cx).value().trim().to_string();
        let port_text = self.proxy_port.read(cx).value().trim().to_string();
        let username = self.proxy_username.read(cx).value().trim().to_string();
        let password = self.proxy_password.read(cx).value().to_string();
        let auth = (!username.is_empty() || !password.is_empty())
            .then_some(ProxyAuth { username, password });
        let proxy = match self.proxy_choice {
            ProxyChoice::FollowGlobal => return Ok(None),
            ProxyChoice::Direct => OutboundProxy::Direct,
            ProxyChoice::Socks5 => {
                let port: u16 = port_text
                    .parse()
                    .map_err(|_| rust_i18n::t!("form.error_proxy_port").to_string())?;
                OutboundProxy::Socks5 { host, port, auth }
            }
            ProxyChoice::Http => {
                let port: u16 = port_text
                    .parse()
                    .map_err(|_| rust_i18n::t!("form.error_proxy_port").to_string())?;
                OutboundProxy::Http { host, port, auth }
            }
        };
        proxy.validate()?;
        Ok(Some(proxy))
    }

    fn build_ssh_connection(
        &self,
        cx: &mut Context<impl Sized>,
    ) -> Result<SshConnectionBuild, String> {
        let host = self.host.read(cx).value().trim().to_string();
        if host.is_empty() {
            return Err(rust_i18n::t!("form.error_host").to_string());
        }
        // 选中凭据模板时用户名取自模板：会话里存一份同样的用户名，退回内联
        // 认证时不至于留空；模板的认证方式与口令由引用关系在连接时叠加。
        let template = self
            .identity_choice
            .and_then(|id| self.identities.iter().find(|option| option.id == id));
        let username = match template {
            Some(option) => option.username.clone(),
            None => self.username.read(cx).value().trim().to_string(),
        };
        if username.is_empty() {
            return Err(rust_i18n::t!("form.error_username").to_string());
        }
        let port: u16 = self
            .port
            .read(cx)
            .value()
            .trim()
            .parse()
            .map_err(|_| rust_i18n::t!("form.error_port").to_string())?;
        if port == 0 {
            return Err(rust_i18n::t!("form.error_port").to_string());
        }
        let key_path = self.key_path.read(cx).value().trim().to_string();
        if self.auth == AuthChoice::PrivateKey && key_path.is_empty() {
            return Err(rust_i18n::t!("form.error_key").to_string());
        }
        // 存展开后的绝对路径：与 ssh_config 导入同一口径，连接层不必再理解 `~`。
        let key_path = crate::keys::expand_home(&key_path)
            .to_string_lossy()
            .to_string();
        let mut options = self
            .base
            .as_ref()
            .and_then(|session| match &session.connection {
                Connection::Ssh(ssh) => Some(ssh),
                _ => None,
            })
            .map(|ssh| ssh.options.clone())
            .unwrap_or_default();
        options.auto_reconnect = self.auto_reconnect;
        // MAC 只在非空时校验：留空是"不启用唤醒"的合法表达，不该报错。
        // 填错必须挡下——静默保留旧值会让用户以为改成功了。
        options.wol_mac = match transom_core::normalize_mac(&self.wol_mac.read(cx).value()) {
            Ok(mac) => mac,
            Err(_) => return Err(rust_i18n::t!("form.error_mac").to_string()),
        };
        // 连接参数与键位一样属于"填错就填不进"的数值：越界一律回退存量，
        // 不用报错打断保存流程。
        if let Ok(timeout) = self.connect_timeout.read(cx).value().trim().parse::<u64>()
            && (1..=120).contains(&timeout)
        {
            options.connect_timeout_secs = timeout;
        }
        if let Ok(keepalive) = self.keepalive.read(cx).value().trim().parse::<u64>()
            && keepalive <= 3600
        {
            options.keepalive_secs = keepalive;
        }
        let auth = match self.auth {
            AuthChoice::Password => AuthMethod::Password {
                save: self.save_password,
            },
            AuthChoice::PrivateKey => AuthMethod::PrivateKey {
                path: PathBuf::from(&key_path),
                // 证书路径与私钥同口径展开 `~`；留空表示这台机器不带用户证书。
                cert_path: {
                    let raw = self.cert_path.read(cx).value().trim().to_string();
                    (!raw.is_empty()).then(|| {
                        PathBuf::from(crate::keys::expand_home(&raw).to_string_lossy().to_string())
                    })
                },
                save_passphrase: self.save_passphrase,
            },
            AuthChoice::Agent => AuthMethod::Agent,
        };
        // 引用模板时口令归模板槽位，这里一律不产出，避免同一密码写进两处。
        let password = match auth {
            AuthMethod::Password { .. } if template.is_none() => {
                let value = self.password.read(cx).value().to_string();
                (!value.is_empty()).then(|| Zeroizing::new(value))
            }
            _ => None,
        };
        let key_passphrase = match auth {
            AuthMethod::PrivateKey { .. } if template.is_none() => {
                let value = self.passphrase.read(cx).value().to_string();
                (!value.is_empty()).then(|| Zeroizing::new(value))
            }
            _ => None,
        };
        Ok(SshConnectionBuild {
            connection: Connection::Ssh(SshConnection {
                host,
                port,
                username,
                auth,
                options,
                proxy_jump: self
                    .jump_choice
                    .map(|jump_session_id| ProxyJumpConfig { jump_session_id }),
            }),
            password,
            key_passphrase,
        })
    }

    /// 构建 Telnet 连接：无内建认证，仅主机、端口、编码与选项。
    ///
    /// 超时与保活的取值口径与 SSH 一致：越界一律回退存量，不用报错打断
    /// 保存流程。
    fn build_telnet_connection(
        &self,
        cx: &mut Context<impl Sized>,
    ) -> Result<TelnetConnection, String> {
        let host = self.host.read(cx).value().trim().to_string();
        if host.is_empty() {
            return Err(rust_i18n::t!("form.error_host").to_string());
        }
        let port: u16 = self
            .port
            .read(cx)
            .value()
            .trim()
            .parse()
            .map_err(|_| rust_i18n::t!("form.error_port").to_string())?;
        if port == 0 {
            return Err(rust_i18n::t!("form.error_port").to_string());
        }
        let mut options = self
            .base
            .as_ref()
            .and_then(|session| match &session.connection {
                Connection::Telnet(telnet) => Some(telnet.options.clone()),
                _ => None,
            })
            .unwrap_or_default();
        options.auto_reconnect = self.auto_reconnect;
        if let Ok(timeout) = self.connect_timeout.read(cx).value().trim().parse::<u64>()
            && (1..=120).contains(&timeout)
        {
            options.connect_timeout_secs = timeout;
        }
        if let Ok(keepalive) = self.keepalive.read(cx).value().trim().parse::<u64>()
            && keepalive <= 3600
        {
            options.keepalive_secs = keepalive;
        }
        Ok(TelnetConnection {
            host,
            port,
            encoding: self.telnet_encoding,
            options,
        })
    }

    fn build_serial_connection(
        &self,
        cx: &mut Context<impl Sized>,
    ) -> Result<SerialConnection, String> {
        let port = self.serial_port.read(cx).value().trim().to_string();
        if port.is_empty() {
            return Err(rust_i18n::t!("form.error_serial_port").to_string());
        }
        let baud_rate: u32 = self
            .serial_baud
            .read(cx)
            .value()
            .trim()
            .parse()
            .map_err(|_| rust_i18n::t!("form.error_serial_baud").to_string())?;
        if baud_rate == 0 {
            return Err(rust_i18n::t!("form.error_serial_baud").to_string());
        }
        Ok(SerialConnection {
            port,
            baud_rate,
            data_bits: self.serial_data_bits,
            parity: self.serial_parity,
            stop_bits: self.serial_stop_bits,
            flow_control: self.serial_flow_control,
        })
    }
}

pub(crate) fn text_input(
    window: &mut Window,
    cx: &mut Context<impl Sized>,
    value: &str,
    placeholder: &str,
) -> Entity<InputState> {
    let value = value.to_string();
    let placeholder = placeholder.to_string();
    cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(value)
            .placeholder(placeholder)
    })
}

fn password_input(
    window: &mut Window,
    cx: &mut Context<impl Sized>,
    placeholder: &str,
) -> Entity<InputState> {
    masked_input(window, cx, "", placeholder)
}

/// 带初值的掩码输入框；用于回填已保存在配置里的代理口令。
fn masked_input(
    window: &mut Window,
    cx: &mut Context<impl Sized>,
    value: &str,
    placeholder: &str,
) -> Entity<InputState> {
    let value = value.to_string();
    let placeholder = placeholder.to_string();
    cx.new(|cx| {
        InputState::new(window, cx)
            .masked(true)
            .default_value(value)
            .placeholder(placeholder)
    })
}

fn default_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}
