use super::types::{
    AuthChoice, ConnectionChoice, ProxyChoice, SessionFormInit, SessionFormSeed,
    SessionTargetDefaults,
};
use transom_core::{
    AuthMethod, Connection, FtpSecurity, OutboundProxy, RdpColorDepth, RdpPerformance, RdpSecurity,
    SerialDataBits, SerialFlowControl, SerialParity, SerialStopBits, TextEncoding,
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
        Some(Connection::Ftp(_)) => ConnectionChoice::Ftp,
        Some(Connection::Rdp(_)) => ConnectionChoice::Rdp,
        _ => ConnectionChoice::Ssh,
    };
    let local = local_defaults(base.as_ref());
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
    let (connect_timeout, keepalive) = connection_timeouts(base.as_ref());
    let telnet_encoding = telnet
        .map(|telnet| telnet.encoding)
        .unwrap_or(TextEncoding::Utf8);
    let (ftp_security, ftp_passive) = ftp_defaults(base.as_ref());
    let rdp = rdp_defaults(base.as_ref());
    let serial = serial_defaults(serial);
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
        serial_port: serial.port,
        serial_baud: serial.baud,
        serial_data_bits: serial.data_bits,
        serial_parity: serial.parity,
        serial_stop_bits: serial.stop_bits,
        serial_flow_control: serial.flow_control,
        telnet_encoding,
        ftp_security,
        ftp_passive,
        rdp_domain: rdp.domain,
        rdp_security: rdp.security,
        rdp_color_depth: rdp.color_depth,
        rdp_performance: rdp.performance,
        rdp_dynamic_resolution: rdp.dynamic_resolution,
        rdp_width: rdp.width,
        rdp_height: rdp.height,
        rdp_shared_dir: rdp.shared_dir,
        rdp_audio: rdp.audio,
        rdp_clipboard: rdp.clipboard,
        local_working_dir: local.working_dir,
        local_shell: local.shell,
        local_shell_args: local.shell_args,
        local_command: local.command,
        local_run_as_admin: local.run_as_admin,
        local_env: local.env,
        available_shells,
        identities,
        forwards,
        jump_options,
        folder_options,
        global_proxy,
        block_view,
    }
}

struct LocalDefaults {
    working_dir: String,
    shell: String,
    shell_args: String,
    command: String,
    run_as_admin: bool,
    env: String,
}

struct SerialDefaults {
    port: String,
    baud: String,
    data_bits: SerialDataBits,
    parity: SerialParity,
    stop_bits: SerialStopBits,
    flow_control: SerialFlowControl,
}

struct RdpDefaults {
    domain: String,
    security: RdpSecurity,
    color_depth: RdpColorDepth,
    performance: RdpPerformance,
    dynamic_resolution: bool,
    width: String,
    height: String,
    shared_dir: String,
    audio: bool,
    clipboard: bool,
}

fn local_defaults(base: Option<&transom_core::Session>) -> LocalDefaults {
    let local = base.and_then(|session| match &session.connection {
        Connection::Local(local) => Some(local),
        _ => None,
    });
    LocalDefaults {
        working_dir: local
            .and_then(|value| {
                value
                    .working_dir
                    .as_ref()
                    .map(|path| path.display().to_string())
            })
            .unwrap_or_default(),
        shell: local
            .and_then(|value| value.shell.clone())
            .unwrap_or_default(),
        shell_args: local
            .map(|value| value.shell_args.join(" "))
            .unwrap_or_default(),
        command: local
            .and_then(|value| value.command.as_ref().map(|command| command.join(" ")))
            .unwrap_or_default(),
        run_as_admin: local.is_some_and(|value| value.run_as_admin),
        env: local
            .map(|value| value.env_to_multiline())
            .unwrap_or_default(),
    }
}

fn serial_defaults(serial: Option<&transom_core::SerialConnection>) -> SerialDefaults {
    SerialDefaults {
        port: serial.map(|value| value.port.clone()).unwrap_or_default(),
        baud: serial
            .map(|value| value.baud_rate)
            .unwrap_or(115_200)
            .to_string(),
        data_bits: serial
            .map(|value| value.data_bits)
            .unwrap_or(SerialDataBits::Eight),
        parity: serial
            .map(|value| value.parity)
            .unwrap_or(SerialParity::None),
        stop_bits: serial
            .map(|value| value.stop_bits)
            .unwrap_or(SerialStopBits::One),
        flow_control: serial
            .map(|value| value.flow_control)
            .unwrap_or(SerialFlowControl::None),
    }
}

fn connection_timeouts(base: Option<&transom_core::Session>) -> (String, String) {
    base.and_then(|session| match &session.connection {
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
    .unwrap_or_else(|| ("15".to_string(), "30".to_string()))
}

fn ftp_defaults(base: Option<&transom_core::Session>) -> (FtpSecurity, bool) {
    base.and_then(|session| match &session.connection {
        Connection::Ftp(ftp) => Some((ftp.security, ftp.passive)),
        _ => None,
    })
    .unwrap_or((FtpSecurity::Plain, true))
}

fn rdp_defaults(base: Option<&transom_core::Session>) -> RdpDefaults {
    base.and_then(|session| match &session.connection {
        Connection::Rdp(rdp) => Some(RdpDefaults {
            domain: rdp.domain.clone().unwrap_or_default(),
            security: rdp.security,
            color_depth: rdp.color_depth,
            performance: rdp.performance,
            dynamic_resolution: rdp.dynamic_resolution,
            width: rdp
                .desktop_size
                .map(|size| size.width)
                .unwrap_or(1920)
                .to_string(),
            height: rdp
                .desktop_size
                .map(|size| size.height)
                .unwrap_or(1080)
                .to_string(),
            shared_dir: rdp.shared_dir.clone().unwrap_or_default(),
            audio: rdp.audio,
            clipboard: rdp.clipboard,
        }),
        _ => None,
    })
    .unwrap_or(RdpDefaults {
        domain: String::new(),
        security: RdpSecurity::Negotiate,
        color_depth: RdpColorDepth::Depth32,
        performance: RdpPerformance::Lan,
        dynamic_resolution: true,
        width: "1920".to_string(),
        height: "1080".to_string(),
        shared_dir: String::new(),
        audio: true,
        clipboard: true,
    })
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
    let ftp = base.as_ref().and_then(|session| match &session.connection {
        Connection::Ftp(ftp) => Some(ftp),
        _ => None,
    });
    let rdp = base.as_ref().and_then(|session| match &session.connection {
        Connection::Rdp(rdp) => Some(rdp),
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
        .or_else(|| ftp.map(|ftp| ftp.host.clone()))
        .or_else(|| rdp.map(|rdp| rdp.host.clone()))
        .unwrap_or_default();
    let port = ssh
        .map(|ssh| ssh.port)
        .or_else(|| mosh.map(|mosh| mosh.ssh_port))
        .or_else(|| telnet.map(|telnet| telnet.port))
        .or_else(|| ftp.map(|ftp| ftp.port))
        .or_else(|| rdp.map(|rdp| rdp.port))
        .unwrap_or(match connection {
            ConnectionChoice::Telnet => 23,
            ConnectionChoice::Ftp => 21,
            ConnectionChoice::Rdp => 3389,
            _ => 22,
        })
        .to_string();
    let username = ssh
        .map(|ssh| ssh.username.clone())
        .or_else(|| mosh.map(|mosh| mosh.username.clone()))
        .or_else(|| ftp.map(|ftp| ftp.username.clone()))
        .or_else(|| rdp.map(|rdp| rdp.username.clone()))
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
