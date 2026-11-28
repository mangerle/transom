//! AI 工具后端执行器抽象 Trait 定义。
//!
//! 设计原理（Why）：
//! - 通过抽象 Trait 解耦底层 `SshRemote` / `SftpClient` / `ContainerService` / GPUI 终端，
//!   使得 `transom-ai` 可在无网络、无 GUI 的单元测试中完整运行与验证；
//! - 所有网络/远端方法均提供 `*_with_io` 统一超时与取消包装，避免各工具重复编写 `select!`。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::{ToolIoContext, with_io_guard};
use transom_ai_contracts::error::ToolError;
use transom_ai_contracts::memory::MemoryProposal;

/// 远端命令执行结果。
#[derive(Clone, Debug)]
pub struct ExecCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// 远端命令实时输出进度回调。
pub type ExecProgressCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// 远端命令执行器抽象（由控制平面复用 `SshRemote` 实现）。
#[async_trait]
pub trait RemoteExecutor: Send + Sync {
    async fn exec(
        &self,
        command: &str,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Result<ExecCommandResult, ToolError>;

    /// 带实时输出流预览的远端命令执行（默认回退到 `exec`）。
    async fn exec_with_progress(
        &self,
        command: &str,
        timeout: Duration,
        cancel: CancellationToken,
        _on_progress: ExecProgressCallback,
    ) -> Result<ExecCommandResult, ToolError> {
        self.exec(command, timeout, cancel).await
    }
}

/// 远端文件系统与传输执行器抽象（由控制平面复用 `SftpClient` + `sudo_write` 实现）。
#[async_trait]
pub trait RemoteFsExecutor: Send + Sync {
    async fn list_dir(&self, path: &str) -> Result<String, ToolError>;
    /// 探测远端路径是否存在。
    ///
    /// 与直接读取文件不同，调用方必须能区分“确认不存在”和“探测失败”，
    /// 否则检查点可能把权限错误误记为待删除的新文件。
    async fn file_exists(&self, path: &str) -> Result<bool, ToolError>;
    async fn read_file(&self, path: &str, max_bytes: usize) -> Result<String, ToolError>;
    async fn write_file(&self, path: &str, content: &str, backup: bool) -> Result<(), ToolError>;
    async fn mkdir(&self, path: &str) -> Result<(), ToolError>;
    async fn search_text(&self, path: &str, pattern: &str) -> Result<String, ToolError>;
    async fn transfer(
        &self,
        direction: TransferDirection,
        local_path: &str,
        remote_path: &str,
        overwrite: bool,
        cancel: CancellationToken,
    ) -> Result<String, ToolError>;
    async fn rename(&self, _from: &str, _to: &str) -> Result<(), ToolError> {
        Err(ToolError::ExecutionFailed {
            tool: "remote_fs",
            reason: "当前后端未实现远端重命名/移动".to_owned(),
        })
    }
    async fn delete(&self, _path: &str, _recursive: bool) -> Result<(), ToolError> {
        Err(ToolError::ExecutionFailed {
            tool: "remote_fs",
            reason: "当前后端未实现远端删除".to_owned(),
        })
    }
    async fn chmod(&self, _path: &str, _mode: u32) -> Result<(), ToolError> {
        Err(ToolError::ExecutionFailed {
            tool: "remote_fs",
            reason: "当前后端未实现远端权限修改".to_owned(),
        })
    }

    /// 带统一截止时间与取消门的目录读取。
    async fn list_dir_with_io(&self, path: &str, io: &ToolIoContext) -> Result<String, ToolError> {
        with_io_guard("remote_fs", io, self.list_dir(path)).await
    }

    /// 带统一截止时间与取消门的路径探测。
    async fn file_exists_with_io(&self, path: &str, io: &ToolIoContext) -> Result<bool, ToolError> {
        with_io_guard("remote_fs", io, self.file_exists(path)).await
    }

    /// 带统一截止时间与取消门的文件读取。
    async fn read_file_with_io(
        &self,
        path: &str,
        max_bytes: usize,
        io: &ToolIoContext,
    ) -> Result<String, ToolError> {
        with_io_guard("remote_fs", io, self.read_file(path, max_bytes)).await
    }

    /// 带统一截止时间与取消门的文件写入。
    async fn write_file_with_io(
        &self,
        path: &str,
        content: &str,
        backup: bool,
        io: &ToolIoContext,
    ) -> Result<(), ToolError> {
        with_io_guard("remote_fs", io, self.write_file(path, content, backup)).await
    }

