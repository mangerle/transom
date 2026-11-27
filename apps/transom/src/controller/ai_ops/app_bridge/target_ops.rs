//! 统一目标执行层：为原生 `remote_exec`、`remote_fs`、`transfer_file` 提供本机与跨会话远端路由。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use transom_ai::{
    RemoteExecutor, RemoteFsExecutor, ToolError, ToolRisk, TransferDirection,
    classify_shell_command, detect_blocking_command, format_blocking_command_hint,
};
use transom_core::AiManagedTaskStatus;

use super::ControllerAppActionBridge;
use super::local_ops::{
    MAX_LOCAL_READ_BYTES, exec_local_shell, format_exit_output, handle_local_fs, req_str,
};
use super::target_resolver::{
    ensure_remote_dir, ensure_sftp, extract_remote_target, guard_readonly, is_local_target,
    resolve_or_connect_target,
};
use crate::controller::ai_ops::bridge::{SshRemoteExecutor, SshRemoteFsBridge};

/// 统一处理 `remote_exec`（支持 `target = "local"`、远端目标自动连接或当前活动标签）。
pub(super) async fn handle_remote_exec(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let command = req_str(args, "command", "remote_exec")?;
    if let Some(hit) = detect_blocking_command(command) {
        return Ok(format_blocking_command_hint(&hit));
    }
    let default_timeout = bridge.deps.runtime.command_timeout.as_secs().max(30);
    let timeout_secs = args["timeout_secs"]
        .as_u64()
        .unwrap_or(default_timeout)
        .clamp(5, 600);

    if is_local_target(args) {
        return exec_local_shell(bridge, command, args, Duration::from_secs(timeout_secs)).await;
    }

    let target = extract_remote_target(args);
    let session = resolve_or_connect_target(bridge, target, timeout_secs).await?;
    let risk = classify_shell_command(command);
    guard_readonly(&session, risk != ToolRisk::SafeRead)?;
    let exec = SshRemoteExecutor {
        remote: session.remote.clone(),
        tasks: Some(bridge.deps.runtime.tasks.clone()),
        host_label: session.meta.host_name.clone(),
    };
    let out = exec
        .exec(
            command,
            Duration::from_secs(timeout_secs),
            bridge.deps.runtime.cancel.clone(),
        )
        .await?;
    Ok(format_exit_output(out.exit_code, &out.stdout, &out.stderr))
}

/// 统一处理 `remote_fs`（支持本机与跨会话远端文件的 8 种动作）。
pub(super) async fn handle_remote_fs(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    let path = req_str(args, "path", "remote_fs")?;
    if is_local_target(args) {
        if action == "edit" {
            crate::controller::ai_ops::checkpoint_ops::record_local_backup(&bridge.deps, path)
                .await?;
        }
        return handle_local_fs(action, path, args).await;
    }

    let target = extract_remote_target(args);
    let session = resolve_or_connect_target(bridge, target, 30).await?;
    let is_write = matches!(action, "edit" | "mkdir" | "rename" | "delete" | "chmod");
    guard_readonly(&session, is_write)?;
    let fs = SshRemoteFsBridge {
        remote: session.remote.clone(),
        sftp: Some(ensure_sftp(&session).await?),
    };
    let io = bridge.io_context();

    match action {
        "list" => fs.list_dir_with_io(path, &io).await,
        "read" => fs.read_file_with_io(path, MAX_LOCAL_READ_BYTES, &io).await,
        "search" => {
            fs.search_text_with_io(path, args["pattern"].as_str().unwrap_or(""), &io)
                .await
        }
        "mkdir" => {
            fs.mkdir_with_io(path, &io).await?;
            Ok(format!("已创建目录: {path}"))
        }
        "edit" => {
            crate::controller::ai_ops::checkpoint_ops::record_remote_backup(
                &bridge.deps,
                &session.meta.host_name,
                path,
                &fs,
            )
            .await?;
            let content = args["content"].as_str().unwrap_or("");
            if let Some((parent, _)) = path.rsplit_once('/')
                && !parent.is_empty()
            {
                ensure_remote_dir(&session.remote, parent).await?;
            }
            fs.write_file_with_io(path, content, true, &io).await?;
            Ok(format!("已写入文件并完成备份: {path}"))
        }
        "rename" => {
            let to = req_str(args, "target_path", "remote_fs")?;
            fs.rename_with_io(path, to, &io).await?;
            Ok(format!("已重命名/移动: {path} -> {to}"))
        }
        "delete" => {
            let rec = args["recursive"].as_bool().unwrap_or(false);
            fs.delete_with_io(path, rec, &io).await?;
            Ok(format!("已删除远端路径: {path}"))
        }
        "chmod" => {
            let raw = req_str(args, "mode", "remote_fs")?;
            let mode = u32::from_str_radix(raw.trim_start_matches('0'), 8).map_err(|_| {
                ToolError::InvalidArguments {
                    tool: "remote_fs",
                    reason: format!("无效的八进制权限: {raw}"),
                }
            })?;
            fs.chmod_with_io(path, mode, &io).await?;
            Ok(format!("已修改权限 {raw}: {path}"))
        }
        other => Err(ToolError::InvalidArguments {
            tool: "remote_fs",
            reason: format!("未知 remote_fs 动作: {other}"),
        }),
    }
}

