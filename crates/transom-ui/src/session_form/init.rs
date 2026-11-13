//! 会话表单初始状态配置、跳板链路拓扑计算与文件夹默认值应用。

use super::defaults::seed_from_init;
use super::types::{
    AuthChoice, SessionForm, SessionFormInit, SessionFormSection, SessionFormSeed,
    SessionForwardDraft, SessionForwardKind, SessionTargetDefaults,
};
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::{AppContext, Context, Entity, Window};
use std::collections::HashSet;

/// 表单创建的输入控件集合：把二十余个实体句柄从装配字面量中收拢。
pub(super) struct SessionFormInputs {
    name: Entity<InputState>,
    folder_path: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    key_path: Entity<InputState>,
    cert_path: Entity<InputState>,
    passphrase: Entity<InputState>,
    serial_port: Entity<InputState>,
    serial_baud: Entity<InputState>,
    startup_commands: Entity<TextareaState>,
    local_working_dir: Entity<InputState>,
    local_shell: Entity<InputState>,
    local_shell_args: Entity<InputState>,
    local_command: Entity<InputState>,
    local_env: Entity<TextareaState>,
    connect_timeout: Entity<InputState>,
    keepalive: Entity<InputState>,
    mosh_server_path: Entity<InputState>,
    mosh_server_port: Entity<InputState>,
    wol_mac: Entity<InputState>,
    proxy_host: Entity<InputState>,
    proxy_port: Entity<InputState>,
    proxy_username: Entity<InputState>,
    proxy_password: Entity<InputState>,
    forward_draft: SessionForwardDraft,
}

impl SessionFormInputs {
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<impl Sized>,
        seed: &SessionFormSeed,
    ) -> Self {
        let target = &seed.target;
        let forward_draft = SessionForwardDraft {
            kind: SessionForwardKind::Local,
            name: text_input(window, cx, "", &rust_i18n::t!("forward.name")),
            bind_host: text_input(window, cx, "127.0.0.1", &rust_i18n::t!("forward.bind_host")),
            bind_port: number_input(window, cx, "", "8080", (1.0, 65535.0, 1.0)),
            target_host: text_input(
                window,
                cx,
                "127.0.0.1",
                &rust_i18n::t!("forward.target_host"),
            ),
            target_port: number_input(window, cx, "", "80", (1.0, 65535.0, 1.0)),
            error: None,
        };
        Self {
            name: text_input(
                window,
                cx,
                &target.name,
                &rust_i18n::t!("form.name_placeholder"),
            ),
            folder_path: text_input(
                window,
                cx,
                &seed.folder_path,
                &rust_i18n::t!("form.group_none"),
            ),
            host: text_input(window, cx, &target.host, "192.168.1.100"),
            port: number_input(window, cx, &target.port, "22", (1.0, 65535.0, 1.0)),
            username: text_input(
                window,
                cx,
                &target.username,
                &rust_i18n::t!("form.username_placeholder"),
            ),
            password: password_input(
                window,
                cx,
                &rust_i18n::t!("form.password_optional_placeholder"),
            ),
            key_path: text_input(
                window,
                cx,
                &seed.key_path,
                &rust_i18n::t!("form.key_placeholder"),
            ),
            cert_path: text_input(
                window,
                cx,
                &seed.cert_path,
                &rust_i18n::t!("form.cert_placeholder"),
            ),
            passphrase: password_input(
                window,
                cx,
                &rust_i18n::t!("form.passphrase_optional_placeholder"),
            ),
            serial_port: text_input(
                window,
                cx,
                &seed.serial_port,
                &rust_i18n::t!("form.serial_port_placeholder"),
            ),
            serial_baud: text_input(window, cx, &seed.serial_baud, "115200"),
            startup_commands: cx.new(|cx| {
                TextareaState::new(window, cx).default_value(seed.startup_commands.clone())
            }),
            local_working_dir: text_input(
                window,
                cx,
                &seed.local_working_dir,
                &rust_i18n::t!("form.local.working_dir_placeholder"),
            ),
            local_shell: text_input(
                window,
                cx,
                &seed.local_shell,
                &rust_i18n::t!("form.local.shell_placeholder"),
            ),
            local_shell_args: text_input(
                window,
                cx,
                &seed.local_shell_args,
                &rust_i18n::t!("form.local.shell_args_placeholder"),
            ),
            local_command: text_input(
                window,
                cx,
                &seed.local_command,
                &rust_i18n::t!("form.local.command_placeholder"),
            ),
            local_env: cx
                .new(|cx| TextareaState::new(window, cx).default_value(seed.local_env.clone())),
            connect_timeout: number_input(
                window,
                cx,
                &seed.connect_timeout,
                "15",
                (1.0, 120.0, 1.0),
            ),
            keepalive: number_input(window, cx, &seed.keepalive, "30", (0.0, 3600.0, 5.0)),
            mosh_server_path: text_input(window, cx, &target.mosh_server_path, "mosh-server"),
            mosh_server_port: text_input(
                window,
                cx,
                &target.mosh_server_port,
                &rust_i18n::t!("form.mosh.port_placeholder"),
            ),
            wol_mac: text_input(
                window,
                cx,
                &target.wol_mac,
                &rust_i18n::t!("form.wol_mac_placeholder"),
            ),
            proxy_host: text_input(window, cx, &target.proxy_host, "127.0.0.1"),
            proxy_port: number_input(window, cx, &target.proxy_port, "1080", (1.0, 65535.0, 1.0)),
            proxy_username: text_input(
                window,
                cx,
                &target.proxy_username,
                &rust_i18n::t!("form.proxy_username_placeholder"),
            ),
            proxy_password: masked_input(
                window,
                cx,
                &target.proxy_password,
                &rust_i18n::t!("form.proxy_password_placeholder"),
            ),
            forward_draft,
        }
    }
}

