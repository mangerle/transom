//! SFTP 传输方向与目录递归汇总：跨 net 与核心同步模型共享的契约。
//!
//! 设计原理（Why）：`TransferDirection` 被 net 的目录传输与 core 的增量同步
//! 差异项共同引用，`TransferSummary` 是目录递归操作的返回值；传输起点
//! `TransferStart` 与校验选项 `VerifyOptions` 是命令载荷与 net 执行层之间的
//! 契约，三者也属于跨层契约，因此留在 core。只描述界面队列状态的
//! `TransferTask` 系列模型已随控制层不感知队列的边界迁至 transom-ui。

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

/// 单文件传输的起点。
///
/// 设计原理（Why）：起点曾经由调用方传一个 `resume: bool` 表达，传输层再拿
/// “目标端已有字节数”反推断点。于是“目标端已存在同名文件”被误判成“上次
/// 中断的残留”：尺寸恰好相同就直接跳过传输，尺寸更短就把源文件尾部追加到
/// 旧内容之后，最终落盘的是一个既不等于源、也不等于旧版的脏文件，只能由
/// 传输后校验才发现。断点的唯一可信来源是本任务自己记录过的进度，目标端
/// 恰好存在什么内容与断点无关，因此起点必须由持有任务状态的调用方显式声明。
///
/// 代价与局限：`Continue` 信任调用方给出的偏移对应的前缀确实是本任务此前的
/// 写入；若源文件在中断期间被外部修改，前缀可能与源内容不一致，需要依赖
/// 传输后校验发现并从头覆盖重传。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransferStart {
    /// 覆盖：目标端已有内容一律丢弃，从第 0 字节写入。
    #[default]
    Overwrite,
    /// 续传：目标端前 `offset` 字节是本任务此前的连续写入，从其后继续。
    Continue {
        /// 本任务已经写入目标端的字节数。
        offset: u64,
    },
}

impl TransferStart {
    /// 起始偏移；`Overwrite` 恒为 0。
    pub fn offset(self) -> u64 {
        match self {
            Self::Overwrite => 0,
            Self::Continue { offset } => offset,
        }
    }
}

/// 传输后完整性校验的选项。
///
/// 设计原理（Why）：是否校验、校验策略级别与哈希算法三者必须同时成立才有
/// 意义，“不校验却指定抽样策略”是非法组合；收敛成一个类型后，操作命令与
/// 调用点都无法再拼出互相矛盾的三个独立字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VerifyOptions {
    /// 传输完成后是否执行校验。
    pub enabled: bool,
    /// 校验策略级别（完整校验或快速抽样），仅在 `enabled` 为真时生效。
    pub mode: VerifyMode,
    /// 校验使用的哈希算法，仅在 `enabled` 为真时生效。
    pub algorithm: ChecksumAlgorithm,
}

impl VerifyOptions {
    /// 按指定策略与算法开启校验。
    pub fn enabled(mode: VerifyMode, algorithm: ChecksumAlgorithm) -> Self {
        Self {
            enabled: true,
            mode,
            algorithm,
        }
    }
}

/// 支持的校验和算法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ChecksumAlgorithm {
    /// SHA-256 算法（安全性最高，密码学推荐）。
    #[default]
    Sha256,
    /// MD5 算法（兼容性最好，绝大多数 Linux/Unix 均内置 md5sum）。
    Md5,
}

impl ChecksumAlgorithm {
    /// 获取算法的标准简称，用于 UI 徽标展示。
    pub fn label(&self) -> &'static str {
        match self {
            Self::Sha256 => "SHA256",
            Self::Md5 => "MD5",
        }
    }
}

impl std::fmt::Display for ChecksumAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
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
