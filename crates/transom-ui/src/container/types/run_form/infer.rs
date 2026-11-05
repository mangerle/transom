//! 镜像自适应智能推断、参数解析与切分算法。
//!
//! 设计原理（Why）：
//! - 根据常见官方镜像命名智能推荐端口与容器名，提供零配置开箱即用运行体验；
//! - 集中管理 CPU 核心换算与 Docker CLI 命令切分，附带严密的单元测试保障。

use transom_core::ContainerPortPublish;

/// 根据镜像引用推断建议的容器名与暴露端口映射。
pub(crate) fn infer_image_defaults(
    image: &str,
) -> (Option<String>, Vec<(u16, u16, &'static str)>) {
    let clean = image
        .split('/')
        .last()
        .unwrap_or(image)
        .split(':')
        .next()
        .unwrap_or(image)
        .trim();
    if clean.is_empty() {
        return (None, Vec::new());
    }

    let suggested_name = format!("{clean}-1");
    let lower = clean.to_ascii_lowercase();

    let ports: Vec<(u16, u16, &'static str)> = match lower.as_str() {
        "nginx" | "httpd" => vec![(8080, 80, "tcp"), (8443, 443, "tcp")],
        "redis" => vec![(6379, 6379, "tcp")],
        "mysql" | "mariadb" => vec![(3306, 3306, "tcp")],
        "postgres" | "postgresql" => vec![(5432, 5432, "tcp")],
        "mongo" | "mongodb" => vec![(27017, 27017, "tcp")],
        "rabbitmq" => vec![(5672, 5672, "tcp"), (15672, 15672, "tcp")],
        "elasticsearch" => vec![(9200, 9200, "tcp")],
        "grafana" => vec![(3000, 3000, "tcp")],
        "prometheus" => vec![(9090, 9090, "tcp")],
        "minio" => vec![(9000, 9000, "tcp"), (9001, 9001, "tcp")],
        _ => Vec::new(),
    };

    (Some(suggested_name), ports)
}

/// 把「核数」输入换算成 Docker 的 `NanoCpus`（1e-9 核）。
pub(crate) fn parse_cores(text: &str) -> Option<u64> {
    let cores: f64 = text.trim().parse().ok()?;
    if !cores.is_finite() || cores <= 0.0 {
        return None;
    }
    Some((cores * 1_000_000_000.0).round() as u64)
}

/// 命令按空白切分为 argv。
pub(crate) fn split_argv(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

/// 解析形如 "8080:80, 53:53/udp" 的端口配置字符串（兼容批量导入与测试）。
pub(crate) fn parse_ports(input: &str) -> Result<Vec<ContainerPortPublish>, String> {
    let mut out = Vec::new();
    for seg in split_list(input) {
        let (binding, proto) = match seg.split_once('/') {
            Some((b, p)) => (b.trim(), p.trim().to_ascii_lowercase()),
            None => (seg.as_str(), "tcp".to_string()),
        };
        let (host_str, cont_str) = binding
            .split_once(':')
            .ok_or_else(|| format!("端口映射缺少冒号分隔符: {seg}"))?;
        let host = host_str
            .trim()
            .parse::<u16>()
            .map_err(|_| format!("非法宿主机端口: {host_str}"))?;
        let container = cont_str
            .trim()
            .parse::<u16>()
            .map_err(|_| format!("非法容器端口: {cont_str}"))?;
        out.push(ContainerPortPublish {
            host,
            container,
            protocol: proto,
        });
    }
    Ok(out)
}

/// 按逗号切分字符串列表并去除首尾空白，跳过空条目。
pub(crate) fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{infer_image_defaults, parse_cores, split_argv};

    #[test]
    fn test_infer_image_defaults() {
        let (name, ports) = infer_image_defaults("library/redis:alpine");
        assert_eq!(name.as_deref(), Some("redis-1"));
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0], (6379, 6379, "tcp"));

        let (name, ports) = infer_image_defaults("nginx:1.27");
        assert_eq!(name.as_deref(), Some("nginx-1"));
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0], (8080, 80, "tcp"));
    }

    #[test]
    fn test_parse_cores() {
        assert_eq!(parse_cores("1.5"), Some(1_500_000_000));
        assert_eq!(parse_cores("0"), None);
        assert_eq!(parse_cores("-1"), None);
    }

    #[test]
    fn test_split_argv() {
        assert_eq!(
            split_argv("nginx -g daemon off;"),
            vec!["nginx", "-g", "daemon", "off;"]
        );
    }
}
