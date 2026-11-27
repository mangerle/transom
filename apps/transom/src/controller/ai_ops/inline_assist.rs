//! 终端内联命令生成（Ctrl/Cmd+I）控制平面处理器。

use transom_ai::generate_inline_command;
use transom_core::{AiAppEvent, AppEvent};

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
    let settings = match ctx.ai_store.load_settings() {
        Ok(s) => s,
        Err(err) => {
            send_err(&mut state.ai_set, event_tx, request_id, err.to_string());
            return;
        }
    };
    let Some((provider, model_cfg)) = settings.resolve_model("") else {
        send_err(
            &mut state.ai_set,
            event_tx,
            request_id,
            "尚未配置默认 AI 模型，请先在设置中配置供应商".to_owned(),
        );
        return;
    };
    let api_key = match ctx
        .ai_store
        .load_provider_api_key(ctx.secret_store.as_ref(), &provider.id)
    {
        Ok(Some(secret)) => secret.to_string(),
        _ => String::new(),
    };
    let target = match settings.build_target(&model_cfg.id, None, api_key) {
        Ok(t) => t,
        Err(err) => {
            send_err(&mut state.ai_set, event_tx, request_id, err.to_string());
            return;
        }
    };

    state.ai_set.spawn(async move {
        let evt = match generate_inline_command(&target, &prompt, &host_label, &platform).await {
            Ok(raw) => parse_inline_response(request_id, &raw),
            Err(err) => AiAppEvent::InlineCommandResult {
                request_id,
                command: None,
                explanation: String::new(),
                error: Some(err.to_string()),
            },
        };
        let _ = event_tx.send_async(AppEvent::Ai(Box::new(evt))).await;
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
