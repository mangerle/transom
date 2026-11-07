//! 容器内 exec 的请求/响应线格式结构体。

use serde::Deserialize;

/// `POST /containers/{id}/exec` 响应。
#[derive(Debug, Clone, Deserialize)]
pub struct ExecCreateResponse {
    #[serde(rename = "Id")]
    pub id: String,
}

/// `GET /exec/{id}/json` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExecInspectResponse {
    #[serde(rename = "ExitCode", default)]
    pub exit_code: Option<i64>,
    #[serde(rename = "Running", default)]
    pub running: bool,
}
