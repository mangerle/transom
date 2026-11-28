//! 生命周期钩子领域类型、匹配引擎与执行器契约。
//!
//! 设计原理（Why）：
//! - 将“靠提示词提醒容易漏掉的运维纪律”下沉为确定性钩子，例如编辑 Nginx 配置后自动运行 `nginx -t`、
//!   高危部署或整轮结束后发送飞书/钉钉/企微 Webhook 通知；
//! - `transom-ai` 运行时只负责命令路由与 HTTP 发送，底层契约由本模块提供。

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

/// 生命周期钩子触发时机。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// 工具调用派发前（返回阻塞原因时可阻止工具执行）。
    PreToolUse,
    /// 工具执行完成后（适合配置校验、产物检查）。
    #[default]
    PostToolUse,
    /// 本轮 Agent 响应结束时（适合总结汇报与 Webhook 通知）。
    Stop,
}

impl HookEvent {
    pub fn label(&self) -> &'static str {
        match self {
            Self::PreToolUse => "工具执行前",
            Self::PostToolUse => "工具执行后",
            Self::Stop => "对话结束时",
        }
    }
}

/// 钩子触发匹配条件（所有非空条件均需同时满足）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookMatcher {
    /// 匹配工具名（如 `remote_fs`、`remote_exec`），留空表示任意工具。
    #[serde(default)]
    pub tool_name: Option<String>,
    /// 匹配参数或路径中包含的子串（如 `nginx`、`docker-compose`）。
    #[serde(default)]
    pub path_contains: Option<String>,
    /// 匹配命令前缀（仅对包含 `command` 参数的工具生效）。
    #[serde(default)]
    pub command_prefix: Option<String>,
}

/// 钩子命中后执行的动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HookAction {
    /// 在本机或目标主机执行校验命令（如 `nginx -t`）。
    ShellCommand {
        command: String,
        #[serde(default)]
        target: Option<String>,
    },
    /// 向飞书 / 钉钉 / 企业微信或通用 HTTP 端点推送 JSON Webhook 通知。
    Webhook {
        url: String,
        #[serde(default)]
        template: Option<String>,
    },
}

impl Default for HookAction {
    fn default() -> Self {
        Self::ShellCommand {
            command: String::new(),
            target: None,
        }
    }
}

/// 持久化存储的单条生命周期钩子规则。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookRule {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub event: HookEvent,
    #[serde(default)]
    pub matcher: HookMatcher,
    #[serde(default)]
    pub action: HookAction,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// `PreToolUse` 钩子失败时是否阻断本次工具调用。
    #[serde(default)]
    pub block_on_failure: bool,
}

fn default_enabled() -> bool {
    true
}

/// 钩子执行时的上下文快照。
#[derive(Clone, Debug, Default)]
pub struct HookContext {
    pub tool_name: Option<String>,
    pub args_json: Option<String>,
    /// 本次工具调用实际执行的 Shell 命令（若适用）。
    pub command: Option<String>,
    pub target_label: String,
    pub tool_ok: Option<bool>,
    pub summary: String,
}

/// 钩子组执行结果汇总。
#[derive(Clone, Debug, Default)]
pub struct HookOutcome {
    /// 若某个 `block_on_failure` 钩子失败，返回阻断说明。
    pub blocked_reason: Option<String>,
    /// 供日志或反馈给模型/卡片的附加说明。
    pub notes: Vec<String>,
}

/// 控制平面实现的钩子执行器接口。
pub trait HookExecutor: Send + Sync {
    fn execute<'a>(
        &'a self,
        event: HookEvent,
        ctx: &'a HookContext,
    ) -> Pin<Box<dyn Future<Output = HookOutcome> + Send + 'a>>;
}

/// 判断单条钩子规则是否命中当前上下文。
pub fn rule_matches(rule: &HookRule, event: HookEvent, ctx: &HookContext) -> bool {
    if !rule.enabled || rule.event != event {
        return false;
    }
    if event == HookEvent::Stop {
        return true;
    }
    if let Some(expected_tool) = rule
        .matcher
        .tool_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && ctx.tool_name.as_deref() != Some(expected_tool)
    {
        return false;
    }
    let args_lower = ctx.args_json.as_deref().unwrap_or("").to_ascii_lowercase();
    if let Some(needle) = rule
        .matcher
        .path_contains
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && !args_lower.contains(&needle.to_ascii_lowercase())
        && !ctx
            .summary
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
    {
        return false;
    }
    if let Some(prefix) = rule
        .matcher
        .command_prefix
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let cmd = ctx
            .command
            .as_deref()
            .map(str::trim)
            .filter(|command| !command.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                ctx.args_json
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .and_then(|value| value["command"].as_str().map(str::to_owned))
            })
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !cmd.starts_with(&prefix.to_ascii_lowercase()) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_tool_hook_matches_nginx_edit() {
        let rule = HookRule {
            id: "h1".into(),
            name: "Nginx 校验".into(),
            event: HookEvent::PostToolUse,
            matcher: HookMatcher {
                tool_name: Some("remote_fs".into()),
                path_contains: Some("nginx".into()),
                command_prefix: None,
            },
            action: HookAction::ShellCommand {
                command: "nginx -t".into(),
                target: None,
            },
            enabled: true,
            block_on_failure: false,
        };
        let ctx = HookContext {
            tool_name: Some("remote_fs".into()),
            args_json: Some(r#"{"action":"write","path":"/etc/nginx/conf.d/app.conf"}"#.into()),
            command: None,
            target_label: "prod-01".into(),
            tool_ok: Some(true),
            summary: "/etc/nginx/conf.d/app.conf".into(),
        };
        assert!(rule_matches(&rule, HookEvent::PostToolUse, &ctx));
        assert!(!rule_matches(&rule, HookEvent::PreToolUse, &ctx));

        let command_rule = HookRule {
            matcher: HookMatcher {
                command_prefix: Some("systemctl restart".to_owned()),
                ..HookMatcher::default()
            },
            ..rule
        };
        let command_ctx = HookContext {
            command: Some("systemctl restart nginx".to_owned()),
            ..HookContext::default()
        };
        assert!(rule_matches(
            &command_rule,
            HookEvent::PostToolUse,
            &command_ctx
        ));
    }
}
