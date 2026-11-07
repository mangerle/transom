//! 引擎级操作：系统磁盘占用、全局清理与 `/events` 长连接自愈。

use super::{DockerApi, JSON_BODY_LIMIT};
use crate::error::ContainerError;
use crate::http;
use crate::models::{ContainersPruneResponse, DockerEventMessage, SystemDfResponse};
use http_body_util::BodyExt;
use hyper::Method;
use transom_core::{ContainerPruneReport, ContainerSystemDiskUsage};

impl DockerApi {
    /// 清理悬空镜像（没有 tag 也不再被任何镜像引用的那批）。
    ///
    /// 设计原理（Why）：与卷不同，镜像的 prune 默认语义就是 `dangling=true`，
    /// 这里显式带上过滤，避免依赖引擎默认值；过滤条件同样只能走查询参数。
    /// 删除对象既可能是取消标签（`Untagged`）也可能是真删层（`Deleted`），
    /// 清理所有已停止的容器。
    pub async fn prune_containers(&self) -> Result<ContainerPruneReport, ContainerError> {
        const OPERATION: &str = "清理已停止容器";
        let response = self
            .send(Method::POST, "/containers/prune", None, OPERATION)
            .await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, OPERATION, body, "containers"));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let report: ContainersPruneResponse =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: OPERATION.to_string(),
                message: error.to_string(),
            })?;
        Ok(ContainerPruneReport {
            deleted: report.containers_deleted,
            reclaimed: report.space_reclaimed,
            errors: Vec::new(),
        })
    }

    /// 查询全局系统磁盘占用分析。
    pub async fn system_df(&self) -> Result<ContainerSystemDiskUsage, ContainerError> {
        const OPERATION: &str = "查询系统磁盘占用";
        let response = self
            .send(Method::GET, "/system/df", None, OPERATION)
            .await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, OPERATION, body, "system"));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let report: SystemDfResponse =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: OPERATION.to_string(),
                message: error.to_string(),
            })?;
        Ok(report.into_disk_usage())
    }

    /// 系统级联合清理（已停止容器、镜像、网络、可选数据卷与构建缓存）。
    pub async fn system_prune(
        &self,
        all: bool,
        volumes: bool,
    ) -> Result<ContainerPruneReport, ContainerError> {
        let mut total_deleted = Vec::new();
        let mut total_reclaimed: u64 = 0;
        let mut errors = Vec::new();
        let mut merge =
            |step: &str, result: Result<ContainerPruneReport, ContainerError>| match result {
                Ok(report) => {
                    total_deleted.extend(report.deleted);
                    total_reclaimed = total_reclaimed.saturating_add(report.reclaimed);
                }
                Err(error) => errors.push(format!("{step}失败: {error}")),
            };

        merge("清理已停止容器", self.prune_containers().await);
        merge("清理镜像", self.prune_images_filtered(all).await);
        merge("清理未使用网络", self.prune_networks().await);
        if volumes {
            merge("清理未使用数据卷", self.prune_volumes().await);
        }
        merge("清理构建缓存", self.prune_build_cache().await);

        Ok(ContainerPruneReport {
            deleted: total_deleted,
            reclaimed: total_reclaimed,
            errors,
        })
    }

    /// 监听 Docker 引擎全局长连接事件流（GET /events），支持连接中断指数退避自愈。
    ///
    /// ### 设计初衷 (Why)
    /// Docker 引擎事件流用于实时驱动 UI 容器与镜像状态刷新。网络波动、引擎重启或 SSH 转发隧道抖动
    /// 均可能导致 HTTP chunk 流中断或 EOF。若直接退出，前端将永久失去事件响应。
    ///
    /// ### 核心优势
    /// 内置指数退避重连机制（1s 至 16s），并在重连等待期间积极响应平滑停机信号 `shutdown_rx`，
    /// 避免无限自旋，同时保证消费端通道关闭时立即终止。
    ///
    /// ### 代价与局限
    /// 在断线重连的间隔期内发生的瞬时事件可能遗漏；调用方可在重连后主动触发全量轮询兜底。
    pub async fn listen_events(
        &self,
        event_tx: tokio::sync::mpsc::Sender<transom_core::EngineEvent>,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), ContainerError> {
        let mut retry_delay = std::time::Duration::from_secs(1);
        const MAX_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(16);
        let mut last_timestamp = 0i64;

        loop {
            if *shutdown_rx.borrow() || event_tx.is_closed() {
                break;
            }

            let path = if last_timestamp > 0 {
                format!("/events?since={last_timestamp}")
            } else {
                "/events".to_string()
            };
            let response = match self.send(Method::GET, &path, None, "listen_events").await {
                Ok(resp) if resp.status.is_success() => {
                    retry_delay = std::time::Duration::from_secs(1);
                    resp
                }
                Ok(resp) => {
                    let body = http::read_body_text(resp.body, 4096).await;
                    log::warn!(
                        "连接 Docker /events 失败，状态码: {}，原因: {body}，将在 {retry_delay:?} 后重试",
                        resp.status
                    );
                    tokio::select! {
                        biased;
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() { break; }
                        }
                        _ = tokio::time::sleep(retry_delay) => {
                            retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
                            continue;
                        }
                    }
                    continue;
                }
                Err(error) => {
                    log::warn!("连接 Docker /events 出错: {error}，将在 {retry_delay:?} 后重试");
                    tokio::select! {
                        biased;
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() { break; }
                        }
                        _ = tokio::time::sleep(retry_delay) => {
                            retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
                            continue;
                        }
                    }
                    continue;
                }
            };

            let mut body = std::pin::pin!(response.body);
            let mut buffer = String::new();

            loop {
                tokio::select! {
                    biased;
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            log::debug!("收到平滑停机通知，结束 Docker /events 监听流");
                            return Ok(());
                        }
                    }
                    frame = body.frame() => {
                        let Some(frame_res) = frame else {
                            log::info!("Docker /events 事件流结束，准备重连自愈");
                            break;
                        };
                        let Ok(frame) = frame_res else {
                            log::warn!("Docker /events 帧读取中断，准备重连自愈");
                            break;
                        };
                        let Ok(data) = frame.into_data() else {
                            continue;
                        };
                        let Ok(text) = std::str::from_utf8(&data) else {
                            continue;
                        };
                        buffer.push_str(text);

                        while let Some(pos) = buffer.find('\n') {
                            let line = buffer[..pos].trim().to_string();
                            buffer.drain(..=pos);
                            if line.is_empty() {
                                continue;
                            }
                            if let Ok(msg) = serde_json::from_str::<DockerEventMessage>(&line)
                                && let Some(event) = msg.into_engine_event() {
                                    last_timestamp = last_timestamp.max(event.timestamp);
                                    if event_tx.send(event).await.is_err() {
                                        return Ok(());
                                    }
                                }
                        }
                    }
                }
            }

            if *shutdown_rx.borrow() || event_tx.is_closed() {
                break;
            }
            // 长连接断开期间可能漏事件：让界面做一次全量对账，避免列表长期停留在旧状态。
            let resync = transom_core::EngineEvent {
                resource_type: "engine".to_string(),
                action: "resync".to_string(),
                id: String::new(),
                name: None,
                image: None,
                exit_code: None,
                timestamp: last_timestamp,
            };
            if event_tx.send(resync).await.is_err() {
                break;
            }
            tokio::select! {
                biased;
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() { break; }
                }
                _ = tokio::time::sleep(retry_delay) => {
                    retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
                }
            }
        }

        Ok(())
    }
}
