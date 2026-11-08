//! 容器资源的 REST 操作：列表、检视、创建、生命周期、进程与统计。

use super::{DockerApi, JSON_BODY_LIMIT, format_inspect_json};
use crate::error::ContainerError;
use crate::http;
use crate::models::{
    ChangeItem, ContainerInspect, ContainerListItem, ContainerUpdateRequest, RestartPolicyUpdate,
    StatsResponse, TopResponse,
};
use hyper::Method;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use transom_core::{
    ContainerDetails, ContainerFileSystemChange, ContainerLifecycleAction, ContainerProcess,
    ContainerStats, ContainerSummary, ContainerUpdateSpec,
};

impl DockerApi {
    /// 容器列表（含已停止）。
    pub async fn list_containers(&self) -> Result<Vec<ContainerSummary>, ContainerError> {
        let items: Vec<ContainerListItem> =
            self.get_json("/containers/json?all=1", "列出容器").await?;
        Ok(items
            .into_iter()
            .map(ContainerListItem::into_summary)
            .collect())
    }

    /// 容器是否分配了 TTY：决定日志流是裸流还是 8 字节多路复用流。
    pub async fn container_tty(&self, container: &str) -> Result<bool, ContainerError> {
        let inspect: ContainerInspect = self
            .get_json(
                &format!("/containers/{}/json", http::percent_encode(container)),
                "检视容器",
            )
            .await?;
        Ok(inspect.config.map(|config| config.tty).unwrap_or(false))
    }

