//! 远端命令执行器与托管任务桥接。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use transom_ai::{ExecCommandResult, RemoteExecutor, ToolError};
use transom_core::AiManagedTaskStatus;
use transom_net::SshRemote;

use super::super::task_runner::{
    ManagedTaskRegistry, kill_remote_pid, strip_pgid_marker, wrap_remote_command_with_pgid,
};

/// 复用 `SshRemote` 独立 exec 通道执行远端命令（全量纳管至托管任务中心）。
pub(in crate::controller) struct SshRemoteExecutor {
    pub(in crate::controller) remote: SshRemote,
    pub(in crate::controller) tasks: Option<Arc<ManagedTaskRegistry>>,
    pub(in crate::controller) host_label: String,
}

#[async_trait]
impl RemoteExecutor for SshRemoteExecutor {
    async fn exec(
        &self,
        command: &str,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Result<ExecCommandResult, ToolError> {
        self.exec_inner(command, timeout, cancel, None).await
    }

    async fn exec_with_progress(
        &self,
        command: &str,
        timeout: Duration,
        cancel: CancellationToken,
        on_progress: transom_ai::tools::ExecProgressCallback,
    ) -> Result<ExecCommandResult, ToolError> {
        self.exec_inner(command, timeout, cancel, Some(on_progress))
            .await
    }
}

impl SshRemoteExecutor {
    async fn exec_inner(
        &self,
        command: &str,
        timeout: Duration,
        cancel: CancellationToken,
        on_progress: Option<transom_ai::tools::ExecProgressCallback>,
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
        let progress_state = on_progress.map(|cb| {
            Arc::new(Mutex::new((
                String::new(),
                std::time::Instant::now() - Duration::from_millis(300),
                cb,
            )))
        });
        tasks.spawn_background(async move {
            let res = remote_bg
                .exec_command_stream(&wrapped, timeout, task_cancel.clone(), |chunk, _is_err| {
                    task.emit_chunk(chunk);
                    if let Some(state) = &progress_state {
                        let maybe_snapshot = {
                            let mut guard = state.lock();
                            guard.0.push_str(chunk);
                            // 仅保留末尾 2KB 缓冲，避免长输出内存积压
                            if guard.0.len() > 2048 {
                                let keep_from = guard.0.len().saturating_sub(2048);
                                let mut valid_start = keep_from;
                                while !guard.0.is_char_boundary(valid_start) {
                                    valid_start += 1;
                                }
                                guard.0.drain(..valid_start);
                            }
                            if guard.1.elapsed() >= Duration::from_millis(250) {
                                guard.1 = std::time::Instant::now();
                                Some((guard.0.clone(), guard.2.clone()))
                            } else {
                                None
                            }
                        };
                        if let Some((buf, cb)) = maybe_snapshot {
                            cb(&buf);
                        }
                    }
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
                        tasks_bg.spawn_background(kill_remote_pid(remote, pid));
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
            if tx.send(mapped).is_err() {
                log::debug!("远端托管命令结果接收方已取消，丢弃最终结果");
            }
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
