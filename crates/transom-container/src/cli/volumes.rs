//! 数据卷子命令的 CLI 编排。

use super::CliBackend;
use crate::error::ContainerError;

impl CliBackend {
    pub async fn list_volumes(
        &self,
    ) -> Result<Vec<transom_core::ContainerVolumeSummary>, ContainerError> {
        let stdout = self
            .run_checked(&[
                "volume".to_string(),
                "ls".to_string(),
                "--format".to_string(),
                "{{json .}}".to_string(),
            ])
            .await?;
        let mut volumes = Vec::new();
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            let item: crate::models::VolumeItem =
                serde_json::from_str(line).map_err(|error| ContainerError::Decode {
                    operation: "解析 CLI 数据卷列表".to_string(),
                    message: format!("{error}，原始行: {line}"),
                })?;
            volumes.push(item.into_summary());
        }
        Ok(volumes)
    }

    pub async fn inspect_volume(
        &self,
        name: &str,
    ) -> Result<transom_core::ContainerVolumeDetails, ContainerError> {
        let (item, raw) = self
            .inspect_value::<crate::models::VolumeItem>(&[
                "volume".to_string(),
                "inspect".to_string(),
                name.to_string(),
            ])
            .await?;
        Ok(transom_core::ContainerVolumeDetails {
            summary: item.into_summary(),
            // CLI 列表不携带挂载信息，占用容器留给 HTTP 后端展示。
            used_by: Vec::new(),
            inspect: raw,
        })
    }

    pub async fn create_volume(
        &self,
        name: &str,
        driver: Option<&str>,
    ) -> Result<(), ContainerError> {
        let mut args = vec!["volume".to_string(), "create".to_string()];
        if let Some(driver) = driver {
            args.push("--driver".to_string());
            args.push(driver.to_string());
        }
        args.push(name.to_string());
        self.run_checked(&args).await.map(|_| ())
    }

    pub async fn remove_volume(&self, name: &str, force: bool) -> Result<(), ContainerError> {
        let mut args = vec!["volume".to_string(), "rm".to_string()];
        if force {
            args.push("-f".to_string());
        }
        args.push(name.to_string());
        self.run_checked(&args).await.map(|_| ())
    }

    pub async fn prune_volumes(
        &self,
    ) -> Result<transom_core::ContainerPruneReport, ContainerError> {
        let stdout = self
            .run_checked(&[
                "volume".to_string(),
                "ls".to_string(),
                "-q".to_string(),
                "-f".to_string(),
                "dangling=true".to_string(),
            ])
            .await?;
        let mut deleted = Vec::new();
        for name in stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            self.remove_volume(name, false).await?;
            deleted.push(name.to_string());
        }
        Ok(transom_core::ContainerPruneReport {
            deleted,
            reclaimed: 0,
            errors: Vec::new(),
        })
    }
}