    /// 容器详情（基础信息页签）。
    pub async fn inspect_container(
        &self,
        container: &str,
    ) -> Result<ContainerDetails, ContainerError> {
        // size=1 让引擎顺带算可写层/镜像层占用；单容器 inspect 可以接受这点开销，
        // 统计页的磁盘卡片需要它。
        let suffix = format!(
            "/containers/{}/json?size=1",
            http::percent_encode(container)
        );
        let operation = "检视容器";
        let response = self.send(Method::GET, &suffix, None, operation).await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, operation, body, container));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let raw = format_inspect_json(&bytes);
        let inspect: ContainerInspect =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: operation.to_string(),
                message: error.to_string(),
            })?;
        Ok(inspect.into_details(raw))
    }

    /// 查询容器文件系统变更（Diff）。
    ///
    /// # Errors
    /// 容器不存在或引擎返回异常时返回 [`ContainerError`]。
    pub async fn container_changes(
        &self,
        container: &str,
    ) -> Result<Vec<ContainerFileSystemChange>, ContainerError> {
        let suffix = format!("/containers/{}/changes", http::percent_encode(container));
        let items: Vec<ChangeItem> = self.get_json(&suffix, "查询容器文件系统变更").await?;
        Ok(items.into_iter().map(ChangeItem::into_change).collect())
    }

    /// 创建容器（不启动）并返回新容器 ID。
    ///
    /// 设计原理（Why）：`Cmd` 用 argv 而不是字符串，交给引擎 exec 形态解析，
    /// 避免宿主侧拼 Shell；端口与挂载按 Docker 的 `PortBindings`/`Binds` 形态
    /// 组装，空列表传空数组而不是 null，因为 Podman 对 null 的处理与 Docker
    /// 不完全一致。
    ///
    /// # Errors
    /// 引擎拒绝（镜像不存在、名称冲突、端口占用等）时返回 [`ContainerError::Api`]，
    /// 响应体原文带上定位信息。
    pub async fn create_container(
        &self,
        spec: &transom_core::ContainerCreateSpec,
    ) -> Result<String, ContainerError> {
        let mut payload = serde_json::json!({
            "Image": spec.image,
            "Env": spec.env,
            "Cmd": spec.command,
            // 健康检查属于顶层 Config，不在 HostConfig 里：真机验证过放错位置会被
            // 守护进程静默丢弃（创建成功但 Config.Healthcheck 为 null）。
            // 留空时显式下发 NONE，避免继承镜像自带的 HEALTHCHECK 造成误判。
            "Healthcheck": healthcheck(spec),
            "HostConfig": {
                "Binds": spec.binds,
                "PortBindings": port_bindings(&spec.ports),
                "RestartPolicy": {
                    "Name": spec.restart.clone().unwrap_or_else(|| "no".to_string()),
                    "MaximumRetryCount": 0,
                },
                // 资源限制留空即不下发：0 在 Docker 里同样是「不限制」，但少写字段
                // 更不容易踩到版本差异。
                "Memory": spec.memory_mb.unwrap_or(0).saturating_mul(1024 * 1024),
                "NanoCpus": spec.nano_cpus.unwrap_or(0),
            },
        });
        if let Some(value) = spec.hostname.as_deref() {
            payload["Hostname"] = serde_json::json!(value);
        }
        if let Some(value) = spec.user.as_deref() {
            payload["User"] = serde_json::json!(value);
        }
        if let Some(value) = spec.working_dir.as_deref() {
            payload["WorkingDir"] = serde_json::json!(value);
        }
        if !spec.entrypoint.is_empty() {
            payload["Entrypoint"] = serde_json::json!(spec.entrypoint);
        }
        if !spec.labels.is_empty() {
            payload["Labels"] = serde_json::json!(spec.labels);
        }
        if spec.tty {
            payload["Tty"] = serde_json::json!(true);
        }
        let host_config =
            payload["HostConfig"]
                .as_object_mut()
                .ok_or_else(|| ContainerError::Decode {
                    operation: "构造创建容器请求".to_string(),
                    message: "HostConfig 必须是对象".to_string(),
                })?;
        if let Some(value) = spec.network_mode.as_deref() {
            host_config.insert("NetworkMode".to_string(), serde_json::json!(value));
        }
        if spec.auto_remove {
            host_config.insert("AutoRemove".to_string(), serde_json::json!(true));
        }
        if spec.read_only_rootfs {
            host_config.insert("ReadonlyRootfs".to_string(), serde_json::json!(true));
        }
        if spec.privileged {
            host_config.insert("Privileged".to_string(), serde_json::json!(true));
        }
        if spec.init {
            host_config.insert("Init".to_string(), serde_json::json!(true));
        }
        if !spec.cap_add.is_empty() {
            host_config.insert("CapAdd".to_string(), serde_json::json!(spec.cap_add));
        }
        if !spec.cap_drop.is_empty() {
            host_config.insert("CapDrop".to_string(), serde_json::json!(spec.cap_drop));
        }
        if !spec.dns.is_empty() {
            host_config.insert("Dns".to_string(), serde_json::json!(spec.dns));
        }
        let path = if spec.name.is_empty() {
            "/containers/create".to_string()
        } else {
            format!(
                "/containers/create?name={}",
                http::percent_encode(&spec.name)
            )
        };
        let body = serde_json::to_vec(&payload).map_err(|error| ContainerError::Decode {
            operation: "构造创建容器请求".to_string(),
            message: error.to_string(),
        })?;
        let response = self
            .send(Method::POST, &path, Some(body), "创建容器")
            .await?;
        if !response.status.is_success() {
            let status = response.status;
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "创建容器", body, &spec.image));
        }
        #[derive(serde::Deserialize)]
        struct Created {
            #[serde(rename = "Id", default)]
            id: String,
        }
        let body = http::read_body_bytes(response.body, 64 * 1024).await?;
        let created: Created =
            serde_json::from_slice(&body).map_err(|error| ContainerError::Decode {
                operation: "解析创建容器响应".to_string(),
                message: format!("{error}: {}", String::from_utf8_lossy(&body)),
            })?;
        Ok(created.id)
    }

    /// 容器内进程列表（`ps aux` 口径）。
    pub async fn list_processes(
        &self,
        container: &str,
    ) -> Result<Vec<ContainerProcess>, ContainerError> {
        let response: TopResponse = self
            .get_json(
                &format!(
                    "/containers/{}/top?ps_args=aux",
                    http::percent_encode(container)
                ),
                "列出容器进程",
            )
            .await?;
        Ok(response.into_processes())
    }

    /// 采样一次容器统计；CPU 百分比需要两次样本，见 [`Self::container_stats`]。
    pub async fn container_stats(
        &self,
        container: &str,
        cancel: &Arc<AtomicBool>,
    ) -> Result<ContainerStats, ContainerError> {
        let first: StatsResponse = self
            .get_json(
                &format!(
                    "/containers/{}/stats?stream=0",
                    http::percent_encode(container)
                ),
                "采样容器统计",
            )
            .await?;
        // 两个样本间隔约 1 秒：非流式响应通常没有 precpu 数据，只能用两次
        // 采样自己做差值。取消检查走 100ms 粒度，避免关闭页签后还等满一秒。
        for _ in 0..10 {
            if cancel.load(Ordering::Relaxed) {
                return Err(ContainerError::Cancelled);
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let second: StatsResponse = self
            .get_json(
                &format!(
                    "/containers/{}/stats?stream=0",
                    http::percent_encode(container)
                ),
                "采样容器统计",
            )
            .await?;
        Ok(second.to_stats(Some(&first)))
    }

    /// 启动/停止/重启/删除容器。
    pub async fn lifecycle(
        &self,
        container: &str,
        action: ContainerLifecycleAction,
    ) -> Result<(), ContainerError> {
        let encoded = http::percent_encode(container);
        let (method, path, operation) = match action {
            ContainerLifecycleAction::Start => (
                Method::POST,
                format!("/containers/{encoded}/start"),
                "启动容器",
            ),
            ContainerLifecycleAction::Stop => (
                Method::POST,
                format!("/containers/{encoded}/stop?t=10"),
                "停止容器",
            ),
            ContainerLifecycleAction::Restart => (
                Method::POST,
                format!("/containers/{encoded}/restart?t=10"),
                "重启容器",
            ),
            ContainerLifecycleAction::Pause => (
                Method::POST,
                format!("/containers/{encoded}/pause"),
                "暂停容器",
            ),
            ContainerLifecycleAction::Unpause => (
                Method::POST,
                format!("/containers/{encoded}/unpause"),
                "恢复容器",
            ),
            ContainerLifecycleAction::Kill { signal } => {
                let query = signal
                    .as_deref()
                    .map(|sig| format!("?signal={}", http::percent_encode(sig)))
                    .unwrap_or_default();
                (
                    Method::POST,
                    format!("/containers/{encoded}/kill{query}"),
                    "强制终止容器",
                )
            }
            ContainerLifecycleAction::Remove { force } => (
                Method::DELETE,
                format!("/containers/{encoded}?v=1&force={}", u8::from(force)),
                "删除容器",
            ),
        };
        let response = self.send(method, &path, None, operation).await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, operation, body, Some(container))
    }

    /// 动态更新容器资源配置（CPU、内存配额与重启策略）。
    ///
    /// # Errors
    /// 容器不存在、参数非法或引擎更新失败时返回 [`ContainerError`]。
    pub async fn update_container(
        &self,
        container: &str,
        spec: &ContainerUpdateSpec,
    ) -> Result<(), ContainerError> {
        let encoded = http::percent_encode(container);
        let path = format!("/containers/{encoded}/update");
        let body = ContainerUpdateRequest {
            memory: spec.memory_bytes,
            nano_cpus: spec.nano_cpus,
            restart_policy: spec
                .restart_policy
                .as_ref()
                .map(|name| RestartPolicyUpdate { name: name.clone() }),
        };
        let payload = serde_json::to_vec(&body).map_err(|error| ContainerError::Http {
            operation: "序列化更新容器参数".to_string(),
            message: error.to_string(),
        })?;
        let response = self
            .send(Method::POST, &path, Some(payload), "更新容器配置")
            .await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "更新容器配置", body, Some(container))
    }

    /// 将容器就地提交为新镜像（Commit）。
    ///
    /// # Errors
    /// 容器不存在或引擎提交失败时返回 [`ContainerError`]。
    pub async fn commit_container(
        &self,
        container: &str,
        repo: &str,
        tag: Option<&str>,
        comment: Option<&str>,
        author: Option<&str>,
    ) -> Result<String, ContainerError> {
        let mut query = vec![
            format!("container={}", http::percent_encode(container)),
            format!("repo={}", http::percent_encode(repo)),
        ];
        if let Some(tag) = tag
            && !tag.is_empty()
        {
            query.push(format!("tag={}", http::percent_encode(tag)));
        }
        if let Some(comment) = comment
            && !comment.is_empty()
        {
            query.push(format!("comment={}", http::percent_encode(comment)));
        }
        if let Some(author) = author
            && !author.is_empty()
        {
            query.push(format!("author={}", http::percent_encode(author)));
        }
        let path = format!("/commit?{}", query.join("&"));
        let response = self.send(Method::POST, &path, None, "提交容器镜像").await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "提交容器镜像", body, container));
        }
        let bytes = http::read_body_bytes(response.body, 64 * 1024).await?;
        let id = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("Id")
                    .and_then(|id| id.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        Ok(id)
    }

    /// 重命名容器。
    ///
    /// 设计原理（Why）：引擎只有 `POST /containers/{id}/rename?name=` 这一条
    /// 通道，没有通用「改元数据」接口；改名后正在看的工作区选中项仍然按 ID 定位，
    /// 因此界面不会因为改名丢会话。
    ///
    /// # Errors
    /// 新名称非法、容器不存在或名称已被占用时返回对应错误。
    pub async fn rename_container(
        &self,
        container: &str,
        name: &str,
    ) -> Result<(), ContainerError> {
        if !transom_core::is_valid_container_name(name) {
            return Err(ContainerError::InvalidName {
                name: name.to_string(),
                reason: "容器名只能包含字母、数字、点、下划线和短横线，且必须以字母或数字开头"
                    .to_string(),
            });
        }
        let path = format!(
            "/containers/{}/rename?name={}",
            http::percent_encode(container),
            http::percent_encode(name)
        );
        let response = self
            .send(
                Method::POST,
                &path,
                None,
                &format!("重命名容器 {container}"),
            )
            .await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "重命名容器", body, Some(container))
    }
}

