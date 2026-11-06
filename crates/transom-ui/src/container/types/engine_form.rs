//! 引擎连接表单的状态与校验。

use super::*;
use transom_core::TlsConfig;

/// SSH 连接附加字段的表单状态。
///
/// 独立于主输入框（主输入框承载 host），这三个字段只在 SSH 档位显示。
pub(crate) struct SshFormFields {
    /// SSH 端口输入框；留空表示使用默认 22。
    pub(crate) port: Entity<InputState>,
    /// 登录用户名；留空表示沿用当前系统用户名。
    pub(crate) user: Entity<InputState>,
    /// 私钥文件路径；留空表示按 SSH 默认规则查找。
    pub(crate) identity_file: Entity<InputState>,
    /// 登录密码输入框；留空表示走私钥或 Agent 免密。
    pub(crate) password: Entity<InputState>,
    /// 是否使用 sudo 提权执行容器命令。
    pub(crate) use_sudo: bool,
    /// 独立的 sudo 提权密码输入框；留空表示复用登录密码。
    pub(crate) sudo_password: Entity<InputState>,
}

/// TCP TLS 附加字段的表单状态。
///
/// 只在 TCP 档位且用户勾选 TLS 时才展示。
pub(crate) struct TlsFormFields {
    /// 是否启用 TLS。
    pub(crate) enabled: bool,
    /// CA 证书文件路径。
    pub(crate) ca_cert: Entity<InputState>,
    /// 客户端证书文件路径。
    pub(crate) client_cert: Entity<InputState>,
    /// 客户端私钥文件路径。
    pub(crate) client_key: Entity<InputState>,
}

/// 容器表单状态。
pub(crate) struct ContainerFormState {
    pub(crate) editing: Option<EngineProfileId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) folder_id: Option<transom_core::FolderId>,
    pub(crate) kind: ContainerEngineKind,
    pub(crate) icon: SessionIcon,
    pub(crate) color: SessionColor,
    pub(crate) endpoint_choice: EndpointChoice,
    /// 主输入框：根据 endpoint_choice 承载不同含义（socket 路径、URL、SSH host 等）。
    pub(crate) endpoint_value: Entity<InputState>,
    /// CLI 参数；只有自定义 CLI 类型使用，单独保存避免再把引号交给字符串拆分。
    pub(crate) cli_args: Entity<InputState>,
    /// SSH 附加字段；始终存在于状态中，只在档位为 SSH 时渲染。
    pub(crate) ssh: SshFormFields,
    /// TCP TLS 字段；始终存在于状态中，只在档位为 TCP 时渲染。
    pub(crate) tls: TlsFormFields,
    /// 测试连接结果提示。
    pub(crate) test_status: Option<String>,
    pub(crate) test_ok: bool,
    pub(crate) test_request: Option<ContainerRequestId>,
}

