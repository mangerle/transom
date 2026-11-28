//! 外部 ACP Agent 真实模型探测：结合 ACP `session/new` 协议响应与本地 CLI 配置。

use std::env;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::Command;
use transom_ai_contracts::acp::{augmented_path_env, is_acp_command_available};

use crate::config::AcpAgentConfig;
use crate::error::AiError;
use crate::protocol_lines::{MAX_PROTOCOL_LINE_BYTES, read_bounded_line};

const ACP_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// 外部 ACP Agent 模型探测结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AcpModelProbeResult {
    /// 当前默认/激活的模型 ID。
    pub current_model: Option<String>,
    /// 探测到的可用模型列表。
    pub models: Vec<String>,
}

fn push_unique(list: &mut Vec<String>, model: &str) {
    let trimmed = model.trim();
    if !trimmed.is_empty() && !list.iter().any(|m| m == trimmed) {
        list.push(trimmed.to_owned());
    }
}

/// 从 ACP `session/new` 的 `result` 对象中提取当前模型与可用模型列表。
pub(crate) fn parse_session_new_models(result: &Value) -> AcpModelProbeResult {
    let mut out = AcpModelProbeResult::default();

    if let Some(models_obj) = result.get("models") {
        if let Some(cur) = models_obj
            .get("currentModelId")
            .or_else(|| models_obj.get("current_model_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.current_model = Some(cur.to_owned());
            push_unique(&mut out.models, cur);
        }
        if let Some(arr) = models_obj
            .get("availableModels")
            .or_else(|| models_obj.get("available_models"))
            .and_then(Value::as_array)
        {
            for item in arr {
                let id = item
                    .get("modelId")
                    .or_else(|| item.get("id"))
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str());
                if let Some(m) = id {
                    push_unique(&mut out.models, m);
                }
            }
        }
    }

    if out.current_model.is_none()
        && let Some(m) = result.get("model").and_then(Value::as_str)
    {
        let trimmed = m.trim();
        if !trimmed.is_empty() {
            out.current_model = Some(trimmed.to_owned());
            push_unique(&mut out.models, trimmed);
        }
    }

    if let Some(opts) = result.get("configOptions").and_then(Value::as_array) {
        for opt in opts {
            if opt.get("category").and_then(Value::as_str) == Some("model") {
                if out.current_model.is_none()
                    && let Some(cur) = opt.get("currentValue").and_then(Value::as_str)
                {
                    let trimmed = cur.trim();
                    if !trimmed.is_empty() {
                        out.current_model = Some(trimmed.to_owned());
                    }
                }
                if let Some(choices) = opt.get("options").and_then(Value::as_array) {
                    for choice in choices {
                        if let Some(v) = choice
                            .get("value")
                            .or_else(|| choice.get("id"))
                            .and_then(Value::as_str)
                        {
                            push_unique(&mut out.models, v);
                        }
                    }
                }
            }
        }
    }

    out
}

/// 解析 Claude Code 本地 `~/.claude/settings.json` 中的模型配置。
pub(crate) fn parse_claude_settings_json(raw: &str) -> AcpModelProbeResult {
    let mut out = AcpModelProbeResult::default();
    let Ok(val) = serde_json::from_str::<Value>(raw) else {
        return out;
    };
    if let Some(m) = val.get("model").and_then(Value::as_str) {
        let trimmed = m.trim();
        if !trimmed.is_empty() && trimmed != "default" {
            out.current_model = Some(trimmed.to_owned());
            push_unique(&mut out.models, trimmed);
        }
    }
    if let Some(env_obj) = val.get("env").and_then(Value::as_object) {
        for key in [
            "ANTHROPIC_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_SMALL_FAST_MODEL",
        ] {
            if let Some(m) = env_obj.get(key).and_then(Value::as_str) {
                let trimmed = m.trim();
                if !trimmed.is_empty() {
                    if key == "ANTHROPIC_MODEL" && out.current_model.is_none() {
                        out.current_model = Some(trimmed.to_owned());
                    }
                    push_unique(&mut out.models, trimmed);
                }
            }
        }
    }
    out
}

