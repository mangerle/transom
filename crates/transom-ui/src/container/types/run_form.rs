//! 运行容器表单：端口、卷、环境变量等文本解析。

use super::*;

/// 表单里的单行输入：占位符走 i18n，可选预填值。
fn run_input(
    placeholder: &'static str,
    default: Option<String>,
    window: &mut Window,
    cx: &mut Context<crate::app::AppShell>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let state = InputState::new(window, cx).placeholder(rust_i18n::t!(placeholder).to_string());
        match default {
            Some(value) => state.default_value(value),
            None => state,
        }
    })
}

/// 运行容器表单：把镜像跑起来的最小必要参数。
///
/// 设计原理（Why）：`docker run` 的参数面极大，界面只开放真正决定「能不能跑起来」
/// 的六项（名称、镜像、命令、端口、挂载、环境变量、重启策略），其余走镜像与引擎
/// 默认；文本框按 `docker run` 的常见写法收集，再解析成强类型 spec 交给引擎层校验，
/// 这样用户写 `8080:80/tcp` 与 `8080:80` 都能得到同一个映射。
pub(crate) struct ContainerRunForm {
    /// 表单归属的工作区标签。
    pub(crate) tab_id: TabId,
    pub(crate) name: Entity<InputState>,
    pub(crate) image: Entity<InputState>,
    pub(crate) command: Entity<InputState>,
    pub(crate) ports: Entity<InputState>,
    pub(crate) binds: Entity<InputState>,
    pub(crate) env: Entity<InputState>,
    pub(crate) health: Entity<InputState>,
    pub(crate) health_interval: Entity<InputState>,
    pub(crate) memory: Entity<InputState>,
    pub(crate) cpus: Entity<InputState>,
    pub(crate) restart: Option<String>,
    /// 是否展开高级选项（命令、环境变量、资源限制、重启策略等）。
    pub(crate) advanced_open: bool,
    /// 校验或引擎返回的错误提示，展示在表单底部。
    pub(crate) error: Option<String>,
}

impl ContainerRunForm {
    /// 脏判定用的文本快照：取消时与打开基线逐项比对。
    pub(crate) fn text_values(&self, cx: &App) -> Vec<String> {
        [
            &self.name,
            &self.image,
            &self.command,
            &self.ports,
            &self.binds,
            &self.env,
            &self.health,
            &self.health_interval,
            &self.memory,
            &self.cpus,
        ]
        .into_iter()
        .map(|input| input.read(cx).value().to_string())
        .collect()
    }

    pub(crate) fn new(
        tab_id: TabId,
        image_hint: Option<String>,
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
    ) -> Self {
        Self {
            tab_id,
            name: run_input("container.run.name_placeholder", None, window, cx),
            image: run_input("container.run.image_placeholder", image_hint, window, cx),
            command: run_input("container.run.command_placeholder", None, window, cx),
            ports: run_input("container.run.ports_placeholder", None, window, cx),
            binds: run_input("container.run.binds_placeholder", None, window, cx),
            env: run_input("container.run.env_placeholder", None, window, cx),
            health: run_input("container.run.health_placeholder", None, window, cx),
            health_interval: run_input(
                "container.run.health_interval_placeholder",
                None,
                window,
                cx,
            ),
            memory: run_input("container.run.memory_placeholder", None, window, cx),
            cpus: run_input("container.run.cpus_placeholder", None, window, cx),
            restart: None,
            advanced_open: false,
            error: None,
        }
    }

