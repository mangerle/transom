use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{AuthMethod, Connection, Session, SshConnection, TerminalProfile};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthChoice {
    Password,
    PrivateKey,
    Agent,
}

pub(crate) struct SessionForm {
    pub base: Option<Session>,
    pub name: Entity<InputState>,
    /// 以 `/` 分隔的文件夹路径，保存时解析为稳定的文件夹节点。
    pub folder_path: Entity<InputState>,
    pub host: Entity<InputState>,
    pub port: Entity<InputState>,
    pub username: Entity<InputState>,
    pub password: Entity<InputState>,
    pub key_path: Entity<InputState>,
    pub passphrase: Entity<InputState>,
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

/// 表单初始值聚合，避免构造函数平铺过多参数。
struct SessionFormSeed {
    base: Option<Session>,
    auth: AuthChoice,
    save_password: bool,
    save_passphrase: bool,
    key_path: String,
    folder_path: String,
}

impl SessionForm {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<impl Sized>,
        base: Option<Session>,
        folder_path: Option<String>,
    ) -> Self {
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
        Self::build(
            window,
            cx,
            SessionFormSeed {
                base,
                auth,
                save_password,
                save_passphrase,
                key_path,
                folder_path: folder_path.unwrap_or_default(),
            },
        )
    }

    fn build(window: &mut Window, cx: &mut Context<impl Sized>, seed: SessionFormSeed) -> Self {
        let SessionFormSeed {
            base,
            auth,
            save_password,
            save_passphrase,
            key_path,
            folder_path,
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

        let base_ssh = self
            .base
            .as_ref()
            .and_then(|session| match &session.connection {
                Connection::Ssh(ssh) => Some(ssh),
                _ => None,
            });
        let options = base_ssh.map(|ssh| ssh.options.clone()).unwrap_or_default();
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

        let mut session = self.base.clone().unwrap_or_else(|| {
            Session::new(
                name.clone(),
                Connection::Ssh(SshConnection {
                    host: host.clone(),
                    port,
                    username: username.clone(),
                    auth: auth.clone(),
                    options: options.clone(),
                }),
            )
        });
        session.name = name;
        let folder_path = {
            let path = self.folder_path.read(cx).value().trim().to_string();
            (!path.is_empty()).then_some(path)
        };
        session.connection = Connection::Ssh(SshConnection {
            host,
            port,
            username,
            auth: auth.clone(),
            options,
        });
        if self.base.is_none() {
            session.terminal = TerminalProfile::default();
        }

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

        Ok(BuiltSession {
            session,
            folder_path,
            password,
            key_passphrase,
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
