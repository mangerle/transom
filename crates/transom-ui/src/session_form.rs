use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{
    AuthMethod, Connection, SerialConnection, SerialDataBits, SerialFlowControl, SerialParity,
    SerialPortDescriptor, SerialStopBits, Session, SshConnection, StartupCommands, TerminalProfile,
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
}

pub(crate) struct SessionForm {
    pub base: Option<Session>,
    pub name: Entity<InputState>,
    /// 以 `/` 分隔的文件夹路径，保存时解析为稳定的文件夹节点。
    pub folder_path: Entity<InputState>,
    pub connection: ConnectionChoice,
    pub host: Entity<InputState>,
    pub port: Entity<InputState>,
    pub username: Entity<InputState>,
    pub password: Entity<InputState>,
    pub key_path: Entity<InputState>,
    pub passphrase: Entity<InputState>,
    pub serial_port: Entity<InputState>,
    pub serial_baud: Entity<InputState>,
    pub serial_data_bits: SerialDataBits,
    pub serial_parity: SerialParity,
    pub serial_stop_bits: SerialStopBits,
    pub serial_flow_control: SerialFlowControl,
    pub serial_ports: Vec<SerialPortDescriptor>,
    pub serial_ports_error: Option<String>,
    pub startup_commands: Entity<TextareaState>,
    pub auth: AuthChoice,
    pub save_password: bool,
    pub save_passphrase: bool,
    pub confirm_delete: bool,
    pub error: Option<String>,
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
    folder_path: String,
    startup_commands: String,
    serial_port: String,
    serial_baud: String,
    serial_data_bits: SerialDataBits,
    serial_parity: SerialParity,
    serial_stop_bits: SerialStopBits,
    serial_flow_control: SerialFlowControl,
}

impl SessionForm {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<impl Sized>,
        base: Option<Session>,
        folder_path: Option<String>,
    ) -> Self {
        let connection = match base.as_ref().map(|session| &session.connection) {
            Some(Connection::Serial(_)) => ConnectionChoice::Serial,
            _ => ConnectionChoice::Ssh,
        };
        let serial = base.as_ref().and_then(|session| match &session.connection {
            Connection::Serial(serial) => Some(serial),
            _ => None,
        });
        let ssh = base.as_ref().and_then(|session| match &session.connection {
            Connection::Ssh(ssh) => Some(ssh),
            _ => None,
        });
        let (auth, save_password, save_passphrase, key_path) = match ssh.map(|ssh| &ssh.auth) {
            Some(AuthMethod::Password { save }) => {
                (AuthChoice::Password, *save, false, String::new())
            }
            Some(AuthMethod::PrivateKey {
                path,
                save_passphrase,
            }) => (
                AuthChoice::PrivateKey,
                false,
                *save_passphrase,
                path.display().to_string(),
            ),
            Some(AuthMethod::Agent) => (AuthChoice::Agent, false, false, String::new()),
            _ => (AuthChoice::Password, true, false, String::new()),
        };
        let startup_commands = base
            .as_ref()
            .map(|session| session.startup.to_multiline())
            .unwrap_or_default();
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
                folder_path: folder_path.unwrap_or_default(),
                startup_commands,
                serial_port,
                serial_baud,
                serial_data_bits,
                serial_parity,
                serial_stop_bits,
                serial_flow_control,
            },
        )
    }

    fn build(window: &mut Window, cx: &mut Context<impl Sized>, seed: SessionFormSeed) -> Self {
        let SessionFormSeed {
            base,
            connection,
            auth,
            save_password,
            save_passphrase,
            key_path,
            folder_path,
            startup_commands,
            serial_port,
            serial_baud,
            serial_data_bits,
            serial_parity,
            serial_stop_bits,
            serial_flow_control,
        } = seed;
        let ssh = base.as_ref().and_then(|session| match &session.connection {
            Connection::Ssh(ssh) => Some(ssh),
            _ => None,
        });
        let name = base
            .as_ref()
            .map(|session| session.name.clone())
            .unwrap_or_default();
        let host = ssh.map(|ssh| ssh.host.clone()).unwrap_or_default();
        let port = ssh.map(|ssh| ssh.port).unwrap_or(22).to_string();
        let username = ssh
            .map(|ssh| ssh.username.clone())
            .unwrap_or_else(default_username);

        Self {
            base,
            name: text_input(window, cx, &name, &rust_i18n::t!("form.name_placeholder")),
            folder_path: text_input(
                window,
                cx,
                &folder_path,
                &rust_i18n::t!("form.folder_placeholder"),
            ),
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
            startup_commands: cx
                .new(|cx| TextareaState::new(window, cx).default_value(startup_commands)),
            auth,
            save_password,
            save_passphrase,
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
        };

        let mut session = self
            .base
            .clone()
            .unwrap_or_else(|| Session::new(name.clone(), connection.clone()));
        session.name = name;
        session.startup = startup;
        session.connection = connection;
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

    fn build_ssh_connection(
        &self,
        cx: &mut Context<impl Sized>,
    ) -> Result<SshConnectionBuild, String> {
        let host = self.host.read(cx).value().trim().to_string();
        if host.is_empty() {
            return Err(rust_i18n::t!("form.error_host").to_string());
        }
        let username = self.username.read(cx).value().trim().to_string();
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
        let options = self
            .base
            .as_ref()
            .and_then(|session| match &session.connection {
                Connection::Ssh(ssh) => Some(ssh),
                _ => None,
            })
            .map(|ssh| ssh.options.clone())
            .unwrap_or_default();
        let auth = match self.auth {
            AuthChoice::Password => AuthMethod::Password {
                save: self.save_password,
            },
            AuthChoice::PrivateKey => AuthMethod::PrivateKey {
                path: PathBuf::from(&key_path),
                save_passphrase: self.save_passphrase,
            },
            AuthChoice::Agent => AuthMethod::Agent,
        };
        let password = match auth {
            AuthMethod::Password { .. } => {
                let value = self.password.read(cx).value().to_string();
                (!value.is_empty()).then(|| Zeroizing::new(value))
            }
            _ => None,
        };
        let key_passphrase = match auth {
            AuthMethod::PrivateKey { .. } => {
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
            }),
            password,
            key_passphrase,
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

fn text_input(
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
    let placeholder = placeholder.to_string();
    cx.new(|cx| {
        InputState::new(window, cx)
            .masked(true)
            .placeholder(placeholder)
    })
}

fn default_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}
