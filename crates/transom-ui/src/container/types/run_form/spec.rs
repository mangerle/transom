//! 运行容器规格解析与 Docker CLI 命令生成。
//!
//! 设计原理（Why）：
//! - 将图形表单的结构化输入精确映射为 Docker 引擎底层的 `ContainerCreateSpec` 契约；
//! - 动态生成完整、可复制执行的 `docker run` 命令字符串，实现所见即所得。

use super::infer::{parse_cores, split_argv, split_list};
use super::{RunContainerFormState, RunPreset};
use gpui_kit::component::input::InputState;
use gpui_kit::{App, Entity};
use std::collections::HashMap;
use transom_core::{ContainerCreateSpec, ContainerPortPublish};

impl RunContainerFormState {
    /// 生成实时 Docker Run CLI 预览命令。
    pub(crate) fn generate_preview_command(&self, cx: &App) -> String {
        let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let name = read(&self.name);
        let image = read(&self.image);
        let image_display = if image.is_empty() { "<image>" } else { &image };
        let command = read(&self.command);

        let mut parts = Vec::new();
        parts.push("docker run".to_string());

        match self.preset {
            RunPreset::Daemon => {
                parts.push("  -d".to_string());
            }
            RunPreset::Interactive => {
                parts.push("  --rm -it".to_string());
            }
        }

        if !name.is_empty() {
            parts.push(format!("  --name {name}"));
        }

        let network_mode = read(&self.network_mode);
        if !network_mode.is_empty() {
            parts.push(format!("  --network {network_mode}"));
        }
        let hostname = read(&self.hostname);
        if !hostname.is_empty() {
            parts.push(format!("  --hostname {hostname}"));
        }
        let user = read(&self.user);
        if !user.is_empty() {
            parts.push(format!("  --user {user}"));
        }
        let working_dir = read(&self.working_dir);
        if !working_dir.is_empty() {
            parts.push(format!("  --workdir {working_dir}"));
        }
        if self.read_only_rootfs {
            parts.push("  --read-only".to_string());
        }
        if self.privileged {
            parts.push("  --privileged".to_string());
        }
        if self.init {
            parts.push("  --init".to_string());
        }
        if self.auto_remove && self.preset != RunPreset::Interactive {
            parts.push("  --rm".to_string());
        }
        for cap in split_list(&read(&self.cap_add)) {
            parts.push(format!("  --cap-add {cap}"));
        }
        for cap in split_list(&read(&self.cap_drop)) {
            parts.push(format!("  --cap-drop {cap}"));
        }
        for dns in split_list(&read(&self.dns)) {
            parts.push(format!("  --dns {dns}"));
        }
        for (key, value) in parse_labels(&read(&self.labels)).unwrap_or_default() {
            parts.push(format!("  --label {key}={value}"));
        }

        for p in &self.ports {
            let h = read(&p.host_port);
            let c = read(&p.container_port);
            if !c.is_empty() {
                let proto = if p.protocol == "udp" { "/udp" } else { "" };
                if !h.is_empty() {
                    parts.push(format!("  -p {h}:{c}{proto}"));
                } else {
                    parts.push(format!("  -p {c}{proto}"));
                }
            }
        }

        for v in &self.volumes {
            let h = read(&v.host_path);
            let c = read(&v.container_path);
            if !h.is_empty() && !c.is_empty() {
                let ro = if v.read_only { ":ro" } else { "" };
                parts.push(format!("  -v {h}:{c}{ro}"));
            }
        }

        for e in &self.envs {
            let k = read(&e.key);
            let val = read(&e.value);
            if !k.is_empty() {
                parts.push(format!("  -e {k}=\"{val}\""));
            }
        }

        if let Some(ref r) = self.restart
            && !r.is_empty()
            && self.preset == RunPreset::Daemon
        {
            parts.push(format!("  --restart {r}"));
        }

        let mem = read(&self.memory);
        if !mem.is_empty() {
            parts.push(format!("  -m {mem}m"));
        }

        let cpus = read(&self.cpus);
        if !cpus.is_empty() {
            parts.push(format!("  --cpus {cpus}"));
        }

        parts.push(format!("  {image_display}"));

        if !command.is_empty() {
            parts.push(format!("  {command}"));
        }

        parts.join(" \\\n")
    }

