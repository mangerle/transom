//! AI 控制平面密钥读取的异步边界。

use std::sync::Arc;

use transom_core::SecretStore;
use transom_store::AiStore;

/// 在阻塞线程池读取供应商 API Key，避免 Keyring I/O 阻塞异步控制面。
///
/// # Errors
/// 阻塞任务加入或 Keyring 读取失败时返回中文错误字符串，由调用方转为运行事件。
pub(super) async fn load_provider_api_key(
    store: Arc<AiStore>,
    secrets: Arc<dyn SecretStore>,
    provider_id: String,
) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        store
            .load_provider_api_key(secrets.as_ref(), &provider_id)
            .map(|value| value.map(|secret| secret.to_string()))
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("读取 AI 供应商密钥任务异常: {error}"))?
}
