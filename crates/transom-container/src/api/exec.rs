//! 容器内 exec：创建会话、启动流、调整窗口与同步命令。

use super::{DockerApi, JSON_BODY_LIMIT};
use crate::error::ContainerError;
use crate::http;
use crate::models::{ExecCreateResponse, ExecInspectResponse};
use bytes::Bytes;
use hyper::Method;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

impl DockerApi {
    /// 创建交互式 exec（TTY）。
    pub async fn create_exec(
        &self,
        container: &str,
        command: &[String],
        user: Option<&str>,
        working_dir: Option<&str>,
        env: &[String],
        tty: bool,
    ) -> Result<String, ContainerError> {
        let payload = ExecCreateRequest {
            attach_stdin: true,
            attach_stdout: true,
            attach_stderr: true,
            tty,
            cmd: command.to_vec(),
            env: env.to_vec(),
            working_dir: working_dir.map(str::to_string),
            user: user.map(str::to_string),
        };
        let body = serde_json::to_vec(&payload).map_err(|error| ContainerError::Decode {
            operation: "构造 exec 请求".to_string(),
            message: error.to_string(),
        })?;
        let path = format!("/containers/{}/exec", http::percent_encode(container));
        let response = self
            .send(Method::POST, &path, Some(body), "创建 exec")
            .await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "创建 exec", body, container));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let created: ExecCreateResponse =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: "解析 exec 创建响应".to_string(),
                message: error.to_string(),
            })?;
        Ok(created.id)
    }

    /// 启动 exec 并取得升级后的双向流。
    pub async fn start_exec_stream(
        &self,
        exec_id: &str,
    ) -> Result<crate::transport::EngineStream, ContainerError> {
        let path = self.path(&format!("/exec/{}/start", http::percent_encode(exec_id)));
        let body = br#"{"Detach":false,"Tty":true}"#.to_vec();
        http::request_upgrade(
            &self.transport,
            Method::POST,
            &path,
            Bytes::from(body),
            Some("application/json"),
        )
        .await
    }

    /// 调整 exec 的 TTY 尺寸。
    pub async fn resize_exec(
        &self,
        exec_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), ContainerError> {
        let path = format!(
            "/exec/{}/resize?h={rows}&w={cols}",
            http::percent_encode(exec_id)
        );
        self.post_empty(&path, "调整 exec 尺寸", None).await
    }

    /// 查询 exec 是否结束以及退出码。
    pub async fn inspect_exec(&self, exec_id: &str) -> Result<ExecInspectResponse, ContainerError> {
        self.get_json(
            &format!("/exec/{}/json", http::percent_encode(exec_id)),
            "查询 exec 状态",
        )
        .await
    }

    /// 在容器内执行一条不需要交互的命令（删除、重命名等），等待结束并返回退出码。
    ///
    /// 设计与限制（Why）：归档 API 没有删除/重命名能力，只能通过 exec 调用
    /// `rm`/`mv`；参数以 argv 传递，不经过 Shell，避免引号注入。
    pub async fn run_command(
        &self,
        container: &str,
        command: &[String],
        cancel: &Arc<AtomicBool>,
    ) -> Result<i64, ContainerError> {
        let exec_id = self
            .create_exec(container, command, None, None, &[], false)
            .await
            .map_err(|error| {
                if let ContainerError::Api { status, body, .. } = &error
                    && (*status == 409 || body.contains("is not running")) {
                        return ContainerError::Unsupported {
                            operation: format!(
                                "容器 {container} 未运行，Docker 引擎不支持在已停止的容器中执行目录创建或文件删除/重命名"
                            ),
                        };
                    }
                error
            })?;
        let path = format!("/exec/{}/start", http::percent_encode(&exec_id));
        let body = br#"{"Detach":true,"Tty":false}"#.to_vec();
        let response = self
            .send(Method::POST, &path, Some(body), "启动命令")
            .await?;
        if !response.status.is_success() {
            let status = response.status;
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "启动命令", body, container));
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(ContainerError::Cancelled);
            }
            let inspect = self.inspect_exec(&exec_id).await?;
            if !inspect.running {
                return Ok(inspect.exit_code.unwrap_or(-1));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ContainerError::Http {
                    operation: "等待容器命令结束".to_string(),
                    message: "超过 30 秒仍未结束".to_string(),
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

#[derive(Debug, Serialize)]
struct ExecCreateRequest {
    #[serde(rename = "AttachStdin")]
    attach_stdin: bool,
    #[serde(rename = "AttachStdout")]
    attach_stdout: bool,
    #[serde(rename = "AttachStderr")]
    attach_stderr: bool,
    #[serde(rename = "Tty")]
    tty: bool,
    #[serde(rename = "Cmd")]
    cmd: Vec<String>,
    #[serde(rename = "Env", skip_serializing_if = "Vec::is_empty")]
    env: Vec<String>,
    #[serde(rename = "WorkingDir", skip_serializing_if = "Option::is_none")]
    working_dir: Option<String>,
    #[serde(rename = "User", skip_serializing_if = "Option::is_none")]
    user: Option<String>,
}
