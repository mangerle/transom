//! 生命周期钩子的运行时 HTTP 适配层与契约兼容重导出。

use std::sync::OnceLock;
use std::time::Duration;

pub use transom_ai_contracts::hooks::*;

static WEBHOOK_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn webhook_client() -> &'static reqwest::Client {
    WEBHOOK_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// 复用进程级 `reqwest::Client` 单例向 Webhook 端点发送 JSON 载荷（带 10 秒超时）。
pub async fn send_webhook_json(url: &str, payload: &serde_json::Value) -> Result<(), String> {
    let resp = webhook_client()
        .post(url)
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
