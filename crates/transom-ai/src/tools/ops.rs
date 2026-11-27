//! 远端文件聚合操作、文件传输、运行时监控、容器操作与端口转发工具。

use std::time::Instant;

use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;
use similar::TextDiff;

use super::{AppBridgeDomain, ToolRuntimeContext, TransferDirection, to_rig_err};
use crate::compaction::truncate_tool_output;
use crate::error::ToolError;
use crate::policy::ToolRisk;

const MAX_REMOTE_READ_BYTES: usize = 256 * 1024;

#[derive(Deserialize)]
pub struct RemoteFsArgs {
    action: String,
    path: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    target_path: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    recursive: bool,
}

/// 聚合文件系统操作（list / read / search / edit / mkdir / rename / delete / chmod），支持本机与远端。
pub struct RemoteFsTool {
    ctx: ToolRuntimeContext,
}

impl RemoteFsTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for RemoteFsTool {
    const NAME: &'static str = "remote_fs";
    type Error = ToolExecutionError;
    type Args = RemoteFsArgs;
    type Output = String;

    fn description(&self) -> String {
        "操作本机（target=\"local\"）或远端主机文件系统：action 可选 list / read / search / edit / mkdir / rename / delete / chmod。\
        edit 会在写入前自动生成 Diff 并备份原文件；未连接的远端目标传入 target 会自动连接。"
            .to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "read", "search", "edit", "mkdir", "rename", "delete", "chmod"] },
                "target": { "type": "string", "description": "可选目标：\"local\" 表示本机；服务器名/session_id/IP 表示远端；省略则使用当前活动 SSH 会话" },
                "path": { "type": "string", "description": "文件或目录绝对路径" },
                "content": { "type": "string", "description": "action=edit 时的完整新内容" },
                "pattern": { "type": "string", "description": "action=search 时的搜索关键词" },
                "target_path": { "type": "string", "description": "action=rename 时的目标路径" },
                "mode": { "type": "string", "description": "action=chmod 时的八进制权限，如 0755" },
                "recursive": { "type": "boolean", "description": "action=delete 时是否递归删除" }
            },
            "required": ["action", "path"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let started = Instant::now();
        let mut diff_preview = None;
        let mut revert_target_path = None;
        let mut revert_backup_path = None;
        let risk = match args.action.as_str() {
            "delete" => ToolRisk::Destructive,
            "edit" | "mkdir" | "rename" | "chmod" => ToolRisk::Write,
            _ => ToolRisk::SafeRead,
        };
        let target_trimmed = args
            .target
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let host_label = match target_trimmed {
            Some(t) if t.eq_ignore_ascii_case("local") => "本机 (Local)".to_owned(),
            Some(t) => t.to_owned(),
            None => String::new(),
        };

        if target_trimmed.is_some() || self.ctx.caps.remote_fs.is_none() {
            let bridge = self
                .ctx
                .caps
                .app_bridge
                .as_ref()
                .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
            let payload = json!({
                "target": target_trimmed,
                "path": args.path,
                "content": args.content,
                "pattern": args.pattern,
                "target_path": args.target_path,
                "mode": args.mode,
                "recursive": args.recursive,
            });
            let res = bridge
                .invoke(AppBridgeDomain::RemoteFs, &args.action, payload)
                .await;
            let elapsed = started.elapsed().as_millis() as u64;
            return match res {
                Ok(text) => {
                    let clipped = self.ctx.format_tool_output(Self::NAME, &text);
                    self.ctx.record_card(crate::tools::ToolExecutionCard {
                        tool_name: Self::NAME.to_owned(),
                        host_label,
                        risk,
                        summary: format!("{} {}", args.action, args.path),
                        output: clipped.clone(),
                        ok: true,
                        approved: true,
                        duration_ms: elapsed,
                        diff_preview: None,
                        ..Default::default()
                    });
                    Ok(clipped)
                }
                Err(err) => {
                    let msg = err.to_string();
                    self.ctx.record_card(crate::tools::ToolExecutionCard {
                        tool_name: Self::NAME.to_owned(),
                        host_label,
                        risk,
                        summary: format!("{} {}", args.action, args.path),
                        output: msg,
                        ok: false,
                        approved: true,
                        duration_ms: elapsed,
                        diff_preview: None,
                        ..Default::default()
                    });
                    Err(to_rig_err(err))
                }
            };
        }

        let fs = self
            .ctx
            .caps
            .remote_fs
            .as_ref()
            .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;

        let result = match args.action.as_str() {
            "list" => fs.list_dir(&args.path).await,
            "read" => fs.read_file(&args.path, MAX_REMOTE_READ_BYTES).await,
            "search" => {
                let pat = args.pattern.as_deref().unwrap_or("");
                fs.search_text(&args.path, pat).await
            }
            "mkdir" => fs
                .mkdir(&args.path)
                .await
                .map(|()| format!("已创建目录: {}", args.path)),
            "rename" => {
                let to = args.target_path.as_deref().ok_or_else(|| {
                    to_rig_err(ToolError::InvalidArguments {
                        tool: Self::NAME,
                        reason: "rename 必须提供 target_path".to_owned(),
                    })
                })?;
                fs.rename(&args.path, to)
                    .await
                    .map(|()| format!("已重命名/移动: {} -> {to}", args.path))
            }
            "delete" => fs
                .delete(&args.path, args.recursive)
                .await
                .map(|()| format!("已删除远端路径: {}", args.path)),
            "chmod" => {
                let raw_mode = args.mode.as_deref().ok_or_else(|| {
                    to_rig_err(ToolError::InvalidArguments {
                        tool: Self::NAME,
                        reason: "chmod 必须提供八进制 mode，如 755 或 0644".to_owned(),
                    })
                })?;
                let parsed_mode = u32::from_str_radix(raw_mode.trim_start_matches('0'), 8)
                    .map_err(|_| {
                        to_rig_err(ToolError::InvalidArguments {
                            tool: Self::NAME,
                            reason: format!("无效的八进制权限: {raw_mode}"),
                        })
                    })?;
                fs.chmod(&args.path, parsed_mode)
                    .await
                    .map(|()| format!("已修改权限 {raw_mode}: {}", args.path))
            }
            "edit" => {
                let content = args.content.as_deref().unwrap_or("");
                let old_read = fs.read_file(&args.path, MAX_REMOTE_READ_BYTES).await;
                let old_text = old_read.as_deref().unwrap_or("");
                let diff = TextDiff::from_lines(old_text, content)
                    .unified_diff()
                    .header("修改前", "修改后")
                    .to_string();
                diff_preview = Some(truncate_tool_output(&diff, 3_000));
                let write_res = fs.write_file(&args.path, content, true).await;
                if write_res.is_ok()
                    && let Ok(original) = old_read
                {
                    revert_target_path = Some(args.path.clone());
                    revert_backup_path = self.ctx.save_revert_snapshot(&original);
                }
                write_res.map(|()| format!("已写入文件并完成备份: {}", args.path))
            }
            other => Err(ToolError::InvalidArguments {
                tool: Self::NAME,
                reason: format!("未知 action: {other}"),
            }),
        };

        let elapsed = started.elapsed().as_millis() as u64;
        match result {
            Ok(text) => {
                let clipped = self.ctx.format_tool_output(Self::NAME, &text);
                self.ctx.record_card(crate::tools::ToolExecutionCard {
                    tool_name: Self::NAME.to_owned(),
                    host_label: String::new(),
                    risk,
                    summary: format!("{} {}", args.action, args.path),
                    output: clipped.clone(),
                    ok: true,
                    approved: true,
                    duration_ms: elapsed,
                    diff_preview,
                    revert_target_path,
                    revert_backup_path,
                    ..Default::default()
                });
                Ok(clipped)
            }
            Err(err) => {
                let msg = err.to_string();
                self.ctx.record_card(crate::tools::ToolExecutionCard {
                    tool_name: Self::NAME.to_owned(),
                    host_label: String::new(),
                    risk,
                    summary: format!("{} {}", args.action, args.path),
                    output: msg,
                    ok: false,
                    approved: true,
                    duration_ms: elapsed,
                    diff_preview,
                    ..Default::default()
                });
                Err(to_rig_err(err))
            }
        }
    }
}

