//! 容器运行时（Docker / Podman）的连接参数与管理面数据类型。
//!
//! 设计原理（Why）：
//! - 引擎端点被建模为「引用」而不是一个裸字符串：`Auto` 让同一份会话目录
//!   可以在不同机器上工作（Linux 用 unix socket、Windows 用命名管道），
//!   显式端点则用于多引擎并存或远端守护进程；
//! - 管理面只暴露 UI 需要的字段，API 原始 JSON 在 `transom-container` 内部
//!   反序列化后立即收敛为本模块的强类型，避免界面层跟着上游字段漂移；
//! - 所有枚举都保留 `Unknown`/`Other` 兜底：外部引擎的版本差异不应该让
//!   整个会话目录或列表反序列化失败。

mod create;
mod engine;
mod inventory;

pub use create::*;
pub use engine::*;
pub use inventory::*;

/// 截短容器/镜像 ID，供列表与日志定位。
pub fn short_id(id: &str) -> String {
    let trimmed = id.trim_start_matches("sha256:");
    trimmed.chars().take(12).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ContainerEndpoint, ContainerEngineKind, ContainerEngineRef, ContainerState,
        ContainerSummary, compose_projects, short_id,
    };

    #[test]
    fn endpoint_round_trips_through_json() {
        let endpoint = ContainerEndpoint::Unix {
            path: "/var/run/docker.sock".into(),
        };
        let json = serde_json::to_string(&endpoint).expect("序列化必须成功");
        let decoded: ContainerEndpoint = serde_json::from_str(&json).expect("反序列化必须成功");
        assert_eq!(decoded, endpoint);

        let auto = serde_json::to_string(&ContainerEndpoint::Auto).expect("序列化必须成功");
        let decoded: ContainerEndpoint = serde_json::from_str(&auto).expect("反序列化必须成功");
        assert!(decoded.is_auto());
    }

    #[test]
    fn engine_ref_defaults_to_docker_auto() {
        let reference = ContainerEngineRef::default();
        assert_eq!(reference.kind, ContainerEngineKind::Docker);
        assert!(reference.endpoint.is_auto());
    }

    #[test]
    fn unknown_state_degrades_instead_of_failing() {
        assert_eq!(
            ContainerState::from_api("hibernating"),
            ContainerState::Unknown
        );
        assert!(ContainerState::from_api("running").is_running());
    }

    #[test]
    fn short_id_strips_algorithm_prefix() {
        assert_eq!(short_id("sha256:0123456789abcdef"), "0123456789ab");
        assert_eq!(short_id("abc"), "abc");
    }

    #[test]
    fn cli_endpoints_are_flagged() {
        assert!(ContainerEndpoint::Wslc.is_cli());
        assert!(
            ContainerEndpoint::Cli {
                program: "docker".into(),
                args: Vec::new()
            }
            .is_cli()
        );
        assert!(!ContainerEndpoint::Auto.is_cli());
    }

    #[test]
    fn compose_projects_group_containers_by_label() {
        let mut running = summary("web-1", ContainerState::Running);
        running.compose_project = Some("shop".into());
        running.compose_working_dir = Some("/srv/shop".into());
        running.compose_config_files = vec!["/srv/shop/compose.yml".into()];
        let mut stopped = summary("web-2", ContainerState::Exited);
        stopped.compose_project = Some("shop".into());
        let mut other = summary("standalone", ContainerState::Running);
        other.compose_project = None;

        let projects = compose_projects(&[running, stopped, other]);
        assert_eq!(projects.len(), 1, "非 Compose 容器不参与聚合");
        let project = projects.first().expect("项目存在");
        assert_eq!(project.name, "shop");
        assert_eq!(project.running, 1);
        assert_eq!(project.total, 2);
        assert_eq!(project.state_label(), "1/2 运行中");
        assert_eq!(project.working_dir.as_deref(), Some("/srv/shop"));
        assert_eq!(project.config_files.len(), 1);
    }

    fn summary(name: &str, state: ContainerState) -> ContainerSummary {
        ContainerSummary {
            id: format!("id-{name}"),
            name: name.to_string(),
            image: "demo".to_string(),
            state,
            status: String::new(),
            created: 0,
            ports: Vec::new(),
            compose_project: None,
            compose_working_dir: None,
            compose_config_files: Vec::new(),
            mounts: Vec::new(),
        }
    }
}
