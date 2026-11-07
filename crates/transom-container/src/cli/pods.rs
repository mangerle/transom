//! Pod 子命令的 CLI 编排（Podman 专有）。

use super::CliBackend;
use crate::error::ContainerError;

/// 解析 `podman pod ls --format json`，兼容数组与逐行对象。
fn parse_pods_json(text: &str) -> Result<Vec<transom_core::ContainerPodSummary>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let values: Vec<serde_json::Value> = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Array(items)) => items,
        Ok(value @ serde_json::Value::Object(_)) => vec![value],
        Ok(_) => return Err("Pod 列表不是 JSON 数组".to_string()),
        Err(_) => text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?,
    };
    let mut pods = Vec::with_capacity(values.len());
    for value in values {
        let field = |name: &str| value.get(name).and_then(|item| item.as_str());
        let id = field("Id")
            .or_else(|| field("ID"))
            .unwrap_or_default()
            .to_string();
        if id.is_empty() {
            continue;
        }
        let containers = value
            .get("NumberOfContainers")
            .and_then(|item| item.as_u64())
            .or_else(|| {
                value
                    .get("Containers")
                    .and_then(|item| item.as_array())
                    .map(|items| items.len() as u64)
            })
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32;
        pods.push(transom_core::ContainerPodSummary {
            id,
            name: field("Name").unwrap_or_default().to_string(),
            status: field("Status").unwrap_or_default().to_string(),
            created: value
                .get("Created")
                .map(|item| match item {
                    serde_json::Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default(),
            containers,
        });
    }
    Ok(pods)
}

impl CliBackend {
    pub async fn list_pods(
        &self,
    ) -> Result<Vec<transom_core::ContainerPodSummary>, ContainerError> {
        if self.kind != transom_core::ContainerEngineKind::Podman {
            return Err(ContainerError::Unsupported {
                operation: "Pod 列表仅支持 Podman 引擎".to_string(),
            });
        }
        let stdout = self
            .run_checked(&[
                "pod".to_string(),
                "ls".to_string(),
                "--format".to_string(),
                "json".to_string(),
            ])
            .await?;
        parse_pods_json(&stdout).map_err(|message| ContainerError::Decode {
            operation: "解析 Podman Pod 列表".to_string(),
            message,
        })
    }

    pub async fn pod_action(
        &self,
        pod: &str,
        action: transom_core::ContainerPodAction,
    ) -> Result<(), ContainerError> {
        if self.kind != transom_core::ContainerEngineKind::Podman {
            return Err(ContainerError::Unsupported {
                operation: "Pod 操作仅支持 Podman 引擎".to_string(),
            });
        }
        let mut args = match action {
            transom_core::ContainerPodAction::Start => vec!["pod", "start"],
            transom_core::ContainerPodAction::Stop => vec!["pod", "stop"],
            transom_core::ContainerPodAction::Restart => vec!["pod", "restart"],
            transom_core::ContainerPodAction::Remove => vec!["pod", "rm", "-f"],
        }
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        args.push(pod.to_string());
        self.run_checked_with_timeout(&args, std::time::Duration::from_secs(120))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_podman_pod_json_array() {
        let json = r#"[
          {"Id":"abc","Name":"web","Status":"Running","Created":"2025-01-01","NumberOfContainers":2},
          {"ID":"def","Name":"db","Status":"Exited","Containers":[{},{}]}
        ]"#;
        let pods = parse_pods_json(json).expect("Pod 列表必须可解析");
        assert_eq!(pods.len(), 2);
        assert_eq!(pods[0].name, "web");
        assert_eq!(pods[0].containers, 2);
        assert_eq!(pods[1].id, "def");
        assert_eq!(pods[1].containers, 2);
    }
}
