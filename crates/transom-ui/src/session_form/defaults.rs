use super::types::{
    AuthChoice, ConnectionChoice, ProxyChoice, SessionFormInit, SessionFormSeed,
    SessionTargetDefaults,
};
use transom_core::{
    AuthMethod, Connection, OutboundProxy, SerialDataBits, SerialFlowControl, SerialParity,
    SerialStopBits, TextEncoding,
};

/// 由认证方式推导表单默认值：选择、保存开关、私钥与证书路径。
fn auth_defaults(
    auth_source: Option<&transom_core::AuthMethod>,
) -> (AuthChoice, bool, bool, String, String) {
    match auth_source {
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
    }
}

/// 从初始化上下文推导表单默认值，与控件创建解耦。
pub(super) fn seed_from_init(init: SessionFormInit) -> SessionFormSeed {
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
        Some(Connection::Local(_)) => ConnectionChoice::Local,
        Some(Connection::Serial(_)) => ConnectionChoice::Serial,
        Some(Connection::Telnet(_)) => ConnectionChoice::Telnet,
        Some(Connection::Mosh(_)) => ConnectionChoice::Mosh,
        _ => ConnectionChoice::Ssh,
    };
    let local = base.as_ref().and_then(|session| match &session.connection {
        Connection::Local(local) => Some(local),
        _ => None,
    });
    let local_working_dir = local
        .and_then(|l| l.working_dir.as_ref().map(|p| p.display().to_string()))
        .unwrap_or_default();
    let local_shell = local.and_then(|l| l.shell.clone()).unwrap_or_default();
    let local_shell_args = local.map(|l| l.shell_args.join(" ")).unwrap_or_default();
    let local_command = local
        .and_then(|l| l.command.as_ref().map(|c| c.join(" ")))
        .unwrap_or_default();
    let local_run_as_admin = local.map(|l| l.run_as_admin).unwrap_or(false);
    let local_env = local.map(|l| l.env_to_multiline()).unwrap_or_default();
    let available_shells = transom_platform::shell::discover_shells();
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
    let (auth, save_password, save_passphrase, key_path, cert_path) = auth_defaults(auth_source);
    let startup_commands = base
        .as_ref()
        .map(|session| session.startup.to_multiline())
        .unwrap_or_default();
    let forwards = base
        .as_ref()
        .map(|session| session.forwards.clone())
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
    let target = target_defaults(&base, connection);
    SessionFormSeed {
        base,
        target,
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
        local_working_dir,
        local_shell,
        local_shell_args,
        local_command,
        local_run_as_admin,
        local_env,
        available_shells,
        identities,
        forwards,
        jump_options,
        folder_options,
        global_proxy,
        block_view,
    }
}

/// 由基础会话推导名称、地址、代理、外观与连接选项。
fn target_defaults(
    base: &Option<transom_core::Session>,
    connection: ConnectionChoice,
) -> SessionTargetDefaults {
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
        .unwrap_or_default();
    // Self 字面量会移走 base，需要读的字段先取出来。
    let identity_choice = base.as_ref().and_then(|session| session.identity_id);
    let jump_choice = ssh
        .and_then(|ssh| ssh.proxy_jump.as_ref())
        .map(|jump| jump.jump_session_id);
    let (proxy_choice, proxy_host_text, proxy_port_text, proxy_username_text, proxy_password_text) =
        proxy_defaults(base);

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

    SessionTargetDefaults {
        name,
        host,
        port,
        username,
        identity_choice,
        jump_choice,
        proxy_choice,
        proxy_host: proxy_host_text,
        proxy_port: proxy_port_text,
        proxy_username: proxy_username_text,
        proxy_password: proxy_password_text,
        icon,
        color,
        auto_reconnect,
        agent_forward,
        wol_mac,
        mosh_server_path: mosh_server_path_val,
        mosh_server_port: mosh_server_port_val,
    }
}

/// 由基础会话推导代理表单默认值：类型、主机、端口、用户名与密码。
fn proxy_defaults(
    base: &Option<transom_core::Session>,
) -> (ProxyChoice, String, String, String, String) {
    match base.as_ref().and_then(|session| session.proxy.as_ref()) {
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
    }
}
