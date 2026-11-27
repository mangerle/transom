//! 远端文件读写、目录操作与传输桥接。

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use transom_ai::{RemoteFsExecutor, ToolError, TransferDirection};
use transom_net::{SftpClient, SshRemote};

use super::super::runtime_bridge::shell_quote;

/// 远端文件与传输桥接（优先使用 SFTP，不可用时回退至安全受控的 SSH 命令）。
pub(in crate::controller) struct SshRemoteFsBridge {
    pub(in crate::controller) remote: SshRemote,
    pub(in crate::controller) sftp: Option<SftpClient>,
}

#[async_trait]
impl RemoteFsExecutor for SshRemoteFsBridge {
    async fn file_exists(&self, path: &str) -> Result<bool, ToolError> {
        super::super::remote_fs_probe::remote_file_exists(self, path).await
    }

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
