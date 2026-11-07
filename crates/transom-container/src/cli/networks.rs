//! 网络子命令的 CLI 编排。

use super::CliBackend;
use crate::error::ContainerError;

impl CliBackend {
    pub async fn list_networks(
        &self,
    ) -> Result<Vec<transom_core::ContainerNetworkSummary>, ContainerError> {
        let stdout = self
            .run_checked(&[
                "network".to_string(),
                "ls".to_string(),
                "--format".to_string(),
                "{{json .}}".to_string(),
            ])
            .await?;
        let mut networks = Vec::new();
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            let item: crate::models::NetworkItem =
                serde_json::from_str(line).map_err(|error| ContainerError::Decode {
                    operation: "解析 CLI 网络列表".to_string(),
                    message: format!("{error}，原始行: {line}"),
                })?;
            networks.push(item.summary());
        }
        Ok(networks)
    }

    pub async fn inspect_network(
        &self,
        network: &str,
    ) -> Result<transom_core::ContainerNetworkDetails, ContainerError> {
        let (item, raw) = self
            .inspect_value::<crate::models::NetworkItem>(&[
                "network".to_string(),
                "inspect".to_string(),
                network.to_string(),
            ])
            .await?;
        Ok(item.into_details(raw))
    }

    pub async fn create_network(
        &self,
        spec: &transom_core::ContainerNetworkSpec,
    ) -> Result<(), ContainerError> {
        let mut args = vec!["network".to_string(), "create".to_string()];
        for (flag, value) in [
            ("--driver", spec.driver.as_deref()),
            ("--subnet", spec.subnet.as_deref()),
            ("--gateway", spec.gateway.as_deref()),
        ] {
            if let Some(value) = value {
                args.push(flag.to_string());
                args.push(value.to_string());
            }
        }
        if spec.internal {
            args.push("--internal".to_string());
        }
        args.push(spec.name.clone());
        self.run_checked(&args).await.map(|_| ())
    }

    pub async fn remove_network(&self, network: &str) -> Result<(), ContainerError> {
        self.run_checked(&["network".to_string(), "rm".to_string(), network.to_string()])
            .await
            .map(|_| ())
    }

    pub async fn connect_network(
        &self,
        network: &str,
        container: &str,
    ) -> Result<(), ContainerError> {
        self.run_checked(&[
            "network".to_string(),
            "connect".to_string(),
            network.to_string(),
            container.to_string(),
        ])
        .await
        .map(|_| ())
    }

    pub async fn disconnect_network(
        &self,
        network: &str,
        container: &str,
        force: bool,
    ) -> Result<(), ContainerError> {
        let mut args = vec!["network".to_string(), "disconnect".to_string()];
        if force {
            args.push("-f".to_string());
        }
        args.push(network.to_string());
        args.push(container.to_string());
        self.run_checked(&args).await.map(|_| ())
    }

    pub async fn prune_networks(
        &self,
    ) -> Result<transom_core::ContainerPruneReport, ContainerError> {
        let stdout = self
            .run_checked(&["network".to_string(), "prune".to_string(), "-f".to_string()])
            .await?;
        let mut deleted = Vec::new();
        let mut capturing = false;
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.contains("Deleted Networks:") {
                capturing = true;
                continue;
            }
            if capturing {
                deleted.push(trimmed.to_string());
            }
        }
        Ok(transom_core::ContainerPruneReport {
            deleted,
            reclaimed: 0,
            errors: Vec::new(),
        })
    }
}
