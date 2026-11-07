//! 镜像资源的 REST 操作：列表、检视、拉取/推送、构建、清理与进度流解析。

use super::{DockerApi, JSON_BODY_LIMIT, log_request_error};
use crate::error::ContainerError;
use crate::http;
use crate::models::{
    BuildCachePruneResponse, ImageHistoryItem, ImageInspect, ImageListItem, ImagePruneResponse,
    ImagePullEvent, ImageSearchItem,
};
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::Method;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use transom_core::{
    ContainerImageDetails, ContainerImageLayer, ContainerImageSummary, ContainerPruneReport,
    ContainerRegistryAuth,
};

impl DockerApi {
    /// 镜像列表。
    pub async fn list_images(&self) -> Result<Vec<ContainerImageSummary>, ContainerError> {
        let items: Vec<ImageListItem> = self.get_json("/images/json", "列出镜像").await?;
        Ok(items.into_iter().map(ImageListItem::into_summary).collect())
    }

    /// 镜像详情。
    pub async fn inspect_image(
        &self,
        image: &str,
    ) -> Result<ContainerImageDetails, ContainerError> {
        let (inspect, raw) = self
            .inspect_json::<ImageInspect>(
                &format!("/images/{}/json", http::percent_encode(image)),
                "检视镜像",
            )
            .await?;
        let history = self.image_history(image).await.unwrap_or_default();
        let mut details = inspect.into_details(raw);
        details.history = history;
        Ok(details)
    }

    /// 获取镜像构建历史层。
    pub async fn image_history(
        &self,
        image: &str,
    ) -> Result<Vec<ContainerImageLayer>, ContainerError> {
        let suffix = format!("/images/{}/history", http::percent_encode(image));
        let items: Vec<ImageHistoryItem> = self.get_json(&suffix, "获取镜像历史层").await?;
        Ok(items
            .into_iter()
            .map(ImageHistoryItem::into_layer)
            .collect())
    }

    /// 为镜像附加新标签（Tag）。
    pub async fn tag_image(
        &self,
        image: &str,
        repo: &str,
        tag: Option<&str>,
    ) -> Result<(), ContainerError> {
        let encoded_image = http::percent_encode(image);
        let encoded_repo = http::percent_encode(repo);
        let tag_query = tag
            .filter(|t| !t.is_empty())
            .map(|t| format!("&tag={}", http::percent_encode(t)))
            .unwrap_or_default();
        let path = format!("/images/{encoded_image}/tag?repo={encoded_repo}{tag_query}");
        let response = self.send(Method::POST, &path, None, "镜像打标签").await?;
        if response.status.is_success() {
            return Ok(());
        }
        let body = http::read_body_text(response.body, 8192).await;
        Err(http::status_error(
            response.status,
            "镜像打标签",
            body,
            image,
        ))
    }