impl SessionForm {
    pub fn new(window: &mut Window, cx: &mut Context<impl Sized>, init: SessionFormInit) -> Self {
        Self::build(window, cx, seed_from_init(init))
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
        cx: &mut Context<impl Sized + 'static>,
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
            if current.is_empty() {
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
        let inputs = SessionFormInputs::new(window, cx, &seed);
        let SessionFormSeed {
            base,
            target,
            connection,
            auth,
            save_password,
            save_passphrase,
            key_path: _,
            cert_path: _,
            folder_path: _,
            startup_commands: _,
            connect_timeout: _,
            keepalive: _,
            serial_port: _,
            serial_baud: _,
            serial_data_bits,
            serial_parity,
            serial_stop_bits,
            serial_flow_control,
            telnet_encoding,
            local_working_dir: _,
            local_shell: _,
            local_shell_args: _,
            local_command: _,
            local_run_as_admin,
            local_env: _,
            available_shells,
            identities,
            forwards,
            jump_options,
            folder_options,
            global_proxy,
            block_view,
        } = seed;
        let SessionTargetDefaults {
            identity_choice,
            jump_choice,
            proxy_choice,
            icon,
            color,
            auto_reconnect,
            agent_forward,
            ..
        } = target;
        Self {
            base,
            section: SessionFormSection::Basic,
            name: inputs.name,
            folder_path: inputs.folder_path,
            folder_open: false,
            icon_open: false,
            folder_options,
            connection,
            host: inputs.host,
            port: inputs.port,
            username: inputs.username,
            password: inputs.password,
            key_path: inputs.key_path,
            cert_path: inputs.cert_path,
            passphrase: inputs.passphrase,
            serial_port: inputs.serial_port,
            serial_baud: inputs.serial_baud,
            serial_data_bits,
            serial_parity,
            serial_stop_bits,
            serial_flow_control,
            serial_ports: Vec::new(),
            serial_ports_error: None,
            telnet_encoding,
            startup_commands: inputs.startup_commands,
            local_working_dir: inputs.local_working_dir,
            local_shell: inputs.local_shell,
            local_shell_args: inputs.local_shell_args,
            local_command: inputs.local_command,
            local_run_as_admin,
            local_env: inputs.local_env,
            available_shells,
            connect_timeout: inputs.connect_timeout,
            keepalive: inputs.keepalive,
            auth,
            save_password,
            save_passphrase,
            auto_reconnect,
            agent_forward,
            mosh_server_path: inputs.mosh_server_path,
            mosh_server_port: inputs.mosh_server_port,
            wol_mac: inputs.wol_mac,
            identities,
            identity_choice,
            identity_edit: None,
            forwards,
            forward_draft: inputs.forward_draft,
            jump_options,
            jump_choice,
            proxy_choice,
            proxy_host: inputs.proxy_host,
            proxy_port: inputs.proxy_port,
            proxy_username: inputs.proxy_username,
            proxy_password: inputs.proxy_password,
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

fn number_input(
    window: &mut Window,
    cx: &mut Context<impl Sized>,
    value: &str,
    placeholder: &str,
    (min, max, step): (f64, f64, f64),
) -> Entity<InputState> {
    let value = value.to_string();
    let placeholder = placeholder.to_string();
    cx.new(|cx| {
        InputState::new(window, cx)
            .min(min)
            .max(max)
            .step(step)
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
