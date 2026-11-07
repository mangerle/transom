//! 应用分领域命令枚举定义。
//!
//! 将全局单一的 AppCommand 按照业务领域归类为分领域子命令：
//! - TerminalCommand: Connect, CloseTab, Shutdown
//! - SftpCommand: Sftp（文件操作）, SftpCancel（取消任务）
//! - ForwardCommand: ForwardStart, ForwardStop
//! - HostCommand: DeployPublicKey, RunHostActions, SampleProcesses, SampleProcessCount, KillProcess
//! - SyncCommand: CloudSyncPush, CloudSyncPull, GitSync
//! - ZmodemCommand: ZmodemUpload, ZmodemCancel
//! - ContainerCommand: 引擎工作区与容器/镜像/卷/网络操作
//!
//! 容器命令体量与其它领域差异明显，单独成文件；其余领域放在 session.rs。

mod container;
mod session;

pub use container::ContainerCommand;
pub use session::{
    ConnectParams, ForwardCommand, HostCommand, SftpCommand, SyncCommand, TerminalCommand,
    ZmodemCommand,
};