    /// 拉取镜像；进度通过回调上报，流中的错误会转成 `ContainerError`。
    pub async fn pull_image(
        &self,
        reference: &str,
        progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<(), ContainerError> {
        let (image, tag) = split_reference(reference);
        let mut path = format!("/images/create?fromImage={}", http::percent_encode(&image));
        if let Some(tag) = tag {
            path.push_str(&format!("&tag={}", http::percent_encode(&tag)));
        }
        let response = self.send(Method::POST, &path, None, "拉取镜像").await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "拉取镜像", body, reference));
        }
        consume_pull_stream(response.body, progress).await
    }

    /// 在远端镜像仓库中检索镜像。
    pub async fn search_images(
        &self,
        term: &str,
        limit: Option<u32>,
    ) -> Result<Vec<transom_core::ImageSearchResult>, ContainerError> {
        let mut path = format!("/images/search?term={}", http::percent_encode(term));
        if let Some(limit) = limit {
            path.push_str(&format!("&limit={limit}"));
        }
        let response = self.send(Method::GET, &path, None, "检索远端镜像").await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "检索远端镜像", body, term));
        }
        let bytes = http::read_body_bytes(response.body, 131_072).await?;
        let items: Vec<ImageSearchItem> =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: "反序列化镜像检索结果".to_string(),
                message: error.to_string(),
            })?;
        Ok(items.into_iter().map(Into::into).collect())
    }

    /// 带任务追踪、分层进度与中止令牌的镜像拉取。
    pub async fn pull_image_task(
        &self,
        reference: &str,
        auth: Option<&ContainerRegistryAuth>,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
        progress: &mut (dyn FnMut(String, Option<transom_core::ImageLayerProgress>) + Send),
    ) -> Result<(), ContainerError> {
        let (image, tag) = split_reference(reference);
        let mut path = format!("/images/create?fromImage={}", http::percent_encode(&image));
        if let Some(tag) = tag {
            path.push_str(&format!("&tag={}", http::percent_encode(&tag)));
        }
        let auth_header = crate::auth::resolve_auth_header(&image, auth).await;
        let mut extra_headers = Vec::new();
        if let Some(h) = auth_header.as_deref() {
            extra_headers.push(("X-Registry-Auth", h));
        }
        let full_path = self.path(&path);
        let response = http::request_with_headers(
            &self.transport,
            Method::POST,
            &full_path,
            Bytes::new(),
            None,
            &extra_headers,
        )
        .await
        .inspect_err(|error| log_request_error("拉取镜像任务", error))?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "拉取镜像任务", body, reference));
        }
        consume_pull_task_stream(response.body, cancel, progress).await
    }

    /// 推送镜像；支持自动或显式鉴权，流式上报进度。
    pub async fn push_image(
        &self,
        image: &str,
        tag: Option<&str>,
        auth: Option<&ContainerRegistryAuth>,
        progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<(), ContainerError> {
        let (name, default_tag) = split_reference(image);
        let effective_tag = tag.or(default_tag.as_deref());
        let mut path = format!("/images/{}/push", http::percent_encode(&name));
        if let Some(tag) = effective_tag {
            path.push_str(&format!("?tag={}", http::percent_encode(tag)));
        }
        let auth_header = crate::auth::resolve_auth_header(image, auth).await;
        let mut extra_headers = Vec::new();
        if let Some(h) = auth_header.as_deref() {
            extra_headers.push(("X-Registry-Auth", h));
        }

        let full_path = self.path(&path);
        let response = http::request_with_headers(
            &self.transport,
            Method::POST,
            &full_path,
            Bytes::new(),
            None,
            &extra_headers,
        )
        .await
        .inspect_err(|error| log_request_error("推送镜像", error))?;

        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "推送镜像", body, image));
        }
        consume_pull_stream(response.body, progress).await
    }

    /// 构建镜像；流式上报构建进度行，支持取消。
    pub async fn build_image(
        &self,
        spec: &transom_core::ContainerBuildSpec,
        cancel: &Arc<AtomicBool>,
        progress: &mut (dyn FnMut(String) + Send),
    ) -> Result<(), ContainerError> {
        crate::build::build_image_http(self, spec, cancel, progress).await
    }

    /// 删除镜像。
    pub async fn remove_image(&self, image: &str, force: bool) -> Result<(), ContainerError> {
        let path = format!(
            "/images/{}?force={}",
            http::percent_encode(image),
            u8::from(force)
        );
        let response = self.send(Method::DELETE, &path, None, "删除镜像").await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "删除镜像", body, Some(image))
    }

    /// 清理悬空镜像。
    pub async fn prune_images(&self) -> Result<ContainerPruneReport, ContainerError> {
        self.prune_images_filtered(false).await
    }

    /// 按 `all` 语义清理镜像。
    ///
    /// Docker API 的默认 `filters` 等同于 `dangling=true`；要清理“全部未使用镜像”
    /// 必须显式传 `dangling=false`，空对象 `{}` 并不会扩大清理范围。
    pub(super) async fn prune_images_filtered(
        &self,
        all: bool,
    ) -> Result<ContainerPruneReport, ContainerError> {
        const OPERATION: &str = "清理镜像";
        let suffix = format!(
            "/images/prune?filters={}",
            http::percent_encode(image_prune_filter(all))
        );
        let response = self.send(Method::POST, &suffix, None, OPERATION).await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, OPERATION, body, "images"));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let report: ImagePruneResponse =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: OPERATION.to_string(),
                message: error.to_string(),
            })?;
        let deleted = report
            .images_deleted
            .into_iter()
            .flat_map(|entry| [entry.untagged, entry.deleted])
            .flatten()
            .collect();
        Ok(ContainerPruneReport {
            deleted,
            reclaimed: report.space_reclaimed,
            errors: Vec::new(),
        })
    }

    /// 清理构建缓存。
    pub(super) async fn prune_build_cache(&self) -> Result<ContainerPruneReport, ContainerError> {
        const OPERATION: &str = "清理构建缓存";
        let response = self
            .send(Method::POST, "/build/prune", None, OPERATION)
            .await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, OPERATION, body, "build cache"));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let report: BuildCachePruneResponse =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: OPERATION.to_string(),
                message: error.to_string(),
            })?;
        Ok(ContainerPruneReport {
            deleted: report.caches_deleted,
            reclaimed: report.space_reclaimed,
            errors: Vec::new(),
        })
    }
}

/// 镜像清理的过滤器：显式区分“仅悬空”与“全部未使用”，不依赖引擎默认值。
fn image_prune_filter(all: bool) -> &'static str {
    if all {
        r#"{"dangling":["false"]}"#
    } else {
        r#"{"dangling":["true"]}"#
    }
}

/// 拆分镜像引用为 `(仓库, 可选 tag)`。
///
/// 只在最后一个 `/` 之后找 `:`，避免把注册表端口当 tag；摘要引用不拆 tag。
fn split_reference(reference: &str) -> (String, Option<String>) {
    if reference.contains('@') {
        return (reference.to_string(), None);
    }
    let last_slash = reference.rfind('/').map(|index| index + 1).unwrap_or(0);
    match reference[last_slash..].rfind(':') {
        Some(offset) => {
            let colon = last_slash + offset;
            (
                reference[..colon].to_string(),
                Some(reference[colon + 1..].to_string()),
            )
        }
        None => (reference.to_string(), Some("latest".to_string())),
    }
}

