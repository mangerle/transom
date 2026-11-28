//! 单轮无工具轻量模型调用（终端内联命令生成、上下文压缩摘要），支持超时与取消令牌。

use std::sync::Arc;

use futures::StreamExt as _;
use parking_lot::Mutex;
use rig_agent::agent::{
    AgentBuilder, AgentHook, HookContext, ModelTurnAction, ModelTurnFinished, MultiTurnStreamItem,
};
use rig_core::message::{AssistantContent, Message};
use tokio_util::sync::CancellationToken;

use crate::runner::build_dyn_model;
use transom_ai_contracts::config::ModelTarget;
use transom_ai_contracts::error::AiError;

async fn run_single_turn_text(
    target: &ModelTarget,
    preamble: &str,
    prompt: &str,
    timeout_secs: u64,
    operation: &str,
    cancel: &CancellationToken,
) -> Result<String, AiError> {
    struct TextCollector(Arc<Mutex<String>>);
    impl AgentHook for TextCollector {
        async fn on_model_turn_finished(
            &self,
            _ctx: &HookContext,
            event: ModelTurnFinished<'_>,
        ) -> ModelTurnAction {
            let mut buf = self.0.lock();
            for item in event.content {
                if let AssistantContent::Text(t) = item {
                    buf.push_str(&t.text);
                }
            }
            ModelTurnAction::continue_run()
        }
    }

    if cancel.is_cancelled() {
        return Err(AiError::Cancelled);
    }

    let model = build_dyn_model(target);
    let collected = Arc::new(Mutex::new(String::new()));
    let agent = AgentBuilder::new(model)
        .preamble(preamble)
        .add_hook(TextCollector(collected.clone()))
        .build();
    let fut = async {
        let mut stream = agent.prompt(Message::user(prompt.to_owned())).stream();
        loop {
            let next = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AiError::Cancelled),
                item = stream.next() => item,
            };
            let Some(item) = next else {
                break;
            };
            match item {
                Ok(MultiTurnStreamItem::FinalResponse(_)) => break,
                Ok(_) => {}
                Err(err) => {
                    if cancel.is_cancelled() {
                        return Err(AiError::Cancelled);
                    }
                    return Err(AiError::ModelCallFailed {
                        protocol: target.protocol.as_str().to_owned(),
                        model: target.model.clone(),
                        reason: err.to_string(),
                    });
                }
            }
        }
        Ok(collected.lock().clone())
    };
    tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), fut)
        .await
        .map_err(|_| AiError::Timeout {
            operation: operation.to_owned(),
            timeout_secs,
        })?
}

/// 单轮无工具轻量生成终端内联命令（Ctrl/Cmd+I），带 20 秒超时与可取消支持。
///
/// # Errors
/// 模型调用失败、20 秒超时或取消时返回 [`AiError`]。
pub async fn generate_inline_command_with_cancel(
    target: &ModelTarget,
    prompt: &str,
    host_label: &str,
    platform: &str,
    cancel: &CancellationToken,
) -> Result<String, AiError> {
    let preamble = format!(
        "你是终端命令行助手。目标主机: {host_label}，平台: {platform}。\n\
        请把用户的自然语言需求转换为一条可直接执行的单行 Shell 命令。\n\
        必须严格只输出如下 JSON（不要 markdown 代码块，不要多余文字）：\n\
        {{\"command\":\"...\",\"explanation\":\"一句话中文解释\"}}"
    );
    run_single_turn_text(target, &preamble, prompt, 20, "内联命令生成", cancel).await
}

/// 单轮无工具轻量生成终端内联命令（默认无外部取消令牌的兼容入口）。
///
/// # Errors
/// 模型调用失败或 20 秒超时时返回 [`AiError`]。
pub async fn generate_inline_command(
    target: &ModelTarget,
    prompt: &str,
    host_label: &str,
    platform: &str,
) -> Result<String, AiError> {
    generate_inline_command_with_cancel(
        target,
        prompt,
        host_label,
        platform,
        &CancellationToken::new(),
    )
    .await
}

/// 调用当前模型生成保留指定重点的上下文压缩摘要（`/compact`），支持取消令牌。
///
/// # Errors
/// 模型调用失败、返回空摘要、25 秒超时或取消时返回 [`AiError`]。
pub async fn summarize_for_compaction_with_cancel(
    target: &ModelTarget,
    transcript: &str,
    focus: &str,
    cancel: &CancellationToken,
) -> Result<String, AiError> {
    let focus_line = if focus.trim().is_empty() {
        "请提炼已确认的事实、关键路径/端口/配置变更、已完成事项与未解决的问题。".to_owned()
    } else {
        format!(
            "请务必重点保留以下内容：{}\n同时保留关键主机、路径、配置变更与未完成事项。",
            focus.trim()
        )
    };
    let preamble = format!(
        "你是运维对话上下文压缩器。请将以下长对话记录压缩为精炼、结构化的中文摘要。\n{focus_line}\n只输出摘要正文，不要寒暄。"
    );
    let raw =
        run_single_turn_text(target, &preamble, transcript, 25, "上下文压缩摘要", cancel).await?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AiError::ModelCallFailed {
            protocol: target.protocol.as_str().to_owned(),
            model: target.model.clone(),
            reason: "模型返回空摘要".to_owned(),
        });
    }
    Ok(trimmed.to_owned())
}

/// 调用当前模型生成保留指定重点的上下文压缩摘要（兼容入口）。
///
/// # Errors
/// 模型调用失败、返回空摘要或 25 秒超时时返回 [`AiError`]。
pub async fn summarize_for_compaction(
    target: &ModelTarget,
    transcript: &str,
    focus: &str,
) -> Result<String, AiError> {
    summarize_for_compaction_with_cancel(target, transcript, focus, &CancellationToken::new()).await
}

#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

    use super::generate_inline_command_with_cancel;
    use transom_ai_contracts::config::{ModelTarget, ProviderProtocol};
    use transom_ai_contracts::error::AiError;
    use transom_ai_contracts::thinking::{ThinkingEffort, ThinkingSwitch};

    #[tokio::test]
    async fn pre_cancelled_single_turn_returns_cancelled_immediately() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let target = ModelTarget {
            protocol: ProviderProtocol::OpenAi,
            base_url: "http://127.0.0.1:1".to_owned(),
            api_key: String::new(),
            model: "test-model".to_owned(),
            vision: false,
            effort: Some(ThinkingEffort::Low),
            thinking_switch: ThinkingSwitch::default(),
            max_tokens: None,
            context_window: 8192,
        };
        let err = generate_inline_command_with_cancel(&target, "ls", "local", "linux", &cancel)
            .await
            .expect_err("已取消令牌必须立即返回 Cancelled");
        assert!(matches!(err, AiError::Cancelled));
    }
}