impl ContainerFormState {
    /// 脏判定用的文本快照：名称、主端点值与各附加字段。
    pub(crate) fn text_values(&self, cx: &App) -> Vec<String> {
        vec![
            self.name.read(cx).value().to_string(),
            self.endpoint_value.read(cx).value().to_string(),
            self.cli_args.read(cx).value().to_string(),
            self.ssh.port.read(cx).value().to_string(),
            self.ssh.user.read(cx).value().to_string(),
            self.ssh.identity_file.read(cx).value().to_string(),
            self.ssh.password.read(cx).value().to_string(),
            self.ssh.sudo_password.read(cx).value().to_string(),
            self.tls.ca_cert.read(cx).value().to_string(),
            self.tls.client_cert.read(cx).value().to_string(),
            self.tls.client_key.read(cx).value().to_string(),
        ]
    }

    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
        profile: Option<&ContainerEngineProfile>,
        initial_folder: Option<transom_core::FolderId>,
    ) -> Self {
        let name_value = profile
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        let engine = profile
            .map(|profile| profile.engine.clone())
            .unwrap_or_else(|| ContainerEngineRef::auto(ContainerEngineKind::Docker));
        let requested_choice = EndpointChoice::from_endpoint(&engine.endpoint);
        let available = EndpointChoice::available();
        let endpoint_choice = if available.contains(&requested_choice) {
            requested_choice
        } else {
            EndpointChoice::Auto
        };

        // 从端点中提取各字段回填值
        let endpoint_main_value = EndpointChoice::value_for(&engine.endpoint);
        let (
            cli_args_value,
            ssh_port,
            ssh_user,
            ssh_identity_file,
            ssh_password,
            ssh_use_sudo,
            ssh_sudo_password,
            tls_enabled,
            tls_ca,
            tls_cert,
            tls_key,
        ) = match &engine.endpoint {
            ContainerEndpoint::Cli { program, args } => {
                let _ = program;
                (
                    args.join(" "),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    false,
                    String::new(),
                    false,
                    String::new(),
                    String::new(),
                    String::new(),
                )
            }
            ContainerEndpoint::Ssh {
                port,
                user,
                identity_file,
                password,
                use_sudo,
                sudo_password,
                ..
            } => (
                String::new(),
                port.map(|p| p.to_string()).unwrap_or_default(),
                user.clone().unwrap_or_default(),
                identity_file.clone().unwrap_or_default(),
                password.clone().unwrap_or_default(),
                *use_sudo,
                sudo_password.clone().unwrap_or_default(),
                false,
                String::new(),
                String::new(),
                String::new(),
            ),
            ContainerEndpoint::Tcp { tls, .. } => {
                let tls = tls.as_ref();
                (
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    false,
                    String::new(),
                    tls.is_some(),
                    tls.map(|t| t.ca_cert.clone()).unwrap_or_default(),
                    tls.map(|t| t.client_cert.clone()).unwrap_or_default(),
                    tls.map(|t| t.client_key.clone()).unwrap_or_default(),
                )
            }
            _ => (
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                false,
                String::new(),
                false,
                String::new(),
                String::new(),
                String::new(),
            ),
        };

        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name_value)
                .placeholder(rust_i18n::t!("container.form.name_placeholder").to_string())
        });
        let endpoint = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(endpoint_main_value)
                .placeholder(rust_i18n::t!(endpoint_choice.placeholder_key()).to_string())
        });
        let cli_args = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(cli_args_value)
                .placeholder(rust_i18n::t!("container.form.cli_args_placeholder").to_string())
        });
        let ssh_port_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(ssh_port)
                .placeholder("22".to_string())
        });
        let ssh_user_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(ssh_user)
                .placeholder(rust_i18n::t!("container.form.ssh_user_placeholder").to_string())
        });
        let ssh_identity_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(ssh_identity_file)
                .placeholder(rust_i18n::t!("container.form.ssh_identity_placeholder").to_string())
        });
        let ssh_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(ssh_password)
                .masked(true)
                .placeholder(rust_i18n::t!("container.form.ssh_password_placeholder").to_string())
        });
        let ssh_sudo_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(ssh_sudo_password)
                .masked(true)
                .placeholder(
                    rust_i18n::t!("container.form.ssh_sudo_password_placeholder").to_string(),
                )
        });
        let tls_ca_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(tls_ca)
                .placeholder(rust_i18n::t!("container.form.tls_ca_placeholder").to_string())
        });
        let tls_cert_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(tls_cert)
                .placeholder(rust_i18n::t!("container.form.tls_cert_placeholder").to_string())
        });
        let tls_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(tls_key)
                .placeholder(rust_i18n::t!("container.form.tls_key_placeholder").to_string())
        });

        Self {
            editing: profile.map(|profile| profile.id),
            name,
            folder_id: profile
                .map(|profile| profile.folder_id)
                .unwrap_or(initial_folder),
            kind: engine.kind,
            icon: profile
                .map(|profile| profile.icon)
                .unwrap_or(SessionIcon::Auto),
            color: profile
                .map(|profile| profile.color)
                .unwrap_or(SessionColor::None),
            endpoint_choice,
            endpoint_value: endpoint,
            cli_args,
            ssh: SshFormFields {
                port: ssh_port_input,
                user: ssh_user_input,
                identity_file: ssh_identity_input,
                password: ssh_password_input,
                use_sudo: ssh_use_sudo,
                sudo_password: ssh_sudo_password_input,
            },
            tls: TlsFormFields {
                enabled: tls_enabled,
                ca_cert: tls_ca_input,
                client_cert: tls_cert_input,
                client_key: tls_key_input,
            },
            test_status: None,
            test_ok: false,
            test_request: None,
        }
    }

    /// 构造保存/测试用的连接配置。
    ///
    /// # Errors
    /// 名称为空、需要端点值却留空，或端点在当前平台不可用时返回中文提示。
    pub(crate) fn to_profile(&self, cx: &App) -> Result<ContainerEngineProfile, String> {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return Err(rust_i18n::t!("container.form.name_required").to_string());
        }
        let main_value = self.endpoint_value.read(cx).value().trim().to_string();

        // 构造端点，SSH 和 TCP TLS 需要额外处理
        let endpoint = match self.endpoint_choice {
            EndpointChoice::Cli => {
                let args = self.cli_args.read(cx).value().trim().to_string();
                let full = if args.is_empty() {
                    main_value.clone()
                } else {
                    format!("{main_value} {args}")
                };
                if full.trim().is_empty() {
                    return Err(rust_i18n::t!("container.form.endpoint_required").to_string());
                }
                self.endpoint_choice.to_endpoint(&full)
            }
            EndpointChoice::Ssh => {
                if main_value.is_empty() {
                    return Err(rust_i18n::t!("container.form.endpoint_required").to_string());
                }
                let port_str = self.ssh.port.read(cx).value().trim().to_string();
                let port = if port_str.is_empty() {
                    None
                } else {
                    match port_str.parse::<u16>() {
                        Ok(p) => Some(p),
                        Err(_) => {
                            return Err(
                                rust_i18n::t!("container.form.ssh_port_invalid").to_string()
                            );
                        }
                    }
                };
                let user_str = self.ssh.user.read(cx).value().trim().to_string();
                let user = if user_str.is_empty() {
                    None
                } else {
                    Some(user_str)
                };
                let identity_str = self.ssh.identity_file.read(cx).value().trim().to_string();
                let identity_file = if identity_str.is_empty() {
                    None
                } else {
                    Some(identity_str)
                };
                let pwd_str = self.ssh.password.read(cx).value().trim().to_string();
                let password = if pwd_str.is_empty() {
                    None
                } else {
                    Some(pwd_str)
                };
                let sudo_pwd_str = self.ssh.sudo_password.read(cx).value().trim().to_string();
                let sudo_password = if sudo_pwd_str.is_empty() {
                    None
                } else {
                    Some(sudo_pwd_str)
                };
                ContainerEndpoint::Ssh {
                    host: main_value,
                    port,
                    user,
                    identity_file,
                    password,
                    use_sudo: self.ssh.use_sudo,
                    sudo_password,
                }
            }
            EndpointChoice::Tcp => {
                if main_value.is_empty() {
                    return Err(rust_i18n::t!("container.form.endpoint_required").to_string());
                }
                let tls = if self.tls.enabled {
                    let ca_cert = self.tls.ca_cert.read(cx).value().trim().to_string();
                    let client_cert = self.tls.client_cert.read(cx).value().trim().to_string();
                    let client_key = self.tls.client_key.read(cx).value().trim().to_string();
                    Some(TlsConfig {
                        ca_cert,
                        client_cert,
                        client_key,
                    })
                } else {
                    None
                };
                ContainerEndpoint::Tcp {
                    url: main_value,
                    tls,
                }
            }
            choice => {
                if choice.needs_value() && main_value.is_empty() {
                    return Err(rust_i18n::t!("container.form.endpoint_required").to_string());
                }
                choice.to_endpoint(&main_value)
            }
        };

        endpoint
            .ensure_supported()
            .map_err(|reason| reason.to_string())?;
        let engine = ContainerEngineRef::explicit(self.kind, endpoint);
        let mut profile = ContainerEngineProfile::new(name, engine);
        if let Some(id) = self.editing {
            profile.id = id;
        }
        profile.folder_id = self.folder_id;
        profile.icon = self.icon;
        profile.color = self.color;
        Ok(profile)
    }
}