fn read_local_cli_models(agent_id: &str) -> AcpModelProbeResult {
    let mut out = AcpModelProbeResult::default();
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from);

    if agent_id.contains("claude") {
        if let Ok(m) = env::var("ANTHROPIC_MODEL")
            && !m.trim().is_empty()
        {
            out.current_model = Some(m.trim().to_owned());
            push_unique(&mut out.models, &m);
        }
        if let Some(h) = &home {
            for path in [
                h.join(".claude").join("settings.json"),
                h.join(".claude").join("settings.local.json"),
                h.join(".claude.json"),
            ] {
                if let Ok(raw) = std::fs::read_to_string(path) {
                    let parsed = parse_claude_settings_json(&raw);
                    if out.current_model.is_none() {
                        out.current_model = parsed.current_model;
                    }
                    for m in parsed.models {
                        push_unique(&mut out.models, &m);
                    }
                }
            }
        }
    } else if agent_id.contains("codex") {
        if let Some(h) = &home
            && let Ok(raw) = std::fs::read_to_string(h.join(".codex").join("config.toml"))
        {
            for line in raw.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("model")
                    && let Some(rhs) = rest.trim_start().strip_prefix('=')
                {
                    let val = rhs.trim().trim_matches('"').trim_matches('\'').trim();
                    if !val.is_empty() {
                        out.current_model = Some(val.to_owned());
                        push_unique(&mut out.models, val);
                    }
                }
            }
        }
    } else if agent_id.contains("kimi")
        && let Ok(m) = env::var("KIMI_MODEL")
        && !m.trim().is_empty()
    {
        out.current_model = Some(m.trim().to_owned());
        push_unique(&mut out.models, &m);
    }

    out
}

async fn probe_via_acp_stdio(agent: &AcpAgentConfig) -> Result<AcpModelProbeResult, AiError> {
    let mut cmd = Command::new(&agent.command);
    cmd.args(&agent.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(path_env) = augmented_path_env() {
        cmd.env("PATH", path_env);
    }

    let mut child = cmd.spawn().map_err(|err| AiError::AcpFailed {
        agent: agent.name.clone(),
        reason: format!("启动 ACP 探测进程失败: {err}"),
    })?;
    let mut stdin = child.stdin.take().ok_or_else(|| AiError::AcpFailed {
        agent: agent.name.clone(),
        reason: "无法获取 ACP 探测进程 stdin".to_owned(),
    })?;
    let stdout = child.stdout.take().ok_or_else(|| AiError::AcpFailed {
        agent: agent.name.clone(),
        reason: "无法获取 ACP 探测进程 stdout".to_owned(),
    })?;
    let mut reader = BufReader::new(stdout);

    let cwd = env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_else(|| ".".to_owned());

    let res = tokio::time::timeout(ACP_PROBE_TIMEOUT, async {
        let init = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientCapabilities": {},
                "clientInfo": { "name": "transom", "version": env!("CARGO_PKG_VERSION") }
            }
        });
        let init_line = format!("{init}\n");
        stdin
            .write_all(init_line.as_bytes())
            .await
            .map_err(|e| AiError::AcpFailed {
                agent: agent.name.clone(),
                reason: format!("写入 initialize 失败: {e}"),
            })?;
        stdin.flush().await.map_err(|e| AiError::AcpFailed {
            agent: agent.name.clone(),
            reason: format!("刷新 initialize 失败: {e}"),
        })?;

        let mut new_sent = false;
        while let Some(line) = read_bounded_line(&mut reader, MAX_PROTOCOL_LINE_BYTES)
            .await
            .map_err(|e| AiError::AcpFailed {
                agent: agent.name.clone(),
                reason: format!("读取 ACP 探测响应失败: {e}"),
            })?
        {
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let id = msg.get("id").and_then(Value::as_i64);
            if id == Some(1) && !new_sent {
                new_sent = true;
                let req = json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "session/new",
                    "params": { "cwd": cwd, "mcpServers": [] }
                });
                let req_line = format!("{req}\n");
                stdin
                    .write_all(req_line.as_bytes())
                    .await
                    .map_err(|e| AiError::AcpFailed {
                        agent: agent.name.clone(),
                        reason: format!("写入 session/new 失败: {e}"),
                    })?;
                stdin.flush().await.map_err(|e| AiError::AcpFailed {
                    agent: agent.name.clone(),
                    reason: format!("刷新 session/new 失败: {e}"),
                })?;
            } else if id == Some(2) {
                if let Some(err) = msg.get("error") {
                    return Err(AiError::AcpFailed {
                        agent: agent.name.clone(),
                        reason: format!("ACP session/new 返回错误: {err}"),
                    });
                }
                if let Some(result) = msg.get("result") {
                    return Ok(parse_session_new_models(result));
                }
                return Ok(AcpModelProbeResult::default());
            }
        }
        Ok(AcpModelProbeResult::default())
    })
    .await;

    drop(stdin);
    let _ = child.kill().await;
    let _ = child.wait().await;

    match res {
        Ok(inner) => inner,
        Err(_) => Err(AiError::AcpFailed {
            agent: agent.name.clone(),
            reason: "ACP 模型探测超时".to_owned(),
        }),
    }
}

