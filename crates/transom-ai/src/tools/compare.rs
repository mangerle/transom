//! 多主机横向 Diff 对比工具（`compare_hosts`）：对比两台主机的配置文件内容或诊断命令输出。

use std::time::Instant;

use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;
use similar::TextDiff;

use super::{BulkHostTarget, ToolExecutionCard, ToolRuntimeContext, to_rig_err};
use transom_ai_contracts::error::ToolError;
use transom_ai_contracts::policy::{
    ToolRisk, classify_shell_command, detect_blocking_command, format_blocking_command_hint,
};

fn quote_posix_single(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

fn clip_diff_preview(raw: &str, max_chars: usize) -> String {
    let mut chars = raw.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}\n…(Diff 预览已截断)")
    } else {
        head
    }
}

/// 多主机横向对比工具参数。
#[derive(Deserialize)]
pub struct CompareHostsArgs {
    /// 基准主机名或会话 ID（Diff 中的左侧 `-` 基准）。
    pub host_a: String,
    /// 目标主机名或会话 ID（Diff 中的右侧 `+` 对比目标）。
    pub host_b: String,
    /// 对比模式：`file`（读取文件）或 `command`（执行诊断命令）。
    pub kind: String,
    /// 目标文件路径（`kind = "file"`）或诊断命令（`kind = "command"`）。
    pub target: String,
}

/// 在两台已授权主机之间并发采集文件或命令输出，并生成结构化 Unified Diff。
pub struct CompareHostsTool {
    ctx: ToolRuntimeContext,
}

impl CompareHostsTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }

    fn resolve_target(&self, wanted: &str) -> Result<BulkHostTarget, ToolError> {
        self.ctx
            .bulk_hosts
            .iter()
            .find(|h| h.host_name == wanted || h.session_id == wanted)
            .cloned()
            .ok_or_else(|| ToolError::PolicyDenied {
                reason: format!("主机 `{wanted}` 不在当前会话已授权的多主机协同 Scope 内"),
            })
    }
}

impl super::ToolMetadata for CompareHostsTool {
    const NAME: &'static str = "compare_hosts";

    fn static_description() -> &'static str {
        "在两台已连接且在当前 Scope 内的远端主机之间并发采集指定文件或只读命令输出，生成结构化 Unified Diff 横向对比两台机器的配置/状态差异。"
    }

    fn static_parameters() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "host_a": { "type": "string", "description": "基准主机名或会话 ID（Diff 左侧基准）" },
                "host_b": { "type": "string", "description": "目标主机名或会话 ID（Diff 右侧对比目标）" },
                "kind": {
                    "type": "string",
                    "enum": ["file", "command"],
                    "description": "对比模式：file（读取文件内容）或 command（执行只读诊断命令）"
                },
                "target": { "type": "string", "description": "远端文件路径或诊断命令" }
            },
            "required": ["host_a", "host_b", "kind", "target"]
        })
    }
}

impl Tool for CompareHostsTool {
    const NAME: &'static str = "compare_hosts";
    type Error = ToolExecutionError;
    type Args = CompareHostsArgs;
    type Output = String;

