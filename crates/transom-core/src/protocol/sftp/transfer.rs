//! SFTP 传输方向与目录递归汇总：跨 net 与核心同步模型共享的契约。
//!
//! 设计原理（Why）：`TransferDirection` 被 net 的目录传输与 core 的增量同步
//! 差异项共同引用，`TransferSummary` 是目录递归操作的返回值；二者属于跨层
//! 契约，因此留在 core。只描述界面队列状态的 `TransferTask` 系列模型已随
//! 控制层不感知队列的边界迁至 transom-ui。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferDirection {
    Download,
    Upload,
    /// 目录双向增量同步：单个任务内同时包含上传与下载项
    Sync,
    /// 跨远端流式互传（两台远端服务器之间流式复制）
    RemoteCopy,
}

/// SFTP 传输校验策略级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VerifyMode {
    /// 完整密码学校验：逐字节计算并比对两端摘要。
    #[default]
    FullSha256,
    /// 快速抽样校验：读取文件首尾与中间采样块计算指纹，适合超大文件的极速完整性判定。
    QuickSample,
}

/// 支持的校验和算法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ChecksumAlgorithm {
    /// SHA-256 算法。
    #[default]
    Sha256,
    /// MD5 算法。
    Md5,
}

/// 目录递归传输的汇总结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransferSummary {
    /// 实际完成（含断点续传已存在）的字节数
    pub transferred: u64,
    /// 被排除规则或符号链接跳过的条目数
    pub skipped: u64,
    /// 是否请求了 SHA256 校验且全部文件校验通过
    pub verified: bool,
}
