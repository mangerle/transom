//! 引擎级 CLI 操作：磁盘占用、全局清理与事件流。

use super::CliBackend;
use super::{CLI_PRUNE_TIMEOUT, CLI_SYSTEM_DF_TIMEOUT, parse_human_size};
use super::{spawn_grouped, terminate_child};
use crate::error::ContainerError;
use crate::models::DockerEventMessage;
use tokio::io::AsyncBufReadExt;

impl CliBackend {
    pub async fn system_df(
        &self,
    ) -> Result<transom_core::ContainerSystemDiskUsage, ContainerError> {
        let output = self
            .run_with_timeout(
                &[
                    "system".to_string(),
                    "df".to_string(),
                    "--format={{json .}}".to_string(),
                ],
                CLI_SYSTEM_DF_TIMEOUT,
            )
            .await?;
        if output.success {
            let mut usage = transom_core::ContainerSystemDiskUsage::default();
            for line in output.stdout.lines() {
                let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                let typ = val.get("Type").and_then(|v| v.as_str()).unwrap_or_default();
                let size_str = val.get("Size").and_then(|v| v.as_str()).unwrap_or_default();
                let size_bytes = parse_human_size(size_str).unwrap_or(0);
                let count = val
                    .get("TotalCount")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let reclaimable_str = val
                    .get("Reclaimable")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let reclaimable_bytes = reclaimable_str
                    .split('(')
                    .next()
                    .and_then(parse_human_size)
                    .unwrap_or(0);

                if typ.eq_ignore_ascii_case("images") {
                    usage.images_count = count;
                    usage.images_size = size_bytes;
                    usage.images_reclaimable = reclaimable_bytes;
                } else if typ.eq_ignore_ascii_case("containers") {
                    usage.containers_count = count;
                    usage.containers_size = size_bytes;
                    usage.containers_reclaimable = reclaimable_bytes;
                } else if typ.eq_ignore_ascii_case("local volumes")
                    || typ.eq_ignore_ascii_case("volumes")
                {
                    usage.volumes_count = count;
                    usage.volumes_size = size_bytes;
                    usage.volumes_reclaimable = reclaimable_bytes;
                } else if typ.eq_ignore_ascii_case("build cache") {
                    usage.build_cache_count = count;
                    usage.build_cache_size = size_bytes;
                    usage.build_cache_reclaimable = reclaimable_bytes;
                }
            }
            usage.recalculate_totals();
            return Ok(usage);
        }

        log::warn!(
            "CLI 系统磁盘统计失败，降级为列表汇总，原因: {}",
            output.stderr.trim()
        );
        let mut usage = transom_core::ContainerSystemDiskUsage::default();
        let mut fallback_succeeded = false;
        if let Ok(images) = self.list_images().await {
            fallback_succeeded = true;
            usage.images_count = images.len();
            usage.images_size = images.iter().map(|img| img.size).sum();
        }
        if let Ok(volumes) = self.list_volumes().await {
            fallback_succeeded = true;
            usage.volumes_count = volumes.len();
        }
        if let Ok(containers) = self.list_containers().await {
            fallback_succeeded = true;
            usage.containers_count = containers.len();
        }
        if !fallback_succeeded {
            return Err(ContainerError::Cli {
                command: self.describe(&[
                    "system".to_string(),
                    "df".to_string(),
                    "--format={{json .}}".to_string(),
                ]),
                message: format!(
                    "system df 与列表回退均失败，原始错误: {}",
                    output.stderr.trim()
                ),
            });
        }
        usage.recalculate_totals();
        Ok(usage)
    }

    pub async fn system_prune(
        &self,
        all: bool,
        volumes: bool,
    ) -> Result<transom_core::ContainerPruneReport, ContainerError> {
        let mut args = vec!["system".to_string(), "prune".to_string(), "-f".to_string()];
        if all {
            args.push("-a".to_string());
        }
        if volumes {
            args.push("--volumes".to_string());
        }
        let stdout = self
            .run_checked_with_timeout(&args, CLI_PRUNE_TIMEOUT)
            .await?;
        let mut deleted = Vec::new();
        let mut reclaimed = 0;
        for line in stdout.lines() {
            let line = line.trim();
            if let Some(space_str) = line.strip_prefix("Total reclaimed space:") {
                reclaimed = parse_human_size(space_str.trim()).unwrap_or(0);
            } else if line.starts_with("Deleted ") || line.starts_with("deleted:") {
                deleted.push(line.to_string());
            }
        }
        Ok(transom_core::ContainerPruneReport {
            deleted,
            reclaimed,
            errors: Vec::new(),
        })
    }

    pub async fn listen_events(
        &self,
        event_tx: tokio::sync::mpsc::Sender<transom_core::EngineEvent>,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), ContainerError> {
        let args = vec![
            "events".to_string(),
            "--format".to_string(),
            "{{json .}}".to_string(),
        ];
        let mut cmd = self.command(&args);
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        let mut child = spawn_grouped(&mut cmd).map_err(|e| ContainerError::Cli {
            command: self.describe(&args),
            message: e.to_string(),
        })?;

        let Some(stdout) = child.inner().stdout.take() else {
            return Ok(());
        };

        let mut reader = tokio::io::BufReader::new(stdout);
        let mut line = String::new();

        loop {
            line.clear();
            tokio::select! {
                biased;
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        terminate_child(&mut child).await;
                        break;
                    }
                }
                res = reader.read_line(&mut line) => {
                    let Ok(n) = res else {
                        break;
                    };
                    if n == 0 {
                        break;
                    }
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(msg) = serde_json::from_str::<DockerEventMessage>(trimmed)
                        && let Some(event) = msg.into_engine_event()
                            && event_tx.send(event).await.is_err() {
                                terminate_child(&mut child).await;
                                return Ok(());
                            }
                }
            }
        }

        let _ = child.wait().await;
        Ok(())
    }
}