    fn description(&self) -> String {
        <Self as super::ToolMetadata>::static_description().to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        <Self as super::ToolMetadata>::static_parameters()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let kind = args.kind.trim().to_ascii_lowercase();
        let (cmd, risk, kind_label) = match kind.as_str() {
            "file" => {
                let path = args.target.trim();
                if path.is_empty() {
                    return Err(to_rig_err(ToolError::InvalidArguments {
                        tool: Self::NAME,
                        reason: "对比文件路径不能为空".to_owned(),
                    }));
                }
                (
                    format!("cat -- {}", quote_posix_single(path)),
                    ToolRisk::SafeRead,
                    "文件对比",
                )
            }
            "command" => {
                if let Some(hit) = detect_blocking_command(&args.target) {
                    return Ok(format_blocking_command_hint(&hit));
                }
                let risk = classify_shell_command(&args.target);
                (args.target.clone(), risk, "状态对比")
            }
            other => {
                return Err(to_rig_err(ToolError::InvalidArguments {
                    tool: Self::NAME,
                    reason: format!("未知的对比模式 `{other}`，仅支持 `file` 或 `command`"),
                }));
            }
        };

        let target_a = self.resolve_target(&args.host_a).map_err(to_rig_err)?;
        let target_b = self.resolve_target(&args.host_b).map_err(to_rig_err)?;
        let host_label = format!("{} ↔ {}", target_a.host_name, target_b.host_name);
        let summary = format!("{kind_label}: {}", args.target);

        let started = Instant::now();
        let call_id = self
            .ctx
            .emit_tool_started(Self::NAME, &host_label, risk, &summary)
            .await;
        let timeout = self.ctx.command_timeout;
        let cancel = self.ctx.cancel.clone();

        let (res_a, res_b) = tokio::join!(
            target_a.executor.exec(&cmd, timeout, cancel.clone()),
            target_b.executor.exec(&cmd, timeout, cancel),
        );

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let (out_a, err_a) = match res_a {
            Ok(r) if r.exit_code == 0 => (r.stdout, None),
            Ok(r) => (
                String::new(),
                Some(format!(
                    "{} 退出码 {}: {}",
                    target_a.host_name,
                    r.exit_code,
                    r.stderr.trim()
                )),
            ),
            Err(err) => (
                String::new(),
                Some(format!("{} 执行失败: {err}", target_a.host_name)),
            ),
        };
        let (out_b, err_b) = match res_b {
            Ok(r) if r.exit_code == 0 => (r.stdout, None),
            Ok(r) => (
                String::new(),
                Some(format!(
                    "{} 退出码 {}: {}",
                    target_b.host_name,
                    r.exit_code,
                    r.stderr.trim()
                )),
            ),
            Err(err) => (
                String::new(),
                Some(format!("{} 执行失败: {err}", target_b.host_name)),
            ),
        };

        let errors: Vec<String> = err_a.into_iter().chain(err_b).collect();
        if !errors.is_empty() {
            let err_text = errors.join("\n");
            self.ctx
                .record_card(ToolExecutionCard {
                    call_id: call_id.into(),
                    running: false,
                    tool_name: Self::NAME.to_owned(),
                    host_label,
                    risk,
                    summary,
                    output: err_text.clone(),
                    ok: false,
                    approved: true,
                    duration_ms: elapsed_ms,
                    diff_preview: None,
                    ..Default::default()
                })
                .await;
            return Ok(err_text);
        }

        let diff = TextDiff::from_lines(&out_a, &out_b)
            .unified_diff()
            .header(&target_a.host_name, &target_b.host_name)
            .to_string();

        let (diff_preview, output_text) = if diff.trim().is_empty() {
            let lines = out_a.lines().count();
            (
                None,
                format!(
                    "两台主机（{} 与 {}）内容完全一致（共 {} 行）",
                    target_a.host_name, target_b.host_name, lines
                ),
            )
        } else {
            let added = diff
                .lines()
                .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
                .count();
            let removed = diff
                .lines()
                .filter(|l| l.starts_with('-') && !l.starts_with("---"))
                .count();
            (
                Some(clip_diff_preview(&diff, 6_000)),
                format!(
                    "{} 与 {} 存在差异（+{added} / -{removed} 行）",
                    target_a.host_name, target_b.host_name
                ),
            )
        };

        let model_reply = match &diff_preview {
            Some(d) => format!("{output_text}\n\n{d}"),
            None => output_text.clone(),
        };

        self.ctx
            .record_card(ToolExecutionCard {
                call_id: call_id.into(),
                running: false,
                tool_name: Self::NAME.to_owned(),
                host_label,
                risk,
                summary,
                output: output_text,
                ok: true,
                approved: true,
                duration_ms: elapsed_ms,
                diff_preview,
                ..Default::default()
            })
            .await;

        Ok(model_reply)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use rig_agent::tool::{Tool, ToolContext};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::{CompareHostsArgs, CompareHostsTool};
    use crate::tools::{BulkHostTarget, ExecCommandResult, RemoteExecutor, ToolRuntimeContext};
    use transom_ai_contracts::error::ToolError;
    use transom_ai_contracts::policy::HostProtection;
    use transom_ai_contracts::session::SessionScope;

    struct StaticExec(&'static str);

    #[async_trait]
    impl RemoteExecutor for StaticExec {
        async fn exec(
            &self,
            _command: &str,
            _timeout: Duration,
            _cancel: CancellationToken,
        ) -> Result<ExecCommandResult, ToolError> {
            Ok(ExecCommandResult {
                stdout: self.0.to_owned(),
                stderr: String::new(),
                exit_code: 0,
            })
        }
    }

    #[tokio::test]
    async fn compares_two_hosts_and_emits_unified_diff() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut ctx =
            ToolRuntimeContext::minimal(SessionScope::Global, tx, CancellationToken::new());
        ctx.bulk_hosts = vec![
            BulkHostTarget {
                session_id: "s1".to_owned(),
                host_name: "prod-01".to_owned(),
                login: "root".to_owned(),
                protection: HostProtection::Normal,
                executor: Arc::new(StaticExec("worker_processes 4;\nkeepalive_timeout 65;\n")),
            },
            BulkHostTarget {
                session_id: "s2".to_owned(),
                host_name: "prod-02".to_owned(),
                login: "root".to_owned(),
                protection: HostProtection::Normal,
                executor: Arc::new(StaticExec("worker_processes 8;\nkeepalive_timeout 65;\n")),
            },
        ];

        let tool = CompareHostsTool::new(ctx);
        let mut tool_ctx = ToolContext::default();
        let out = tool
            .call(
                &mut tool_ctx,
                CompareHostsArgs {
                    host_a: "prod-01".to_owned(),
                    host_b: "prod-02".to_owned(),
                    kind: "file".to_owned(),
                    target: "/etc/nginx/nginx.conf".to_owned(),
                },
            )
            .await
            .expect("对比应成功");
        assert!(out.contains("存在差异"));
        assert!(out.contains("-worker_processes 4;"));
        assert!(out.contains("+worker_processes 8;"));

        let started = rx.recv().await.expect("应发出运行中卡片");
        let crate::runner::AgentEvent::ToolCard(started) = started else {
            panic!("期望运行中工具卡片");
        };
        assert!(started.running);
        let done = rx.recv().await.expect("应发出完成态卡片");
        let crate::runner::AgentEvent::ToolCard(done) = done else {
            panic!("期望完成态工具卡片");
        };
        assert_eq!(done.tool_name, "compare_hosts");
        assert_eq!(done.host_label, "prod-01 ↔ prod-02");
        assert!(done.diff_preview.is_some());
    }
}
