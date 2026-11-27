//! 控制平面与 `transom-ai` 后端工具执行器之间的桥接实现。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use transom_ai::{
    ExecCommandResult, RemoteExecutor, RemoteFsExecutor, TerminalContextProvider, ToolError,
    TransferDirection,
};
use transom_core::{AiAppEvent, AiManagedTaskStatus, AiTerminalContextKind, AppEvent, TabId};
use transom_net::{SftpClient, SshRemote};
use uuid::Uuid;

use super::runtime_bridge::shell_quote;
use super::task_runner::{
    ManagedTaskRegistry, kill_remote_pid, strip_pgid_marker, wrap_remote_command_with_pgid,
};

/// 复用 `SshRemote` 独立 exec 通道执行远端命令（全量纳管至托管任务中心）。
pub(super) struct SshRemoteExecutor {
    pub(super) remote: SshRemote,
    pub(super) tasks: Option<Arc<ManagedTaskRegistry>>,
    pub(super) host_label: String,
}

#[async_trait]
impl RemoteExecutor for SshRemoteExecutor {
    async fn exec(
        &self,
        command: &str,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Result<ExecCommandResult, ToolError> {
        let Some(tasks) = &self.tasks else {
            let fut = self.remote.exec_command_capture(command);
            return tokio::select! {
                _ = cancel.cancelled() => Err(ToolError::Cancelled),
                res = tokio::time::timeout(timeout, fut) => {
                    match res {
                        Ok(Ok(out)) => Ok(ExecCommandResult {
                            stdout: out.stdout,
                            stderr: out.stderr,
                            exit_code: out.exit_status.map(|c| c as i32).unwrap_or(-1),
                        }),
                        Ok(Err(err)) => Err(ToolError::ExecutionFailed {
                            tool: "remote_exec",
                            reason: err.to_string(),
                        }),
                        Err(_) => Err(ToolError::ExecutionTimeout {
                            timeout_secs: timeout.as_secs(),
                        }),
                    }
                }
            };
        };

        let title = command.chars().take(60).collect::<String>();
        let task = tasks.start_task(
            "remote_exec",
            &title,
            &self.host_label,
            &cancel,
            Some(self.remote.clone()),
        );
        let wrapped = wrap_remote_command_with_pgid(command);
        let remote_bg = self.remote.clone();
        let tasks_bg = tasks.clone();
        let task_id = task.task_id.clone();
        let task_cancel = task.cancel.clone();
        let task_detach = task.detach.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let res = remote_bg
                .exec_command_stream(&wrapped, timeout, task_cancel.clone(), |chunk, _is_err| {
                    task.emit_chunk(chunk);
                })
                .await;
            let mapped = match res {
                Ok(out) => {
                    let code = out.exit_status.map(|c| c as i32).unwrap_or(-1);
                    let status = if code == 0 {
                        AiManagedTaskStatus::Succeeded
                    } else {
                        AiManagedTaskStatus::Failed
                    };
                    task.finish(status, Some(code), format!("退出码: {code}"));
                    Ok(ExecCommandResult {
                        stdout: strip_pgid_marker(&out.stdout),
                        stderr: out.stderr,
                        exit_code: code,
                    })
                }
                Err(_) if task_cancel.is_cancelled() => {
                    if let Some((remote, pid)) = tasks_bg.cancel_task(&task.task_id) {
                        tokio::spawn(kill_remote_pid(remote, pid));
                    }
                    task.finish(AiManagedTaskStatus::Cancelled, None, "远端命令已取消");
                    Err(ToolError::Cancelled)
                }
                Err(err) => {
                    let reason = err.to_string();
                    task.finish(AiManagedTaskStatus::Failed, None, &reason);
                    Err(map_remote_exec_err(&reason, timeout))
                }
            };
            let _ = tx.send(mapped);
        });

        tokio::select! {
            _ = task_detach.cancelled() => Ok(ExecCommandResult {
                stdout: format!(
                    "命令已转入后台托管运行（task_id: {task_id}），可在任务中心实时查看流式日志或随时停止，主对话可继续进行。"
                ),
                stderr: String::new(),
                exit_code: 0,
            }),
            res = rx => match res {
                Ok(inner) => inner,
                Err(_) => Err(ToolError::ExecutionFailed {
                    tool: "remote_exec",
                    reason: "后台远端命令通道异常关闭".to_owned(),
                }),
            }
        }
    }
}

/// 远端文件与传输桥接（优先使用 SFTP，不可用时回退至安全受控的 SSH 命令）。
pub(super) struct SshRemoteFsBridge {
    pub(super) remote: SshRemote,
    pub(super) sftp: Option<SftpClient>,
}

