//! 控制平面生命周期钩子（Hooks）执行器实现。
//!
//! 设计原理（Why）：
//! - `ShellCommand` 钩子优先在当前活动远端会话或本机执行，带独立超时防止挂死；
//! - `Webhook` 钩子复用进程级静态 `reqwest::Client` 单例（连接池复用），兼容飞书/钉钉/企业微信格式，
//!   绝不在日志中打印完整 Webhook URL（可能含签名 Token）。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use transom_ai::{
    ExecCommandResult, HookAction, HookContext, HookEvent, HookExecutor, HookOutcome, HookRule,
    RemoteExecutor, ToolError, redact_secrets_for_model, rule_matches, send_webhook_json,
};

use super::process::{MAX_PROCESS_CAPTURE_BYTES, capture_command_output};

async fn exec_hook_local(
    cmd: &str,
    timeout: Duration,
    cancel: CancellationToken,
) -> Result<ExecCommandResult, ToolError> {
    let mut command = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    let out = capture_command_output(
        &mut command,
        "hook",
        timeout,
        cancel,
        MAX_PROCESS_CAPTURE_BYTES,
    )
    .await?;
    let mut stderr = out.stderr_lossy().into_owned();
    if out.stdout_truncated || out.stderr_truncated {
        if !stderr.is_empty() {
            stderr.push('\n');
        }
        stderr.push_str("[输出超过上限，已截断]");
    }
    Ok(ExecCommandResult {
        exit_code: out.status.code().unwrap_or(-1),
        stdout: out.stdout_lossy().into_owned(),
        stderr,
    })
}

/// 渲染经过脱敏的 Webhook 通知文本。
fn render_webhook_text(event: HookEvent, ctx: &HookContext, template: Option<&str>) -> String {
    let safe_summary = redact_secrets_for_model(&ctx.summary);
    template
        .filter(|text| !text.trim().is_empty())
        .map(|text| {
            text.replace("{target}", &ctx.target_label)
                .replace("{summary}", &safe_summary)
        })
        .unwrap_or_else(|| {
            format!(
                "[Transom AI · {}] 目标: {} | {}",
                event.label(),
                ctx.target_label,
                safe_summary
            )
        })
}

/// 控制平面持有的钩子执行器。
pub(in crate::controller) struct ControllerHookExecutor {
    pub(in crate::controller) rules: Vec<HookRule>,
    pub(in crate::controller) remote_exec: Option<Arc<dyn RemoteExecutor>>,
    pub(in crate::controller) cancel: CancellationToken,
}

impl HookExecutor for ControllerHookExecutor {
    fn execute<'a>(
        &'a self,
        event: HookEvent,
        ctx: &'a HookContext,
    ) -> Pin<Box<dyn Future<Output = HookOutcome> + Send + 'a>> {
        Box::pin(async move {
            let mut outcome = HookOutcome::default();
            for rule in &self.rules {
                if !rule_matches(rule, event, ctx) {
                    continue;
                }
                match &rule.action {
                    HookAction::ShellCommand { command, target } => {
                        let cmd = command.trim();
                        if cmd.is_empty() {
                            continue;
                        }
                        let run_local =
                            target.as_deref() == Some("local") || self.remote_exec.is_none();
                        let res = if run_local {
                            exec_hook_local(cmd, Duration::from_secs(15), self.cancel.clone()).await
                        } else if let Some(remote) = &self.remote_exec {
                            remote
                                .exec(cmd, Duration::from_secs(15), self.cancel.clone())
                                .await
                        } else {
                            continue;
                        };
                        match res {
                            Ok(out) if out.exit_code == 0 => {
                                outcome.notes.push(format!("{}（通过）", rule.name));
                            }
                            Ok(out) => {
                                let msg = format!(
                                    "{} 失败（退出码 {}）：{}",
                                    rule.name,
                                    out.exit_code,
                                    out.stderr.trim()
                                );
                                log::warn!("生命周期钩子执行未通过: {msg}");
                                if rule.block_on_failure && event == HookEvent::PreToolUse {
                                    outcome.blocked_reason = Some(msg);
                                    return outcome;
                                }
                                outcome.notes.push(msg);
                            }
                            Err(err) => {
                                let msg = format!("{} 执行异常：{err}", rule.name);
                                log::warn!("生命周期钩子执行异常: {msg}");
                                if rule.block_on_failure && event == HookEvent::PreToolUse {
                                    outcome.blocked_reason = Some(msg);
                                    return outcome;
                                }
                                outcome.notes.push(msg);
                            }
                        }
                    }
                    HookAction::Webhook { url, template } => {
                        let url = url.trim();
                        if url.is_empty() {
                            continue;
                        }
                        let text = render_webhook_text(event, ctx, template.as_deref());
                        let payload = serde_json::json!({
                            "msg_type": "text",
                            "msgtype": "text",
                            "content": { "text": text },
                            "text": { "content": text },
                            "event": event.label(),
                            "target": ctx.target_label,
                        });
                        match send_webhook_json(url, &payload).await {
                            Ok(()) => {
                                outcome.notes.push(format!("{} 已通知", rule.name));
                            }
                            Err(err) => {
                                log::warn!("Webhook 钩子 `{}` 发送失败: {err}", rule.name);
                            }
                        }
                    }
                }
            }
            outcome
        })
    }
}

#[cfg(test)]
mod tests {
    use super::render_webhook_text;
    use transom_ai::{HookContext, HookEvent};

    #[test]
    fn webhook_summary_is_redacted() {
        let ctx = HookContext {
            target_label: "prod-01".to_owned(),
            summary: "Authorization: Bearer abcdefghijklmnopqrstuvwxyz".to_owned(),
            ..HookContext::default()
        };
        let text = render_webhook_text(HookEvent::Stop, &ctx, Some("目标={target} 摘要={summary}"));
        assert!(!text.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(text.contains("[REDACTED]"));
    }
}