/// 逐行消费拉取进度流；任何 `error` 字段都会终止并返回失败。
async fn consume_pull_stream(
    body: hyper::body::Incoming,
    progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
) -> Result<(), ContainerError> {
    let mut body = std::pin::pin!(body);
    let mut buffer: Vec<u8> = Vec::with_capacity(8192);
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| ContainerError::Http {
            operation: "读取镜像拉取进度".to_string(),
            message: error.to_string(),
        })?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        buffer.extend_from_slice(&data);
        while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = buffer.drain(..=newline).collect();
            if let Some(error) = handle_pull_line(&line, progress)? {
                return Err(ContainerError::Api {
                    operation: "拉取镜像".to_string(),
                    status: 500,
                    body: error,
                });
            }
        }
    }
    if !buffer.is_empty()
        && let Some(error) = handle_pull_line(&buffer, progress)?
    {
        return Err(ContainerError::Api {
            operation: "拉取镜像".to_string(),
            status: 500,
            body: error,
        });
    }
    Ok(())
}

fn handle_pull_line(
    line: &[u8],
    progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
) -> Result<Option<String>, ContainerError> {
    let text = String::from_utf8_lossy(line);
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let event: ImagePullEvent = match serde_json::from_str(text) {
        Ok(event) => event,
        Err(error) => {
            log::debug!("镜像拉取进度行无法解析，内容: {text}，原因: {error}");
            return Ok(None);
        }
    };
    if let Some(detail) = event.error_detail.and_then(|detail| detail.message) {
        return Ok(Some(detail));
    }
    if let Some(error) = event.error {
        return Ok(Some(error));
    }
    if let Some(detail) = event.progress_detail {
        progress(detail.current.unwrap_or(0), detail.total);
    }
    Ok(None)
}

async fn consume_pull_task_stream(
    body: hyper::body::Incoming,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    progress: &mut (dyn FnMut(String, Option<transom_core::ImageLayerProgress>) + Send),
) -> Result<(), ContainerError> {
    let mut body = std::pin::pin!(body);
    let mut buffer: Vec<u8> = Vec::with_capacity(8192);
    while let Some(frame) = body.frame().await {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(ContainerError::Cancelled);
        }
        let frame = frame.map_err(|error| ContainerError::Http {
            operation: "读取镜像分层进度".to_string(),
            message: error.to_string(),
        })?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        buffer.extend_from_slice(&data);
        while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = buffer.drain(..=newline).collect();
            if let Some(error) = handle_pull_task_line(&line, progress)? {
                return Err(ContainerError::Api {
                    operation: "拉取镜像".to_string(),
                    status: 500,
                    body: error,
                });
            }
        }
    }
    if !buffer.is_empty()
        && let Some(error) = handle_pull_task_line(&buffer, progress)?
    {
        return Err(ContainerError::Api {
            operation: "拉取镜像".to_string(),
            status: 500,
            body: error,
        });
    }
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(ContainerError::Cancelled);
    }
    Ok(())
}

fn handle_pull_task_line(
    line: &[u8],
    progress: &mut (dyn FnMut(String, Option<transom_core::ImageLayerProgress>) + Send),
) -> Result<Option<String>, ContainerError> {
    let text = String::from_utf8_lossy(line);
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let event: ImagePullEvent = match serde_json::from_str(text) {
        Ok(event) => event,
        Err(error) => {
            log::debug!("镜像分层进度行解析跳过，内容: {text}，原因: {error}");
            return Ok(None);
        }
    };
    if let Some(detail) = event.error_detail.and_then(|detail| detail.message) {
        return Ok(Some(detail));
    }
    if let Some(error) = event.error {
        return Ok(Some(error));
    }
    let status_desc = event.status.clone().unwrap_or_default();
    if let Some(id) = event.id {
        let (current, total) = event
            .progress_detail
            .map(|detail| (detail.current.unwrap_or(0), detail.total.unwrap_or(0)))
            .unwrap_or((0, 0));
        let layer_prog = transom_core::ImageLayerProgress {
            id,
            status: status_desc.clone(),
            current,
            total,
        };
        progress(status_desc, Some(layer_prog));
    } else if !status_desc.is_empty() {
        progress(status_desc, None);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_prune_filter_distinguishes_dangling_and_all_unused() {
        assert_eq!(image_prune_filter(false), r#"{"dangling":["true"]}"#);
        assert_eq!(image_prune_filter(true), r#"{"dangling":["false"]}"#);
    }

    #[test]
    fn reference_splits_tag_without_confusing_registry_ports() {
        assert_eq!(
            split_reference("nginx"),
            ("nginx".to_string(), Some("latest".to_string()))
        );
        assert_eq!(
            split_reference("ghcr.io/team/app:v1"),
            ("ghcr.io/team/app".to_string(), Some("v1".to_string()))
        );
        assert_eq!(
            split_reference("localhost:5000/app"),
            ("localhost:5000/app".to_string(), Some("latest".to_string()))
        );
        assert_eq!(
            split_reference("alpine@sha256:abc"),
            ("alpine@sha256:abc".to_string(), None)
        );
    }
}
