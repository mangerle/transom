//! Git 仓库接入层的错误类型与到 [`AppError`] 的映射。
//!
//! 设计原理（Why）：
//! - git 是外部子进程，失败原因横跨「环境缺失」「仓库状态」「并发锁」三类，
//!   每类的用户恢复路径不同，因此拆成独立变体而不是统一塞成一条字符串；
//! - 本层只构造错误、不记录日志：日志由控制器在最终消费点统一打印，
//!   避免同一个失败沿调用链重复刷屏；
//! - 对外一律收敛为带稳定错误码的 `AppError`，界面按错误码取本地化文案，
//!   `detail` 只作诊断，不参与控制流判断。

use transom_core::{AppError, ErrorCode};

/// Git 接入层错误。
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// 无法启动 git 进程（未安装、权限不足、PATH 缺失）。
    #[error("无法执行 git 命令，命令: git {command}，原因: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    /// 子进程超时未返回。
    #[error("git 命令执行超时，命令: git {command}，超时秒数: {secs}")]
    Timeout { command: String, secs: u64 },
    /// 子进程返回非零退出码。
    #[error("git 命令失败，命令: git {command}，退出码: {code}，输出: {stderr}")]
    Failed {
        command: String,
        code: i32,
        stderr: String,
    },
    /// 目标目录不是 Git 仓库（含父目录也不在仓库内的情况）。
    #[error("目录不是 Git 仓库，目录: {path}")]
    NotRepo { path: String },
    /// 索引或引用锁被占用：另一个 git 进程正在写仓库。
    #[error("仓库被其他 Git 进程锁定，目录: {path}")]
    Locked { path: String },
    /// 前置条件不满足，本层主动拒绝执行（不是 git 报错）。
    #[error("执行条件不满足，已拒绝操作，原因: {reason}")]
    Precondition { reason: String },
    /// 输出解析失败：格式与预期版本不符。
    #[error("解析 git 输出失败，命令: git {command}，原因: {reason}")]
    Decode { command: String, reason: String },
    /// 调用方给的路径非法（不存在、非目录或含越权片段）。
    #[error("仓库路径非法，路径: {path}，原因: {reason}")]
    InvalidPath { path: String, reason: String },
    /// 单侧文件超过差异查看上限。
    #[error("文件超过差异查看上限，路径: {path}，上限字节: {limit}")]
    TooLarge { path: String, limit: u64 },
    /// 二进制内容无法按文本比较。
    #[error("文件是二进制内容，无法按文本比较，路径: {path}")]
    Binary { path: String },
    /// 操作被用户取消。
    #[error("Git 操作已取消")]
    Cancelled,
}

impl GitError {
    /// 映射到跨层稳定的错误码。
    pub fn to_app_error(&self) -> AppError {
        let code = match self {
            Self::Spawn { .. } => ErrorCode::GitUnavailable,
            Self::Timeout { .. } => ErrorCode::Timeout,
            Self::Failed { .. } | Self::Decode { .. } => ErrorCode::GitCommandFailed,
            Self::NotRepo { .. } => ErrorCode::GitNotARepo,
            Self::Locked { .. } => ErrorCode::GitBusy,
            Self::InvalidPath { .. } => ErrorCode::Configuration,
            Self::TooLarge { .. } | Self::Binary { .. } => ErrorCode::Io,
            Self::Precondition { .. } => ErrorCode::GitCommandFailed,
            Self::Cancelled => ErrorCode::Cancelled,
        };
        AppError::with_detail(code, self.to_string())
    }

    /// 是否为「目录不是仓库」这一条可预期路径，供上层直接走初始化提示。
    pub fn is_not_repo(&self) -> bool {
        matches!(self, Self::NotRepo { .. })
    }
}

impl From<GitError> for AppError {
    fn from(value: GitError) -> Self {
        value.to_app_error()
    }
}

#[cfg(test)]
mod tests {
    use super::GitError;
    use transom_core::ErrorCode;

    #[test]
    fn spawn_failure_maps_to_unavailable() {
        let error = GitError::Spawn {
            command: "--version".into(),
            source: std::io::Error::other("No such file or directory"),
        };
        assert_eq!(error.to_app_error().code, ErrorCode::GitUnavailable);
    }

    #[test]
    fn not_repo_maps_to_its_own_code_and_predicate() {
        let error = GitError::NotRepo {
            path: "/tmp/x".into(),
        };
        assert_eq!(error.to_app_error().code, ErrorCode::GitNotARepo);
        assert!(error.is_not_repo());
    }

    #[test]
    fn lock_conflict_is_recoverable_busy_code() {
        let error = GitError::Locked {
            path: "/tmp/repo".into(),
        };
        assert_eq!(error.to_app_error().code, ErrorCode::GitBusy);
    }

    /// 前置条件拒绝要落到可识别的码上，并把中文原因带上去；
    /// 否则界面只剩一句笼统的「命令失败」，用户不知道先该做什么。
    #[test]
    fn precondition_maps_to_failed_with_its_reason() {
        let mapped = GitError::Precondition {
            reason: "暂存区有待提交内容".into(),
        }
        .to_app_error();
        assert_eq!(mapped.code, ErrorCode::GitCommandFailed);
        assert!(
            mapped
                .detail
                .as_deref()
                .is_some_and(|text| text.contains("暂存区有待提交内容")),
            "原因文本必须随错误上报，实际: {mapped:?}"
        );
    }
}
