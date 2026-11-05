//! 容器运行命令与 Compose 片段反向导出工具。
//!
//! 设计原理（Why）：
//! - 在运维和调试过程中，经常需要将已有的容器配置复现到另一台机器、备份配置，
//!   或者迁移为 `docker-compose.yml` 规范化编排；
//! - 本模块从 [`ContainerDetails`] 的强类型元数据中提取镜像、端口映射、挂载卷、
//!   环境变量、重启策略等，以纯函数方式生成可直接执行的 `docker run` 命令与
//!   格式规范的 `docker-compose` YAML 片段。

use super::inventory::{ContainerDetails, ContainerMount, ContainerPort};

/// 为容器生成多行格式化的 `docker run` 命令行。
pub fn generate_docker_run(details: &ContainerDetails) -> String {
    let clean_name = details.name.trim_start_matches('/');
    let mut args: Vec<String> = Vec::with_capacity(16);

    args.push("docker run -d".to_string());

    if !clean_name.is_empty() {
        args.push(format!("--name {clean_name}"));
    }

    if let Some(ref policy) = details.restart_policy {
        if !policy.is_empty() && policy != "no" {
            args.push(format!("--restart {policy}"));
        }
    }

    if let Some(ref user) = details.user {
        if !user.is_empty() {
            args.push(format!("--user {user}"));
        }
    }

    if let Some(ref workdir) = details.working_dir {
        if !workdir.is_empty() {
            args.push(format!("-w {workdir}"));
        }
    }

    // 端口映射
    for port in &details.ports {
        if let Some(public) = port.public_port {
            let spec = format_port_spec(port, public);
            args.push(format!("-p {spec}"));
        }
    }

    // 数据卷与绑定挂载
    for mount in &details.mounts {
        let spec = format_mount_spec(mount);
        args.push(format!("-v {spec}"));
    }

    // 环境变量（过滤 PATH 避免过度冗长）
    for env in &details.env {
        if env.starts_with("PATH=") {
            continue;
        }
        let escaped = escape_shell_env(env);
        args.push(format!("-e {escaped}"));
    }

    // 容器入口点（若有自定义 entrypoint）
    if !details.entrypoint.is_empty() {
        let ep = details.entrypoint.join(" ");
        args.push(format!("--entrypoint \"{ep}\""));
    }

    // 镜像名称
    let image_name = if details.image.is_empty() {
        &details.image_id
    } else {
        &details.image
    };
    args.push(image_name.to_string());

    // 启动命令（若有且不与 entrypoint 重叠）
    if !details.command.is_empty() {
        args.push(details.command.clone());
    }

    args.join(" \\\n  ")
}

/// 为容器生成标准的 `docker-compose.yml` 服务定义片段。
pub fn generate_docker_compose(details: &ContainerDetails) -> String {
    let clean_name = details.name.trim_start_matches('/');
    let service_name = sanitize_service_name(clean_name);
    let mut lines = Vec::with_capacity(32);

    lines.push("services:".to_string());
    lines.push(format!("  {service_name}:"));

    let image_name = if details.image.is_empty() {
        &details.image_id
    } else {
        &details.image
    };
    lines.push(format!("    image: {image_name}"));

    if !clean_name.is_empty() {
        lines.push(format!("    container_name: {clean_name}"));
    }

    if let Some(ref policy) = details.restart_policy {
        if !policy.is_empty() && policy != "no" {
            lines.push(format!("    restart: {policy}"));
        }
    }

    if let Some(ref user) = details.user {
        if !user.is_empty() {
            lines.push(format!("    user: \"{user}\""));
        }
    }

    if let Some(ref workdir) = details.working_dir {
        if !workdir.is_empty() {
            lines.push(format!("    working_dir: {workdir}"));
        }
    }

    // 端口映射
    let mut has_ports = false;
    for port in &details.ports {
        if let Some(public) = port.public_port {
            if !has_ports {
                lines.push("    ports:".to_string());
                has_ports = true;
            }
            let spec = format_port_spec(port, public);
            lines.push(format!("      - \"{spec}\""));
        }
    }

    // 数据卷挂载
    if !details.mounts.is_empty() {
        lines.push("    volumes:".to_string());
        for mount in &details.mounts {
            let spec = format_mount_spec(mount);
            lines.push(format!("      - \"{spec}\""));
        }
    }

    // 环境变量
    let mut has_env = false;
    for env in &details.env {
        if env.starts_with("PATH=") {
            continue;
        }
        if !has_env {
            lines.push("    environment:".to_string());
            has_env = true;
        }
        lines.push(format!("      - {env}"));
    }

    // 入口与命令
    if !details.entrypoint.is_empty() {
        let ep_items = details
            .entrypoint
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("    entrypoint: [{ep_items}]"));
    }

    if !details.command.is_empty() {
        lines.push(format!("    command: {}", details.command));
    }

    lines.join("\n")
}

