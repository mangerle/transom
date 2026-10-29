//! 会话表单配置验证与模型组装逻辑。

use super::types::{
    AuthChoice, BuiltSession, ConnectionChoice, IdentityOption, ProxyChoice, SessionForm,
    SshConnectionBuild,
};
use gpui_kit::Context;
use std::path::PathBuf;
use transom_core::{
    AuthMethod, Connection, OutboundProxy, ProxyAuth, ProxyJumpConfig, SerialConnection, Session,
    SshConnection, StartupCommands, TelnetConnection, TerminalProfile,
};
use zeroize::Zeroizing;

impl SessionForm {
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
            ConnectionChoice::Mosh => {
                let built = self.build_mosh_connection(cx)?;
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
            // 只有 SSH 与 Mosh 有“用户名 + 认证方式”这组身份概念，其它类型忽略模板。
            ConnectionChoice::Ssh | ConnectionChoice::Mosh => self.identity_choice,
            _ => None,
        };
        // 出站代理对网络型连接有意义；串口与本地终端没有网络出口。
        session.proxy = match self.connection {
            ConnectionChoice::Ssh | ConnectionChoice::Telnet | ConnectionChoice::Mosh => {
                self.build_proxy(cx)?
            }
            ConnectionChoice::Serial => None,
        };
        session.icon = self.icon;
        session.color = self.color;
        session.block_view = self.block_view;
        // 表单保存是一次明确的用户修改：刷新时间戳供多端合并裁决。
        session.touch();
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
        let template = self.identity_choice.and_then(|id| {
            self.identities
                .iter()
                .find(|option: &&IdentityOption| option.id == id)
        });
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
        options.agent_forward = self.agent_forward;
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

    fn build_mosh_connection(
        &self,
        cx: &mut Context<impl Sized>,
    ) -> Result<SshConnectionBuild, String> {
        let host = self.host.read(cx).value().trim().to_string();
        if host.is_empty() {
            return Err(rust_i18n::t!("form.error_host").to_string());
        }
        let port = match self.port.read(cx).value().trim().parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => 22,
        };
        let username = self.username.read(cx).value().trim().to_string();
        if username.is_empty() {
            return Err(rust_i18n::t!("form.error_username").to_string());
        }

        let key_path = self.key_path.read(cx).value().trim().to_string();
        if self.auth == AuthChoice::PrivateKey && key_path.is_empty() {
            return Err(rust_i18n::t!("form.error_key").to_string());
        }

        let mut options = self
            .base
            .as_ref()
            .and_then(|session| match &session.connection {
                Connection::Mosh(mosh) => Some(mosh.options.clone()),
                Connection::Ssh(ssh) => Some(ssh.options.clone()),
                _ => None,
            })
            .unwrap_or_default();
        options.auto_reconnect = self.auto_reconnect;
        options.agent_forward = self.agent_forward;

        let auth = match self.auth {
            AuthChoice::Password => AuthMethod::Password {
                save: self.save_password,
            },
            AuthChoice::PrivateKey => AuthMethod::PrivateKey {
                path: PathBuf::from(&key_path),
                cert_path: None,
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

        let server_path = {
            let p = self.mosh_server_path.read(cx).value().trim().to_string();
            if p.is_empty() {
                "mosh-server".to_string()
            } else {
                p
            }
        };
        let server_port = self
            .mosh_server_port
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|&p| p > 0);

        Ok(SshConnectionBuild {
            connection: Connection::Mosh(transom_core::MoshConnection {
                host,
                ssh_port: port,
                username,
                server_port,
                server_path,
                auth,
                options,
            }),
            password,
            key_passphrase,
        })
    }
}