#[async_trait]
impl RemoteFsExecutor for SshRemoteFsBridge {
    async fn list_dir(&self, path: &str) -> Result<String, ToolError> {
        if let Some(sftp) = &self.sftp {
            let (_resolved, entries) =
                sftp.list(path)
                    .await
                    .map_err(|err| ToolError::ExecutionFailed {
                        tool: "remote_fs",
                        reason: err.to_string(),
                    })?;
            let lines: Vec<String> = entries
                .into_iter()
                .take(200)
                .map(|entry| {
                    let kind = if entry.is_dir { "DIR " } else { "FILE" };
                    format!("{kind} {:>10} {}", entry.size, entry.name)
                })
                .collect();
            return Ok(lines.join("\n"));
        }
        let quoted = shell_quote(path);
        let out = self
            .remote
            .exec_command_capture(&format!("ls -la {quoted}"))
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: err.to_string(),
            })?;
        Ok(out.stdout)
    }

    async fn read_file(&self, path: &str, max_bytes: usize) -> Result<String, ToolError> {
        if let Some(sftp) = &self.sftp {
            let (text, _meta) = sftp
                .read_text(path, max_bytes as u64)
                .await
                .map_err(|err| ToolError::ExecutionFailed {
                    tool: "remote_fs",
                    reason: err.to_string(),
                })?;
            return Ok(text);
        }
        let quoted = shell_quote(path);
        let out = self
            .remote
            .exec_command_capture(&format!("head -c {max_bytes} {quoted}"))
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: err.to_string(),
            })?;
        Ok(out.stdout)
    }

    async fn write_file(&self, path: &str, content: &str, backup: bool) -> Result<(), ToolError> {
        if let Some(sftp) = &self.sftp {
            sftp.write_text(path, content.as_bytes(), None, true, backup)
                .await
                .map_err(|err| ToolError::ExecutionFailed {
                    tool: "remote_fs",
                    reason: err.to_string(),
                })?;
            return Ok(());
        }
        Err(ToolError::ExecutionFailed {
            tool: "remote_fs",
            reason: "当前会话 SFTP 通道未就绪，无法安全写入文件".to_owned(),
        })
    }

    async fn mkdir(&self, path: &str) -> Result<(), ToolError> {
        let quoted = shell_quote(path);
        let out = self
            .remote
            .exec_command_capture(&format!("mkdir -p {quoted}"))
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: err.to_string(),
            })?;
        if out.exit_status == Some(0) {
            Ok(())
        } else {
            Err(ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: out.stderr,
            })
        }
    }

    async fn search_text(&self, path: &str, pattern: &str) -> Result<String, ToolError> {
        let quoted_path = shell_quote(path);
        let quoted_pat = shell_quote(pattern);
        let out = self
            .remote
            .exec_command_capture(&format!(
                "grep -rn -- {quoted_pat} {quoted_path} | head -n 120"
            ))
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: err.to_string(),
            })?;
        Ok(out.stdout)
    }

    async fn transfer(
        &self,
        direction: TransferDirection,
        local_path: &str,
        remote_path: &str,
        overwrite: bool,
        cancel: CancellationToken,
    ) -> Result<String, ToolError> {
        let Some(sftp) = &self.sftp else {
            return Err(ToolError::ExecutionFailed {
                tool: "transfer_file",
                reason: "当前会话 SFTP 通道未就绪".to_owned(),
            });
        };
        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        match direction {
            TransferDirection::Upload => {
                let local = std::path::Path::new(local_path);
                if !overwrite && sftp.stat(remote_path).await.is_ok() {
                    return Err(ToolError::ExecutionFailed {
                        tool: "transfer_file",
                        reason: format!("远端目标已存在且未允许覆盖: {remote_path}"),
                    });
                }
                let bytes = sftp
                    .upload(local, remote_path, |_, _| {})
                    .await
                    .map_err(|err| ToolError::ExecutionFailed {
                        tool: "transfer_file",
                        reason: err.to_string(),
                    })?;
                Ok(format!(
                    "上传完成: {local_path} -> {remote_path}（{bytes} 字节）"
                ))
            }
            TransferDirection::Download => {
                let local = std::path::Path::new(local_path);
                if !overwrite && local.exists() {
                    return Err(ToolError::ExecutionFailed {
                        tool: "transfer_file",
                        reason: format!("本机目标已存在且未允许覆盖: {local_path}"),
                    });
                }
                if let Some(parent) = local.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|err| {
                        ToolError::ExecutionFailed {
                            tool: "transfer_file",
                            reason: err.to_string(),
                        }
                    })?;
                }
                let bytes = sftp
                    .download(remote_path, local, |_, _| {})
                    .await
                    .map_err(|err| ToolError::ExecutionFailed {
                        tool: "transfer_file",
                        reason: err.to_string(),
                    })?;
                Ok(format!(
                    "下载完成: {remote_path} -> {local_path}（{bytes} 字节）"
                ))
            }
        }
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), ToolError> {
        if let Some(sftp) = &self.sftp {
            return sftp
                .rename(from, to)
                .await
                .map_err(|err| ToolError::ExecutionFailed {
                    tool: "remote_fs",
                    reason: err.to_string(),
                });
        }
        let cmd = format!("mv -- {} {}", shell_quote(from), shell_quote(to));
        let out = self
            .remote
            .exec_command_capture(&cmd)
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: err.to_string(),
            })?;
        if out.exit_status == Some(0) {
            Ok(())
        } else {
            Err(ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: out.stderr,
            })
        }
    }

    async fn delete(&self, path: &str, recursive: bool) -> Result<(), ToolError> {
        if let Some(sftp) = &self.sftp
            && !recursive
        {
            let is_dir = sftp.stat(path).await.map(|m| m.is_dir).unwrap_or(false);
            return sftp
                .delete(path, is_dir)
                .await
                .map_err(|err| ToolError::ExecutionFailed {
                    tool: "remote_fs",
                    reason: err.to_string(),
                });
        }
        let flag = if recursive { "-rf" } else { "-f" };
        let cmd = format!("rm {flag} -- {}", shell_quote(path));
        let out = self
            .remote
            .exec_command_capture(&cmd)
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: err.to_string(),
            })?;
        if out.exit_status == Some(0) {
            Ok(())
        } else {
            Err(ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: out.stderr,
            })
        }
    }

    async fn chmod(&self, path: &str, mode: u32) -> Result<(), ToolError> {
        if let Some(sftp) = &self.sftp {
            return sftp
                .chmod(path, mode)
                .await
                .map_err(|err| ToolError::ExecutionFailed {
                    tool: "remote_fs",
                    reason: err.to_string(),
                });
        }
        let cmd = format!("chmod {:o} -- {}", mode, shell_quote(path));
        let out = self
            .remote
            .exec_command_capture(&cmd)
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: err.to_string(),
            })?;
        if out.exit_status == Some(0) {
            Ok(())
        } else {
            Err(ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: out.stderr,
            })
        }
    }
}