/// 统一处理 `transfer_file`（支持自动连接目标服务器与整个目录递归上传）。
pub(super) async fn handle_transfer_file(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    let direction = match action {
        "download" => TransferDirection::Download,
        _ => TransferDirection::Upload,
    };
    let local_raw = req_str(args, "local_path", "transfer_file")?;
    let remote_raw = req_str(args, "remote_path", "transfer_file")?;
    let overwrite = args["overwrite"].as_bool().unwrap_or(true);

    let target = extract_remote_target(args);
    let session = resolve_or_connect_target(bridge, target, 30).await?;

    if direction == TransferDirection::Download {
        let fs = SshRemoteFsBridge {
            remote: session.remote.clone(),
            sftp: Some(ensure_sftp(&session).await?),
        };
        let io = bridge.io_context();
        return fs
            .transfer_with_io(
                TransferDirection::Download,
                local_raw,
                remote_raw,
                overwrite,
                &io,
            )
            .await;
    }

    guard_readonly(&session, true)?;
    let local = Path::new(local_raw);
    if !local.exists() {
        return Err(ToolError::InvalidArguments {
            tool: "transfer_file",
            reason: format!("本机路径不存在: {local_raw}"),
        });
    }
    let sftp = ensure_sftp(&session).await?;
    let remote_root = remote_raw.trim_end_matches('/');

    if local.is_file() {
        if !overwrite && sftp.stat(remote_root).await.is_ok() {
            return Err(ToolError::ExecutionFailed {
                tool: "transfer_file",
                reason: format!("远端目标已存在且未允许覆盖: {remote_root}"),
            });
        }
        if let Some((parent, _)) = remote_root.rsplit_once('/')
            && !parent.is_empty()
        {
            ensure_remote_dir(&session.remote, parent).await?;
        }
        let bytes = sftp
            .upload(local, remote_root, |_, _| {})
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool: "transfer_file",
                reason: e.to_string(),
            })?;
        return Ok(format!(
            "上传完成（{}）: {local_raw} -> {remote_root}（{bytes} 字节）",
            session.meta.host_name
        ));
    }

    // 目录上传：采用显式栈迭代遍历，严禁递归爆栈，跳过 .git 目录，并纳入托管任务实时推送进度。
    let task = bridge.deps.runtime.tasks.start_task(
        "transfer",
        &format!("上传目录 {local_raw}"),
        &session.meta.host_name,
        &bridge.deps.runtime.cancel,
        None,
    );
    ensure_remote_dir(&session.remote, remote_root).await?;
    let mut stack: Vec<(PathBuf, String)> = vec![(local.to_path_buf(), remote_root.to_owned())];
    let mut file_count = 0usize;
    let mut total_bytes = 0u64;

    while let Some((cur_local, cur_remote)) = stack.pop() {
        if task.cancel.is_cancelled() {
            task.finish(AiManagedTaskStatus::Cancelled, None, "目录上传已取消");
            return Err(ToolError::Cancelled);
        }
        let mut rd =
            tokio::fs::read_dir(&cur_local)
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    tool: "transfer_file",
                    reason: e.to_string(),
                })?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool: "transfer_file",
                reason: e.to_string(),
            })?
        {
            if task.cancel.is_cancelled() {
                task.finish(AiManagedTaskStatus::Cancelled, None, "目录上传已取消");
                return Err(ToolError::Cancelled);
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".git" {
                continue;
            }
            let child_local = entry.path();
            let child_remote = format!("{cur_remote}/{name}");
            let meta = entry
                .metadata()
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    tool: "transfer_file",
                    reason: e.to_string(),
                })?;
            if meta.is_dir() {
                ensure_remote_dir(&session.remote, &child_remote).await?;
                stack.push((child_local, child_remote));
            } else if meta.is_file() {
                if !overwrite && sftp.stat(&child_remote).await.is_ok() {
                    continue;
                }
                let bytes = sftp
                    .upload(&child_local, &child_remote, |_, _| {})
                    .await
                    .map_err(|e| ToolError::ExecutionFailed {
                        tool: "transfer_file",
                        reason: format!("上传 {} 失败: {e}", child_local.display()),
                    })?;
                file_count += 1;
                total_bytes = total_bytes.saturating_add(bytes);
                task.emit_chunk(&format!("[{file_count}] {child_remote} ({bytes} B)\n"));
            }
        }
    }

    let summary = format!(
        "目录上传完成（{}）: {local_raw} -> {remote_root}（共 {file_count} 个文件，{total_bytes} 字节）",
        session.meta.host_name
    );
    task.finish(AiManagedTaskStatus::Succeeded, Some(0), &summary);
    Ok(summary)
}
