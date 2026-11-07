//! 镜像子命令的 CLI 编排：列表、拉取/推送、构建、检视与清理。

use super::CliBackend;
use super::{CLI_IMAGE_REMOVE_TIMEOUT, CLI_IMAGE_TRANSFER_TIMEOUT, parse_human_size};
use crate::error::ContainerError;
use transom_core::ContainerImageSummary;

/// `docker images --format '{{json .}}'` 的字段。
#[derive(Debug, serde::Deserialize)]
struct CliImage {
    #[serde(rename = "ID", default)]
    id: String,
    #[serde(rename = "Repository", default)]
    repository: String,
    #[serde(rename = "Tag", default)]
    tag: String,
    #[serde(rename = "Size", default)]
    size: String,
}

impl CliImage {
    fn into_summary(self) -> ContainerImageSummary {
        let mut tags = Vec::with_capacity(1);
        if !self.repository.is_empty() && self.repository != "<none>" {
            tags.push(format!("{}:{}", self.repository, self.tag));
        }
        ContainerImageSummary {
            id: self.id,
            tags,
            size: parse_human_size(&self.size).unwrap_or(0),
            created: 0,
        }
    }
}

impl CliBackend {
    pub async fn list_images(&self) -> Result<Vec<ContainerImageSummary>, ContainerError> {
        let stdout = self
            .run_checked(&[
                "images".to_string(),
                "--format".to_string(),
                "{{json .}}".to_string(),
            ])
            .await?;
        let mut images = Vec::new();
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            let item: CliImage =
                serde_json::from_str(line).map_err(|error| ContainerError::Decode {
                    operation: "解析 CLI 镜像列表".to_string(),
                    message: format!("{error}，原始行: {line}"),
                })?;
            images.push(item.into_summary());
        }
        Ok(images)
    }

    pub async fn pull_image(&self, reference: &str) -> Result<(), ContainerError> {
        self.run_checked_with_timeout(
            &["pull".to_string(), reference.to_string()],
            CLI_IMAGE_TRANSFER_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    pub async fn push_image(&self, reference: &str) -> Result<(), ContainerError> {
        self.run_checked_with_timeout(
            &["push".to_string(), reference.to_string()],
            CLI_IMAGE_TRANSFER_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    pub async fn build_image(
        &self,
        spec: &transom_core::ContainerBuildSpec,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        progress: &mut (dyn FnMut(String) + Send),
    ) -> Result<(), ContainerError> {
        crate::build::build_image_cli(self, spec, cancel, progress).await
    }

    pub async fn remove_image(&self, image: &str, force: bool) -> Result<(), ContainerError> {
        let mut args = vec!["rmi".to_string()];
        if force {
            args.push("-f".to_string());
        }
        args.push(image.to_string());
        self.run_checked_with_timeout(&args, CLI_IMAGE_REMOVE_TIMEOUT)
            .await
            .map(|_| ())
    }

    pub async fn inspect_image(
        &self,
        image: &str,
    ) -> Result<transom_core::ContainerImageDetails, ContainerError> {
        let (inspect, raw) = self
            .inspect_value::<crate::models::ImageInspect>(&[
                "image".to_string(),
                "inspect".to_string(),
                image.to_string(),
            ])
            .await?;
        let history = self.image_history(image).await.unwrap_or_default();
        let mut details = inspect.into_details(raw);
        details.history = history;
        Ok(details)
    }

    pub async fn image_history(
        &self,
        image: &str,
    ) -> Result<Vec<transom_core::ContainerImageLayer>, ContainerError> {
        let args = vec![
            "history".to_string(),
            "--no-trunc".to_string(),
            "--format={{json .}}".to_string(),
            image.to_string(),
        ];
        let stdout = self.run_checked(&args).await?;
        let mut layers = Vec::new();
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(item) = serde_json::from_str::<crate::models::ImageHistoryItem>(line) {
                layers.push(item.into_layer());
            }
        }
        Ok(layers)
    }

    pub async fn tag_image(
        &self,
        image: &str,
        repo: &str,
        tag: Option<&str>,
    ) -> Result<(), ContainerError> {
        let target = match tag {
            Some(t) if !t.is_empty() => format!("{repo}:{t}"),
            _ => repo.to_string(),
        };
        let args = vec!["tag".to_string(), image.to_string(), target];
        self.run_checked(&args).await.map(|_| ())
    }

    pub async fn prune_images(&self) -> Result<transom_core::ContainerPruneReport, ContainerError> {
        let stdout = self
            .run_checked(&[
                "image".to_string(),
                "ls".to_string(),
                "-q".to_string(),
                "-f".to_string(),
                "dangling=true".to_string(),
            ])
            .await?;
        let mut deleted = Vec::new();
        for id in stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            self.remove_image(id, true).await?;
            deleted.push(transom_core::short_id(id));
        }
        Ok(transom_core::ContainerPruneReport {
            deleted,
            reclaimed: 0,
            errors: Vec::new(),
        })
    }
}
