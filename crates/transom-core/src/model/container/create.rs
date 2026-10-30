//! 创建容器（把镜像跑起来）的请求参数与输入校验。
//!
//! 设计原理（Why）：这是容器管理工具的主干动作之一（Portainer 的 Deploy
//! container、Docker Desktop 的 Run）。参数保持扁平且可序列化，界面拼一次就
//! 能进命令/事件与备份文档；校验集中在这里，进入网络边界前挡掉非法值，避免
//! 引擎返回难以定位的 400。

use serde::{Deserialize, Serialize};

/// 端口映射请求：宿主端口 -> 容器端口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerPortPublish {
    pub host: u16,
    pub container: u16,
    /// `tcp` 或 `udp`；其他值在校验阶段拒绝。
    #[serde(default = "default_tcp")]
    pub protocol: String,
}

fn default_tcp() -> String {
    "tcp".to_string()
}

/// 创建容器的请求参数：界面只暴露常用项，其余走镜像与引擎默认。
///
/// 设计原理（Why）：容器管理工具的主干动作之一就是「把镜像跑起来」
/// （Portainer 的 Deploy container、Docker Desktop 的 Run）。参数保持扁平
/// 且可序列化，是为了让它能直接进命令/事件与备份文档，不需要在界面层再拼
/// 一次 JSON；校验集中在 [`ContainerCreateSpec::validate`]，进入网络边界前
/// 就把非法值挡掉，避免引擎返回难以定位的 400。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerCreateSpec {
    /// 容器名；留空由引擎生成随机名。
    #[serde(default)]
    pub name: String,
    /// 镜像引用，必填。
    #[serde(default)]
    pub image: String,
    /// 启动命令（argv）；留空使用镜像默认。不经过 Shell，避免注入。
    #[serde(default)]
    pub command: Vec<String>,
    /// 追加环境变量（`KEY=VALUE`）。
    #[serde(default)]
    pub env: Vec<String>,
    /// 端口映射列表。
    #[serde(default)]
    pub ports: Vec<ContainerPortPublish>,
    /// 挂载：`卷名:容器路径[:ro]` 或 `宿主路径:容器路径[:ro]`。
    #[serde(default)]
    pub binds: Vec<String>,
    /// 重启策略：`no`/`on-failure`/`always`/`unless-stopped`；留空按 `no`。
    #[serde(default)]
    pub restart: Option<String>,
}