/// 组装创建请求里的 `Healthcheck`：`Interval`/`Timeout` 按纳秒下发。
///
/// 设计原理（Why）：引擎对 `Healthcheck` 的字段要求是「Test 必填」，间隔与超时
/// 留空走默认 30 秒；界面允许只填命令，因此这两个字段独立可空。
fn healthcheck(spec: &transom_core::ContainerCreateSpec) -> serde_json::Value {
    let test = match spec.health_cmd.as_deref() {
        Some(cmd) => serde_json::json!(["CMD-SHELL", cmd]),
        None => serde_json::json!(["NONE"]),
    };
    let mut value = serde_json::json!({ "Test": test });
    if let Some(seconds) = spec.health_interval_secs.filter(|seconds| *seconds > 0) {
        let nanos = seconds.saturating_mul(1_000_000_000);
        value["Interval"] = serde_json::json!(nanos);
        // 真机约束：超时不得大于间隔，也不得小于 1 毫秒，否则创建直接 400。
        // 取间隔的一半再夹到 [1ms, interval] 区间内，两者都能满足。
        let timeout = (nanos / 2).clamp(1_000_000, nanos);
        value["Timeout"] = serde_json::json!(timeout);
    }
    if let Some(retries) = spec.health_retries {
        value["Retries"] = serde_json::json!(retries);
    } else if spec.health_interval_secs.is_some() {
        value["Retries"] = serde_json::json!(3);
    }
    if let Some(seconds) = spec.health_start_period_secs {
        value["StartPeriod"] = serde_json::json!(seconds.saturating_mul(1_000_000_000));
    }
    value
}