/// 通过事件总线向 GPUI 终端实体反向请求 OSC 133 命令块或屏幕内容。
pub(super) struct UiTerminalBridge {
    pub(super) tab_id: TabId,
    pub(super) event_tx: flume::Sender<AppEvent>,
    pub(super) pending: Arc<Mutex<HashMap<String, oneshot::Sender<Option<String>>>>>,
}

#[async_trait]
impl TerminalContextProvider for UiTerminalBridge {
    async fn recent_blocks(&self, limit: usize) -> Result<String, ToolError> {
        self.request(AiTerminalContextKind::RecentBlocks { limit })
            .await
    }

    async fn read_screen_tail(&self, lines: usize) -> Result<String, ToolError> {
        self.request(AiTerminalContextKind::ScreenTail { lines })
            .await
    }
}

impl UiTerminalBridge {
    async fn request(&self, kind: AiTerminalContextKind) -> Result<String, ToolError> {
        let request_id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(request_id.clone(), tx);

        let evt = AppEvent::Ai(Box::new(AiAppEvent::TerminalContextRequested {
            request_id: request_id.clone(),
            tab_id: self.tab_id,
            kind,
        }));
        if self.event_tx.send_async(evt).await.is_err() {
            self.pending.lock().remove(&request_id);
            return Err(ToolError::ExecutionFailed {
                tool: "terminal_context",
                reason: "界面事件通道已关闭".to_owned(),
            });
        }

        match tokio::time::timeout(Duration::from_secs(5), rx).await {
            Ok(Ok(Some(content))) => Ok(content),
            Ok(Ok(None)) => Ok("当前终端暂无可用记录。".to_owned()),
            Ok(Err(_)) | Err(_) => {
                self.pending.lock().remove(&request_id);
                Err(ToolError::ExecutionTimeout { timeout_secs: 5 })
            }
        }
    }
}

pub(super) use super::runtime_bridge::HostRuntimeBridge;

fn map_remote_exec_err(reason: &str, timeout: Duration) -> ToolError {
    if reason.contains("超时") {
        ToolError::ExecutionTimeout {
            timeout_secs: timeout.as_secs(),
        }
    } else {
        ToolError::ExecutionFailed {
            tool: "remote_exec",
            reason: reason.to_owned(),
        }
    }
}