    /// 从已有容器详情生成运行表单，用于「克隆容器」时反向预填。
    pub(crate) fn from_details(
        tab_id: TabId,
        details: &transom_core::ContainerDetails,
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
    ) -> Self {
        let name_hint = if details.name.is_empty() {
            None
        } else {
            Some(format!("{}-clone", details.name))
        };
        let image_hint = if details.image.is_empty() {
            None
        } else {
            Some(details.image.clone())
        };
        let command_hint = if details.command.is_empty() {
            None
        } else {
            Some(details.command.clone())
        };
        let ports_hint = {
            let parts: Vec<String> = details
                .ports
                .iter()
                .filter_map(|port| {
                    port.public_port.map(|pub_p| {
                        let proto = if port.protocol.is_empty() || port.protocol == "tcp" {
                            String::new()
                        } else {
                            format!("/{}", port.protocol)
                        };
                        format!("{pub_p}:{}{proto}", port.private_port)
                    })
                })
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(", "))
            }
        };
        let binds_hint = {
            let parts: Vec<String> = details
                .mounts
                .iter()
                .filter(|m| !m.source.is_empty() && !m.destination.is_empty())
                .map(|m| {
                    let ro = if !m.rw { ":ro" } else { "" };
                    format!("{}:{}{ro}", m.source, m.destination)
                })
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(", "))
            }
        };
        let env_hint = {
            let parts: Vec<String> = details
                .env
                .iter()
                .filter(|e| !e.starts_with("PATH="))
                .cloned()
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(", "))
            }
        };
        Self {
            tab_id,
            name: run_input("container.run.name_placeholder", name_hint, window, cx),
            image: run_input("container.run.image_placeholder", image_hint, window, cx),
            command: run_input("container.run.command_placeholder", command_hint, window, cx),
            ports: run_input("container.run.ports_placeholder", ports_hint, window, cx),
            binds: run_input("container.run.binds_placeholder", binds_hint, window, cx),
            env: run_input("container.run.env_placeholder", env_hint, window, cx),
            health: run_input("container.run.health_placeholder", None, window, cx),
            health_interval: run_input(
                "container.run.health_interval_placeholder",
                None,
                window,
                cx,
            ),
            memory: run_input("container.run.memory_placeholder", None, window, cx),
            cpus: run_input("container.run.cpus_placeholder", None, window, cx),
            restart: details.restart_policy.clone(),
            advanced_open: false,
            error: None,
        }
    }

    /// 读取输入并解析成 spec；解析与校验错误直接返回中文提示。
    ///
    /// # Errors
    /// 端口/挂载写法非法、spec 校验不通过时返回可直接展示的中文原因。
    pub(crate) fn to_spec(&self, cx: &App) -> Result<ContainerCreateSpec, String> {
        let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let mut spec = ContainerCreateSpec {
            name: read(&self.name),
            image: read(&self.image),
            command: split_argv(&read(&self.command)),
            env: split_list(&read(&self.env)),
            ports: parse_ports(&read(&self.ports))?,
            binds: split_list(&read(&self.binds)),
            restart: self.restart.clone(),
            health_cmd: Some(read(&self.health)),
            // 间隔留空或写非数字都按引擎默认处理：这不是必须填对的字段，
            // 报错打断提交流程不值得，真正的非法值（0 秒）由 spec 校验拦下。
            health_interval_secs: read(&self.health_interval).parse::<u64>().ok(),
            memory_mb: read(&self.memory).parse::<u64>().ok(),
            nano_cpus: parse_cores(&read(&self.cpus)),
        };
        spec.validate().map_err(|error| error.to_string())?;
        Ok(spec)
    }
}

/// 逗号分隔列表：卷名、环境变量等都用它，空项直接丢弃。
pub(crate) fn split_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// 把「核数」输入换算成 Docker 的 `NanoCpus`（1e-9 核）。
///
/// 留空、非数字、非正数都按「不限制」处理：这一项不是必填，报错打断提交流程
/// 不值得，真正的量级错误由 spec 校验拦下。
pub(crate) fn parse_cores(text: &str) -> Option<u64> {
    let cores: f64 = text.trim().parse().ok()?;
    if !cores.is_finite() || cores <= 0.0 {
        return None;
    }
    Some((cores * 1_000_000_000.0).round() as u64)
}

/// 命令按空白切分为 argv；本工具不支持带引号的命令，避免把 Shell 语义搬进界面。
pub(crate) fn split_argv(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

/// 解析端口映射：`8080:80`、`8080:80/udp`、`127.0.0.1:8080:80` 之外的写法一律拒绝。
///
/// 返回的 `host` 为 0 或协议不认识都属于非法输入，错误信息直接可展示。
pub(crate) fn parse_ports(text: &str) -> Result<Vec<ContainerPortPublish>, String> {
    let mut items = Vec::new();
    for chunk in split_list(text) {
        let (mapping, protocol) = match chunk.split_once('/') {
            Some((mapping, protocol)) => (mapping, protocol.to_ascii_lowercase()),
            None => (chunk.as_str(), "tcp".to_string()),
        };
        let mut parts = mapping.split(':').collect::<Vec<_>>();
        if parts.len() != 2 {
            return Err(rust_i18n::t!("container.run.port_format", item = chunk).to_string());
        }
        let host = parts[0].parse::<u16>().map_err(|_| {
            rust_i18n::t!("container.run.port_number", value = parts[0].to_string()).to_string()
        })?;
        let container = parts[1].parse::<u16>().map_err(|_| {
            rust_i18n::t!("container.run.port_number", value = parts[1].to_string()).to_string()
        })?;
        parts.clear();
        items.push(ContainerPortPublish {
            host,
            container,
            protocol,
        });
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::{parse_cores, parse_ports, split_argv, split_list};

    #[test]
    fn test_split_helpers() {
        assert_eq!(split_list("a=1, b=2, , c=3"), vec!["a=1", "b=2", "c=3"]);
        assert_eq!(split_argv("nginx -g 'daemon off;'"), vec!["nginx", "-g", "'daemon", "off;'"]);
    }

    #[test]
    fn test_parse_ports() {
        let ports = parse_ports("8080:80, 8443:443/udp").expect("端口映射应合法");
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].host, 8080);
        assert_eq!(ports[0].container, 80);
        assert_eq!(ports[0].protocol, "tcp");
        assert_eq!(ports[1].host, 8443);
        assert_eq!(ports[1].container, 443);
        assert_eq!(ports[1].protocol, "udp");
    }

    #[test]
    fn test_parse_cores() {
        assert_eq!(parse_cores("1.5"), Some(1_500_000_000));
        assert_eq!(parse_cores("0"), None);
        assert_eq!(parse_cores("-1"), None);
        assert_eq!(parse_cores("invalid"), None);
    }
}
