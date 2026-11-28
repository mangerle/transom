//! 外部 MCP 调用的审计摘要清洗与异步落盘。

use transom_ai::{AiAuditEntry, truncate_tool_output};

use super::mcp_bridge::McpControllerSnapshot;

/// 生成进入审计文件的脱敏、限长摘要。
pub(super) fn sanitize_audit_summary(raw: &str) -> String {
    truncate_tool_output(raw, 2_000)
}

/// 在阻塞线程池异步追加 MCP 审计记录。
pub(super) async fn append_mcp_audit(snap: &McpControllerSnapshot, entry: AiAuditEntry) {
    let store = snap.ai_store.clone();
    match tokio::task::spawn_blocking(move || store.append_audit(&entry)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => log::warn!("写入外部 MCP 审计记录失败: {error}"),
        Err(error) => log::warn!("外部 MCP 审计落盘任务异常: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_audit_summary;

    #[test]
    fn mcp_audit_summary_redacts_and_bounds_sensitive_arguments() {
        let raw = format!(
            "[MCP] {}",
            serde_json::json!({
                "password": "super-secret",
                "body": "x".repeat(20_000),
            })
        );
        let safe = sanitize_audit_summary(&raw);
        assert!(!safe.contains("super-secret"));
        assert!(safe.len() <= 9_000);
    }
}