fn format_port_spec(port: &ContainerPort, public: u16) -> String {
    let proto = if port.protocol.is_empty() || port.protocol.eq_ignore_ascii_case("tcp") {
        ""
    } else {
        &port.protocol
    };

    let proto_suffix = if proto.is_empty() {
        String::new()
    } else {
        format!("/{proto}")
    };

    if let Some(ref ip) = port.ip {
        if !ip.is_empty() && ip != "0.0.0.0" && ip != "::" {
            return format!("{ip}:{public}:{}{proto_suffix}", port.private_port);
        }
    }
    format!("{public}:{}{proto_suffix}", port.private_port)
}

fn format_mount_spec(mount: &ContainerMount) -> String {
    let ro_suffix = if !mount.rw || mount.mode.contains("ro") {
        ":ro"
    } else {
        ""
    };
    format!("{}:{}{ro_suffix}", mount.source, mount.destination)
}

fn escape_shell_env(env: &str) -> String {
    if env.contains(' ') || env.contains('"') || env.contains('$') {
        let escaped = env.replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        env.to_string()
    }
}

fn sanitize_service_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "app".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::container::inventory::{
        ContainerDetails, ContainerMount, ContainerPort, ContainerState,
    };

    fn sample_container() -> ContainerDetails {
        ContainerDetails {
            id: "1234567890abcdef".to_string(),
            name: "/my-redis".to_string(),
            image: "redis:7-alpine".to_string(),
            image_id: "sha256:fedcba".to_string(),
            command: "redis-server --appendonly yes".to_string(),
            entrypoint: Vec::new(),
            user: Some("1000".to_string()),
            working_dir: Some("/data".to_string()),
            env: vec![
                "PATH=/usr/bin".to_string(),
                "REDIS_MAX_MEMORY=512mb".to_string(),
            ],
            state: ContainerState::Running,
            status: "Up 2 hours".to_string(),
            created: 1700000000,
            started_at: None,
            finished_at: None,
            exit_code: None,
            health: None,
            health_details: None,
            restart_policy: Some("always".to_string()),
            ports: vec![ContainerPort {
                private_port: 6379,
                public_port: Some(6379),
                ip: Some("0.0.0.0".to_string()),
                protocol: "tcp".to_string(),
            }],
            mounts: vec![ContainerMount {
                kind: "bind".to_string(),
                source: "/host/redis-data".to_string(),
                destination: "/data".to_string(),
                mode: "".to_string(),
                rw: true,
            }],
            compose_project: None,
            oom_killed: false,
            size_rw: 0,
            size_root_fs: 0,
            exposed_ports: vec!["6379/tcp".to_string()],
            inspect: String::new(),
        }
    }

    #[test]
    fn generates_valid_docker_run() {
        let details = sample_container();
        let cmd = generate_docker_run(&details);
        assert!(cmd.contains("docker run -d"));
        assert!(cmd.contains("--name my-redis"));
        assert!(cmd.contains("--restart always"));
        assert!(cmd.contains("--user 1000"));
        assert!(cmd.contains("-w /data"));
        assert!(cmd.contains("-p 6379:6379"));
        assert!(cmd.contains("-v /host/redis-data:/data"));
        assert!(cmd.contains("-e REDIS_MAX_MEMORY=512mb"));
        assert!(!cmd.contains("PATH="));
        assert!(cmd.contains("redis:7-alpine"));
        assert!(cmd.contains("redis-server --appendonly yes"));
    }

    #[test]
    fn generates_valid_docker_compose() {
        let details = sample_container();
        let compose = generate_docker_compose(&details);
        assert!(compose.contains("services:"));
        assert!(compose.contains("  my-redis:"));
        assert!(compose.contains("    image: redis:7-alpine"));
        assert!(compose.contains("    container_name: my-redis"));
        assert!(compose.contains("    restart: always"));
        assert!(compose.contains("    user: \"1000\""));
        assert!(compose.contains("    working_dir: /data"));
        assert!(compose.contains("    ports:"));
        assert!(compose.contains("      - \"6379:6379\""));
        assert!(compose.contains("    volumes:"));
        assert!(compose.contains("      - \"/host/redis-data:/data\""));
        assert!(compose.contains("    environment:"));
        assert!(compose.contains("      - REDIS_MAX_MEMORY=512mb"));
        assert!(!compose.contains("PATH="));
        assert!(compose.contains("    command: redis-server --appendonly yes"));
    }
}
