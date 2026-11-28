//! 终端内联命令生成（Ctrl/Cmd+I）控制平面处理器。

use transom_ai::generate_inline_command;
use transom_core::{AiAppEvent, AppEvent};

use super::credentials::load_provider_api_key;
use crate::controller::state::{ControllerCtx, ControllerState};

pub(super) fn spawn_inline_assist(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    request_id: String,
    prompt: String,
    host_label: String,
    platform: String,
) {
    let event_tx = ctx.event_tx.clone();
    let settings = ctx.ai_config.snapshot();
    let Some((provider, model_cfg)) = settings.resolve_model("") else {
        send_err(
            &mut state.ai_set,
            event_tx,
            request_id,
            "尚未配置默认 AI 模型，请先在设置中配置供应商".to_owned(),
        );
        return;
    };
    let store = ctx.ai_store.clone();
    let secrets = ctx.secret_store.clone();
    let provider_id = provider.id.clone();
    let model_id = model_cfg.id.clone();
    state.ai_set.spawn(async move {
        let api_key = match load_provider_api_key(store, secrets, provider_id).await {
            Ok(Some(secret)) => secret,
            Ok(None) => String::new(),
            Err(error) => {
                let _ = event_tx
                    .send_async(AppEvent::Ai(Box::new(AiAppEvent::InlineCommandResult {
                        request_id,
                        command: None,
                        explanation: String::new(),
                        error: Some(error),
                    })))
                    .await;
                return;
            }
        };
        let target = match settings.build_target(&model_id, None, api_key) {
            Ok(target) => target,
            Err(error) => {
                let _ = event_tx
                    .send_async(AppEvent::Ai(Box::new(AiAppEvent::InlineCommandResult {
                        request_id,
                        command: None,
                        explanation: String::new(),
                        error: Some(error.to_string()),
                    })))
                    .await;
                return;
            }
        };
        let event = match generate_inline_command(&target, &prompt, &host_label, &platform).await {
            Ok(raw) => parse_inline_response(request_id, &raw),
            Err(error) => AiAppEvent::InlineCommandResult {
                request_id,
                command: None,
                explanation: String::new(),
                error: Some(error.to_string()),
            },
        };
        if let Err(error) = event_tx.send_async(AppEvent::Ai(Box::new(event))).await {
            log::warn!("发送内联命令生成结果失败: {error}");
        }
    });
}

fn send_err(
    ai_set: &mut tokio::task::JoinSet<()>,
    event_tx: flume::Sender<AppEvent>,
    request_id: String,
    error: String,
) {
    ai_set.spawn(async move {
        let _ = event_tx
            .send_async(AppEvent::Ai(Box::new(AiAppEvent::InlineCommandResult {
                request_id,
                command: None,
                explanation: String::new(),
                error: Some(error),
            })))
            .await;
    });
}

fn parse_inline_response(request_id: String, raw: &str) -> AiAppEvent {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(cmd) = val["command"].as_str()
    {
        return AiAppEvent::InlineCommandResult {
            request_id,
            command: Some(cmd.trim().to_owned()),
            explanation: val["explanation"].as_str().unwrap_or("").trim().to_owned(),
            error: None,
        };
    }
    let fallback_cmd = trimmed.lines().next().unwrap_or("").trim().to_owned();
    AiAppEvent::InlineCommandResult {
        request_id,
        command: (!fallback_cmd.is_empty()).then_some(fallback_cmd),
        explanation: String::new(),
        error: None,
    }
}