/// 探测外部 ACP Agent 的实际可用模型与当前默认模型。
///
/// # Errors
/// 当外部命令不存在且本地 CLI 配置中也未发现有效模型配置时返回 [`AiError`]。
pub async fn probe_acp_models(agent: &AcpAgentConfig) -> Result<AcpModelProbeResult, AiError> {
    let local = read_local_cli_models(&agent.id);
    let acp_res = if is_acp_command_available(&agent.command) {
        probe_via_acp_stdio(agent).await.ok()
    } else {
        None
    };

    let mut merged = acp_res.unwrap_or_default();
    for m in local.models {
        if !merged.models.iter().any(|x| x == &m) {
            merged.models.insert(0, m);
        }
    }
    if let Some(local_cur) = local.current_model {
        let replace = merged
            .current_model
            .as_deref()
            .is_none_or(|c| c.is_empty() || c == "default");
        if replace {
            merged.current_model = Some(local_cur);
        }
    }

    if merged.models.is_empty() && merged.current_model.is_none() {
        return Err(AiError::AcpFailed {
            agent: agent.name.clone(),
            reason: "未能从 ACP 或本地配置获取到可用模型".to_owned(),
        });
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_new_models_and_config_options() {
        let payload = json!({
            "sessionId": "sess-1",
            "models": {
                "currentModelId": "deepseek-v4.1-flash",
                "availableModels": [
                    { "modelId": "deepseek-v4.1-flash", "name": "DeepSeek V4.1 Flash" },
                    { "modelId": "deepseek-v4.1-flash-eco", "name": "Eco" }
                ]
            }
        });
        let parsed = parse_session_new_models(&payload);
        assert_eq!(parsed.current_model.as_deref(), Some("deepseek-v4.1-flash"));
        assert_eq!(
            parsed.models,
            vec![
                "deepseek-v4.1-flash".to_owned(),
                "deepseek-v4.1-flash-eco".to_owned()
            ]
        );

        let fallback = json!({
            "configOptions": [{
                "category": "model",
                "currentValue": "gpt-5.3-codex",
                "options": [
                    { "value": "gpt-5.3-codex" },
                    { "value": "o4-mini" }
                ]
            }]
        });
        let parsed_fallback = parse_session_new_models(&fallback);
        assert_eq!(
            parsed_fallback.current_model.as_deref(),
            Some("gpt-5.3-codex")
        );
        assert_eq!(
            parsed_fallback.models,
            vec!["gpt-5.3-codex".to_owned(), "o4-mini".to_owned()]
        );
    }

    #[test]
    fn parses_claude_settings_custom_model() {
        let raw = r#"{
            "env": {
                "ANTHROPIC_MODEL": "deepseek-v4.1-flash",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "deepseek-v4.1-flash-eco"
            }
        }"#;
        let parsed = parse_claude_settings_json(raw);
        assert_eq!(parsed.current_model.as_deref(), Some("deepseek-v4.1-flash"));
        assert_eq!(
            parsed.models,
            vec![
                "deepseek-v4.1-flash".to_owned(),
                "deepseek-v4.1-flash-eco".to_owned()
            ]
        );
    }
}
