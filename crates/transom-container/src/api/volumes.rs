//! 数据卷资源的 REST 操作：列表、检视、创建、占用反查与清理。

use super::{DockerApi, JSON_BODY_LIMIT};
use crate::error::ContainerError;
use crate::http;
use crate::models::{ContainerListItem, VolumeItem, VolumeListResponse, VolumePruneResponse};
use hyper::Method;
use transom_core::{ContainerPruneReport, ContainerVolumeDetails, ContainerVolumeSummary};

impl DockerApi {
    /// 数据卷列表。
    pub async fn list_volumes(&self) -> Result<Vec<ContainerVolumeSummary>, ContainerError> {
        let response: VolumeListResponse = self.get_json("/volumes", "列出数据卷").await?;
        Ok(response
            .volumes
            .into_iter()
            .map(VolumeItem::into_summary)
            .collect())
    }

    /// 数据卷详情；使用它的容器从容器列表的挂载信息反查。
    pub async fn inspect_volume(
        &self,
        name: &str,
    ) -> Result<ContainerVolumeDetails, ContainerError> {
        let (volume, raw) = self
            .inspect_json::<VolumeItem>(
                &format!("/volumes/{}", http::percent_encode(name)),
                "检视数据卷",
            )
            .await?;
        let containers: Vec<ContainerListItem> = self
            .get_json("/containers/json?all=1", "列出容器（卷占用）")
            .await
            .unwrap_or_default();
        let used_by = containers
            .into_iter()
            .filter(|item| {
                item.mounts
                    .iter()
                    .any(|mount| mount.name.as_deref() == Some(name))
            })
            .map(|item| item.display_name())
            .collect();
        Ok(ContainerVolumeDetails {
            summary: volume.into_summary(),
            used_by,
            inspect: raw,
        })
    }

    pub async fn create_volume(
        &self,
        name: &str,
        driver: Option<&str>,
    ) -> Result<(), ContainerError> {
        let payload = serde_json::json!({
            "Name": name,
            "Driver": driver.unwrap_or("local"),
        });
        let body = serde_json::to_vec(&payload).map_err(|error| ContainerError::Decode {
            operation: "构造数据卷请求".to_string(),
            message: error.to_string(),
        })?;
        let response = self
            .send(Method::POST, "/volumes/create", Some(body), "创建数据卷")
            .await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "创建数据卷", body, Some(name))
    }

    /// 清理未被任何容器使用的数据卷。
    ///
    /// 设计原理（Why）：Docker 的 prune 默认只回收**匿名**卷，具名卷必须显式带
    /// `all=true` 过滤才会被清理（真机实测：不带过滤时返回空列表，界面上却明明
    /// 列出了未使用卷）；界面「仅未使用卷」那一档对应的正是这个语义，所以过滤
    /// 固定写死。正在被挂载的卷由引擎自己排除，不需要我们传候选名单。
    ///
    /// 过滤条件必须走查询参数：放进 JSON 请求体会被引擎静默忽略（实测两种写法
    /// 结果不同），因此这里显式拼 URL。
    ///
    /// # Errors
    /// 引擎不可达、返回非 2xx 或响应无法解析时返回 [`ContainerError`]。
    pub async fn prune_volumes(&self) -> Result<ContainerPruneReport, ContainerError> {
        const OPERATION: &str = "清理未使用数据卷";
        let suffix = format!(
            "/volumes/prune?filters={}",
            http::percent_encode(r#"{"all":["true"]}"#)
        );
        let response = self.send(Method::POST, &suffix, None, OPERATION).await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, OPERATION, body, "volumes"));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let report: VolumePruneResponse =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: OPERATION.to_string(),
                message: error.to_string(),
            })?;
        Ok(ContainerPruneReport {
            deleted: report.volumes_deleted,
            reclaimed: report.space_reclaimed,
            errors: Vec::new(),
        })
    }

    pub async fn remove_volume(&self, name: &str, force: bool) -> Result<(), ContainerError> {
        let path = format!(
            "/volumes/{}?force={}",
            http::percent_encode(name),
            u8::from(force)
        );
        let response = self.send(Method::DELETE, &path, None, "删除数据卷").await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "删除数据卷", body, Some(name))
    }
}