    /// 读取结构化输入并解析成标准 ContainerCreateSpec。
    pub(crate) fn to_spec(&self, cx: &App) -> Result<ContainerCreateSpec, String> {
        let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();

        let mut ports = Vec::new();
        for p in &self.ports {
            let h_str = read(&p.host_port);
            let c_str = read(&p.container_port);
            if c_str.is_empty() {
                continue;
            }
            let container = c_str.parse::<u16>().map_err(|_| {
                rust_i18n::t!("container.run.port_number", value = c_str).to_string()
            })?;
            let host = if h_str.is_empty() {
                container
            } else {
                h_str.parse::<u16>().map_err(|_| {
                    rust_i18n::t!("container.run.port_number", value = h_str).to_string()
                })?
            };
            ports.push(ContainerPortPublish {
                host,
                container,
                protocol: p.protocol.clone(),
            });
        }

        let mut binds = Vec::new();
        for v in &self.volumes {
            let h = read(&v.host_path);
            let c = read(&v.container_path);
            if !h.is_empty() && !c.is_empty() {
                let ro = if v.read_only { ":ro" } else { "" };
                binds.push(format!("{h}:{c}{ro}"));
            }
        }

        let mut env = Vec::new();
        for e in &self.envs {
            let k = read(&e.key);
            let val = read(&e.value);
            if !k.is_empty() {
                env.push(format!("{k}={val}"));
            }
        }

        let mut spec = ContainerCreateSpec {
            name: read(&self.name),
            image: read(&self.image),
            command: split_argv(&read(&self.command)),
            env,
            ports,
            binds,
            restart: self.restart.clone(),
            health_cmd: Some(read(&self.health)),
            health_interval_secs: read(&self.health_interval).parse::<u64>().ok(),
            memory_mb: read(&self.memory).parse::<u64>().ok(),
            nano_cpus: parse_cores(&read(&self.cpus)),
            network_mode: non_empty(read(&self.network_mode)),
            hostname: non_empty(read(&self.hostname)),
            user: non_empty(read(&self.user)),
            working_dir: non_empty(read(&self.working_dir)),
            labels: parse_labels(&read(&self.labels))?,
            cap_add: split_list(&read(&self.cap_add)),
            cap_drop: split_list(&read(&self.cap_drop)),
            dns: split_list(&read(&self.dns)),
            auto_remove: self.auto_remove || self.preset == RunPreset::Interactive,
            read_only_rootfs: self.read_only_rootfs,
            privileged: self.privileged,
            init: self.init,
            tty: self.preset == RunPreset::Interactive,
            ..ContainerCreateSpec::default()
        };

        spec.validate().map_err(|error| error.to_string())?;
        Ok(spec)
    }
}

/// 空白输入统一视为未填写。
fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// 解析 `KEY=VALUE,KEY2=VALUE2` 形式的容器标签。
fn parse_labels(raw: &str) -> Result<HashMap<String, String>, String> {
    let mut labels = HashMap::new();
    for entry in split_list(raw) {
        let Some((key, value)) = entry.split_once('=') else {
            return Err(rust_i18n::t!("container.run.label_invalid", entry = entry).to_string());
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(rust_i18n::t!("container.run.label_empty_key", entry = entry).to_string());
        }
        labels.insert(key.to_string(), value.trim().to_string());
    }
    Ok(labels)
}

#[cfg(test)]
mod tests {
    use super::parse_labels;

    #[test]
    fn labels_parse_key_value_pairs() {
        let labels = parse_labels("team=infra,env=dev").expect("标签必须可解析");
        assert_eq!(labels.get("team").map(String::as_str), Some("infra"));
        assert_eq!(labels.get("env").map(String::as_str), Some("dev"));
        assert!(parse_labels("missing_equals").is_err());
        assert!(parse_labels("=value").is_err());
    }
}