    /// 带统一截止时间与取消门的目录创建。
    async fn mkdir_with_io(&self, path: &str, io: &ToolIoContext) -> Result<(), ToolError> {
        with_io_guard("remote_fs", io, self.mkdir(path)).await
    }

    /// 带统一截止时间与取消门的文本检索。
    async fn search_text_with_io(
        &self,
        path: &str,
        pattern: &str,
        io: &ToolIoContext,
    ) -> Result<String, ToolError> {
        with_io_guard("remote_fs", io, self.search_text(path, pattern)).await
    }

    /// 带统一截止时间与取消门的文件传输。
    async fn transfer_with_io(
        &self,
        direction: TransferDirection,
        local_path: &str,
        remote_path: &str,
        overwrite: bool,
        io: &ToolIoContext,
    ) -> Result<String, ToolError> {
        with_io_guard(
            "transfer_file",
            io,
            self.transfer(
                direction,
                local_path,
                remote_path,
                overwrite,
                io.cancel.clone(),
            ),
        )
        .await
    }

    /// 带统一截止时间与取消门的重命名。
    async fn rename_with_io(
        &self,
        from: &str,
        to: &str,
        io: &ToolIoContext,
    ) -> Result<(), ToolError> {
        with_io_guard("remote_fs", io, self.rename(from, to)).await
    }

    /// 带统一截止时间与取消门的删除。
    async fn delete_with_io(
        &self,
        path: &str,
        recursive: bool,
        io: &ToolIoContext,
    ) -> Result<(), ToolError> {
        with_io_guard("remote_fs", io, self.delete(path, recursive)).await
    }

    /// 带统一截止时间与取消门的权限修改。
    async fn chmod_with_io(
        &self,
        path: &str,
        mode: u32,
        io: &ToolIoContext,
    ) -> Result<(), ToolError> {
        with_io_guard("remote_fs", io, self.chmod(path, mode)).await
    }
}

/// 传输方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    Upload,
    Download,
}

/// 终端上下文提供者（读取 OSC 133 命令块或当前屏幕末尾行）。
#[async_trait]
pub trait TerminalContextProvider: Send + Sync {
    async fn recent_blocks(&self, limit: usize) -> Result<String, ToolError>;
    async fn read_screen_tail(&self, lines: usize) -> Result<String, ToolError>;
}

/// 运行时监控与容器诊断执行器。
#[async_trait]
pub trait RuntimeInspector: Send + Sync {
    async fn host_runtime_snapshot(&self) -> Result<String, ToolError>;
    async fn container_action(
        &self,
        action: &str,
        container_id: Option<&str>,
        tail: Option<usize>,
    ) -> Result<String, ToolError>;

    /// 带统一截止时间与取消门的主机运行时快照。
    async fn host_runtime_snapshot_with_io(&self, io: &ToolIoContext) -> Result<String, ToolError> {
        with_io_guard("inspect_host_runtime", io, self.host_runtime_snapshot()).await
    }

    /// 带统一截止时间与取消门的容器动作。
    async fn container_action_with_io(
        &self,
        action: &str,
        container_id: Option<&str>,
        tail: Option<usize>,
        io: &ToolIoContext,
    ) -> Result<String, ToolError> {
        with_io_guard(
            "container_ops",
            io,
            self.container_action(action, container_id, tail),
        )
        .await
    }
}

/// 端口转发执行器。
#[async_trait]
pub trait ForwardExecutor: Send + Sync {
    async fn list_forwards(&self) -> Result<String, ToolError>;
    async fn create_forward(
        &self,
        listen_port: u16,
        target_host: &str,
        target_port: u16,
    ) -> Result<String, ToolError>;
    async fn stop_forward(&self, forward_id: &str) -> Result<String, ToolError>;
}

/// 记忆写入提议处理器。
#[async_trait]
pub trait MemoryWriter: Send + Sync {
    async fn propose_host_memory(&self, proposal: MemoryProposal) -> Result<String, ToolError>;
}

/// 本地知识库（长期记忆与自定义 Runbook）BM25 检索引擎抽象。
#[async_trait]
pub trait KnowledgeSearcher: Send + Sync {
    async fn search(&self, query: &str, top_k: usize) -> Result<String, ToolError>;
}
