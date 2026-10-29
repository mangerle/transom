//! 会话表单初始状态配置、跳板链路拓扑计算与文件夹默认值应用。

use super::types::{
    AuthChoice, ConnectionChoice, ProxyChoice, SessionForm, SessionFormInit, SessionFormSeed,
};
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::{AppContext, Context, Entity, Window};
use std::collections::HashSet;
use transom_core::{
    AuthMethod, Connection, OutboundProxy, SerialDataBits, SerialFlowControl, SerialParity,
    SerialStopBits, TextEncoding,
};

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
            Some(Connection::Mosh(_)) => ConnectionChoice::Mosh,
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
        let mosh = base.as_ref().and_then(|session| match &session.connection {
            Connection::Mosh(mosh) => Some(mosh),
            _ => None,
        });
        let auth_source = ssh.map(|s| &s.auth).or_else(|| mosh.map(|m| &m.auth));
        let (auth, save_password, save_passphrase, key_path, cert_path) = match auth_source {
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
                Connection::Mosh(mosh) => Some((
                    mosh.options.connect_timeout_secs,
                    mosh.options.keepalive_secs,
                )),
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
        let mosh = base.as_ref().and_then(|session| match &session.connection {
            Connection::Mosh(mosh) => Some(mosh),
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
            .or_else(|| mosh.map(|mosh| mosh.host.clone()))
            .or_else(|| telnet.map(|telnet| telnet.host.clone()))
            .unwrap_or_default();
        let port = ssh
            .map(|ssh| ssh.port)
            .or_else(|| mosh.map(|mosh| mosh.ssh_port))
            .or_else(|| telnet.map(|telnet| telnet.port))
            .unwrap_or(match connection {
                ConnectionChoice::Telnet => 23,
                _ => 22,
            })
            .to_string();
        let username = ssh
            .map(|ssh| ssh.username.clone())
            .or_else(|| mosh.map(|mosh| mosh.username.clone()))
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
            || mosh.is_some_and(|mosh| mosh.options.auto_reconnect)
            || telnet.is_some_and(|telnet| telnet.options.auto_reconnect);
        let agent_forward = ssh.is_some_and(|ssh| ssh.options.agent_forward)
            || mosh.is_some_and(|mosh| mosh.options.agent_forward);
        // 只有 SSH 会话有 MAC 字段（见 HostAction::wake_for），回填同口径。
        let wol_mac = ssh
            .and_then(|ssh| ssh.options.wol_mac.clone())
            .unwrap_or_default();
        let mosh_server_path_val =
            mosh.map_or_else(|| "mosh-server".to_string(), |m| m.server_path.clone());
        let mosh_server_port_val = mosh
            .and_then(|m| m.server_port)
            .map_or_else(String::new, |p| p.to_string());

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
            agent_forward,
            mosh_server_path: text_input(window, cx, &mosh_server_path_val, "mosh-server"),
            mosh_server_port: text_input(window, cx, &mosh_server_port_val, "留空自动分配"),
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