impl ContainerCreateSpec {
    /// 允许的重启策略集合；引擎另有 `always:N` 之类的计数形式，界面不开放。
    const RESTART_POLICIES: [&'static str; 4] = ["no", "on-failure", "always", "unless-stopped"];

    /// 校验输入并顺手归一化（去空白、小写协议）。
    ///
    /// 返回中文原因，供界面直接展示；不返回引擎错误，因为这一步还没发请求。
    ///
    /// # Errors
    /// 镜像为空、名称非法、端口为 0、协议不认识、挂载路径不是绝对路径或含
    /// `..`、重启策略不在允许集合时返回对应原因。
    pub fn validate(&mut self) -> Result<(), String> {
        self.name = self.name.trim().to_string();
        self.image = self.image.trim().to_string();
        if self.image.is_empty() {
            return Err("必须填写镜像引用".to_string());
        }
        if !self.name.is_empty() && !is_valid_container_name(&self.name) {
            return Err(
                "容器名只能包含字母、数字、点、下划线和短横线，且必须以字母或数字开头".to_string(),
            );
        }
        for publish in &mut self.ports {
            if publish.host == 0 || publish.container == 0 {
                return Err("端口映射的宿主端口与容器端口都不能为 0".to_string());
            }
            publish.protocol = publish.protocol.trim().to_ascii_lowercase();
            if publish.protocol != "tcp" && publish.protocol != "udp" {
                return Err(format!(
                    "端口协议只支持 tcp/udp，收到: {}",
                    publish.protocol
                ));
            }
        }
        for bind in &mut self.binds {
            *bind = bind.trim().to_string();
            // 留空表示「这一栏没填」，由后面的 retain 丢弃，不该当成非法输入报错。
            if bind.is_empty() {
                continue;
            }
            validate_bind(bind)?;
        }
        for env in &mut self.env {
            *env = env.trim().to_string();
            if env.is_empty() {
                continue;
            }
            if !env.contains('=') {
                return Err(format!("环境变量必须是 KEY=VALUE 形式: {env}"));
            }
        }
        self.binds.retain(|bind| !bind.is_empty());
        self.env.retain(|env| !env.is_empty());
        if let Some(restart) = self.restart.as_deref()
            && !Self::RESTART_POLICIES.contains(&restart)
        {
            return Err(format!("不支持的重启策略: {restart}"));
        }
        Ok(())
    }
}

/// 容器名规则与引擎一致：首字符是字母或数字，其后允许 `.`/`_`/`-`。
pub fn is_valid_container_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// 挂载串校验：容器路径必须是绝对路径且不含 `..`，与归档接口的口径一致。
fn validate_bind(bind: &str) -> Result<(), String> {
    let mut parts = bind.split(':');
    let (Some(source), Some(target)) = (parts.next(), parts.next()) else {
        return Err(format!("挂载必须写成 源:容器路径 形式: {bind}"));
    };
    if source.is_empty() || target.is_empty() {
        return Err(format!("挂载的源与目标都不能为空: {bind}"));
    }
    if !target.starts_with('/') {
        return Err(format!("容器内挂载路径必须是绝对路径: {target}"));
    }
    if target.split('/').any(|segment| segment == "..")
        || source.split('/').any(|segment| segment == "..")
    {
        return Err(format!("挂载路径不能包含 .. : {bind}"));
    }
    if let Some(mode) = parts.next()
        && mode != "ro"
        && mode != "rw"
    {
        return Err(format!("挂载模式只支持 ro/rw: {mode}"));
    }
    if parts.next().is_some() {
        return Err(format!("挂载格式不正确: {bind}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ContainerCreateSpec, ContainerPortPublish};

    fn spec() -> ContainerCreateSpec {
        ContainerCreateSpec {
            name: "web".to_string(),
            image: "nginx:latest".to_string(),
            command: vec![
                "nginx".to_string(),
                "-g".to_string(),
                "daemon off;".to_string(),
            ],
            env: vec!["FOO=bar".to_string()],
            ports: vec![ContainerPortPublish {
                host: 8080,
                container: 80,
                protocol: "TCP".to_string(),
            }],
            binds: vec!["data:/usr/share/nginx/html:ro".to_string()],
            restart: Some("always".to_string()),
        }
    }

    #[test]
    fn valid_spec_normalizes_input() {
        let mut item = spec();
        item.name = "  web-1  ".to_string();
        item.ports[0].protocol = "TCP".to_string();
        item.validate().expect("合法参数必须通过校验");
        assert_eq!(item.name, "web-1");
        assert_eq!(item.ports[0].protocol, "tcp", "协议统一小写");
    }

    #[test]
    fn blank_image_and_bad_name_are_rejected() {
        let mut item = spec();
        item.image = "   ".to_string();
        assert!(item.validate().is_err(), "镜像必填");
        let mut item = spec();
        item.name = "_web".to_string();
        assert!(item.validate().is_err(), "首字符必须是字母或数字");
        let mut item = spec();
        item.name.clear();
        assert!(item.validate().is_ok(), "留空由引擎生成随机名");
    }

    #[test]
    fn ports_and_binds_are_validated() {
        let mut item = spec();
        item.ports[0].host = 0;
        assert!(item.validate().is_err(), "端口不能为 0");

        let mut item = spec();
        item.ports[0].protocol = "sctp".to_string();
        assert!(item.validate().is_err(), "只允许 tcp/udp");

        let mut item = spec();
        item.binds = vec!["data:nginx/html".to_string()];
        assert!(item.validate().is_err(), "容器内路径必须绝对");

        let mut item = spec();
        item.binds = vec!["../etc:/data".to_string()];
        assert!(item.validate().is_err(), "不能包含 .. 段");

        let mut item = spec();
        item.binds = vec!["data:/data:xyz".to_string()];
        assert!(item.validate().is_err(), "挂载模式只允许 ro/rw");

        let mut item = spec();
        item.binds = vec!["  ".to_string()];
        assert!(item.validate().is_ok(), "空白挂载项被丢弃而不是报错");
        assert!(item.binds.is_empty());
    }

    #[test]
    fn env_and_restart_policy_are_validated() {
        let mut item = spec();
        item.env = vec!["FOO".to_string()];
        assert!(item.validate().is_err(), "环境变量必须 KEY=VALUE");

        let mut item = spec();
        item.restart = Some("never".to_string());
        assert!(item.validate().is_err(), "重启策略必须在允许集合内");
    }

    #[test]
    fn spec_round_trips_through_json() {
        let json = serde_json::to_value(spec()).expect("必须可序列化");
        let parsed: ContainerCreateSpec = serde_json::from_value(json).expect("必须可反序列化");
        assert_eq!(parsed, spec());
        // 缺字段的旧配置必须仍能解析：全部字段都有默认值。
        let minimal: ContainerCreateSpec =
            serde_json::from_str("{}").expect("空对象必须能解析成默认参数");
        assert!(minimal.image.is_empty());
    }
}