#[derive(Deserialize)]
pub struct TransferFileArgs {
    direction: TransferDirection,
    local_path: String,
    remote_path: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default = "default_true")]
    overwrite: bool,
}

fn default_true() -> bool {
    true
}

/// 本机与远端主机之间的 SFTP 文件/目录传输（强制人工审批）。
pub struct TransferFileTool {
    ctx: ToolRuntimeContext,
}

impl TransferFileTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for TransferFileTool {
    const NAME: &'static str = "transfer_file";
    type Error = ToolExecutionError;
    type Args = TransferFileArgs;
    type Output = String;

    fn description(&self) -> String {
        "在本机与远端主机之间通过 SFTP 上传或下载文件（upload 支持整个目录递归上传）。\
         可通过 target 指定目标服务器名/session_id/IP（未连接时自动连接）；省略 target 则使用当前活动 SSH 会话。"
            .to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "direction": { "type": "string", "enum": ["upload", "download"] },
                "target": { "type": "string", "description": "可选目标服务器名/session_id/IP（未连接自动连接）；省略则使用当前活动 SSH 会话" },
                "local_path": { "type": "string", "description": "本机文件或目录路径（upload 支持目录）" },
                "remote_path": { "type": "string" },
                "overwrite": { "type": "boolean", "description": "是否覆盖已存在目标，默认 true" }
            },
            "required": ["direction", "local_path", "remote_path"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let target_trimmed = args
            .target
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let is_dir_upload = args.direction == TransferDirection::Upload
            && std::path::Path::new(&args.local_path).is_dir();
        let use_bridge =
            target_trimmed.is_some() || is_dir_upload || self.ctx.caps.remote_fs.is_none();
        let started = Instant::now();
        let summary = format!("{} -> {}", args.local_path, args.remote_path);
        let host_label = target_trimmed.unwrap_or("").to_owned();

        let res = if use_bridge {
            let bridge = self
                .ctx
                .caps
                .app_bridge
                .as_ref()
                .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
            let action = match args.direction {
                TransferDirection::Upload => "upload",
                TransferDirection::Download => "download",
            };
            let payload = json!({
                "target": target_trimmed,
                "local_path": args.local_path,
                "remote_path": args.remote_path,
                "overwrite": args.overwrite,
            });
            bridge
                .invoke(AppBridgeDomain::TransferFile, action, payload)
                .await
        } else {
            let fs = self
                .ctx
                .caps
                .remote_fs
                .as_ref()
                .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
            fs.transfer(
                args.direction,
                &args.local_path,
                &args.remote_path,
                args.overwrite,
                self.ctx.cancel.clone(),
            )
            .await
        };

        let elapsed = started.elapsed().as_millis() as u64;
        match res {
            Ok(out) => {
                self.ctx.record_card(crate::tools::ToolExecutionCard {
                    tool_name: Self::NAME.to_owned(),
                    host_label,
                    risk: ToolRisk::Write,
                    summary,
                    output: out.clone(),
                    ok: true,
                    approved: true,
                    duration_ms: elapsed,
                    diff_preview: None,
                    ..Default::default()
                });
                Ok(out)
            }
            Err(err) => {
                self.ctx.record_card(crate::tools::ToolExecutionCard {
                    tool_name: Self::NAME.to_owned(),
                    host_label,
                    risk: ToolRisk::Write,
                    summary,
                    output: err.to_string(),
                    ok: false,
                    approved: true,
                    duration_ms: elapsed,
                    diff_preview: None,
                    ..Default::default()
                });
                Err(to_rig_err(err))
            }
        }
    }
}

pub use super::runtime_ops::{ContainerOpsTool, InspectHostRuntimeTool};