/// 把端口映射列表折叠成 Docker 的 `PortBindings` 形态：`{"80/tcp":[{"HostPort":"8080"}]}`。
fn port_bindings(ports: &[transom_core::ContainerPortPublish]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for publish in ports {
        let key = format!("{}/{}", publish.container, publish.protocol);
        let entry = serde_json::json!({
            "HostIp": "0.0.0.0",
            "HostPort": publish.host.to_string(),
        });
        match map.entry(key) {
            serde_json::map::Entry::Vacant(slot) => {
                slot.insert(serde_json::Value::Array(vec![entry]));
            }
            serde_json::map::Entry::Occupied(mut slot) => {
                if let Some(bindings) = slot.get_mut().as_array_mut() {
                    bindings.push(entry);
                }
            }
        }
    }
    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_bindings_group_by_container_port_and_protocol() {
        let bindings = port_bindings(&[
            transom_core::ContainerPortPublish {
                host: 8080,
                container: 80,
                protocol: "tcp".to_string(),
            },
            transom_core::ContainerPortPublish {
                host: 5353,
                container: 53,
                protocol: "udp".to_string(),
            },
        ]);
        assert_eq!(bindings["80/tcp"][0]["HostPort"], serde_json::json!("8080"));
        assert_eq!(bindings["53/udp"][0]["HostPort"], serde_json::json!("5353"));
        assert!(
            port_bindings(&[])
                .as_object()
                .is_some_and(serde_json::Map::is_empty)
        );
    }
}
