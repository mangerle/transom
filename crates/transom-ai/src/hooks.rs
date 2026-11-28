//! 生命周期钩子的运行时 HTTP 适配层与契约兼容重导出。

use std::time::Duration;

pub use transom_ai_contracts::hooks::*;

/// 复用进程级 `reqwest::Client` 单例向 Webhook 端点发送 JSON 载荷（带 10 秒超时）。
pub async fn send_webhook_json(url: &str, payload: &serde_json::Value) -> Result<(), String> {
    let client = crate::shared_http_client()?;
    let resp = client
        .post(url)
        .timeout(Duration::from_secs(10))
        .json(payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP 状态码: {}", resp.status()))
    }
}
