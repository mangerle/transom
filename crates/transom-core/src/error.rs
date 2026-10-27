use serde::{Deserialize, Serialize};
use std::fmt;

/// 跨层稳定的错误分类枚举。UI 层根据此错误码展示本地化文案，`detail` 仅供诊断排查。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    /// 配置参数非法或格式错误。
    Configuration,
    /// 网络拨号或底层连接建立失败。
    ConnectionFailed,
    /// 域名解析失败。
    Dns,
    /// 操作或连接超时。
    Timeout,
    /// SSH 或服务凭证认证失败。
    AuthFailed,
    /// 键盘交互式认证失败或被拒绝。
    KeyboardInteractiveFailed,
    /// 首次连接未知的远端主机密钥。
    HostKeyUnknown,
    /// 主机公钥指纹与 known_hosts 记录不匹配（疑似中间人拦截）。
    HostKeyMismatch,
    /// 打开 SSH 通道或子系统失败。
    ChannelFailed,
    /// SFTP 文件协议操作失败。
    SftpFailed,
    /// 编辑回传前发现远端文件已被修改（版本指纹不一致），拒绝覆盖。
    SftpTextConflict,
    /// 本地、远端或动态端口转发失败。
    ForwardFailed,
    /// 本地文件或系统底层输入输出故障。
    Io,
    /// 系统安全凭据库（钥匙串/Secret Service）不可用。
    SecretStoreUnavailable,
    /// 备份文件是加密信封，但调用方按明文文件处理。
    BackupEncrypted,
    /// 备份文件不是加密信封，但调用方提供了解密口令。
    BackupNotEncrypted,
    /// 操作被用户或上层任务主动取消。
    Cancelled,
    /// 违反线格式协议契约或数据包解析异常。
    Protocol,
    /// 云端同步失败：端点不可达、认证被拒、远端拒绝写入或内容超限。
    CloudSyncFailed,
    /// 云端还没有任何备份：首次拉取属预期状态，界面按提示而非故障处理。
    CloudSyncMissing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub detail: Option<String>,
}

impl AppError {
    pub fn new(code: ErrorCode) -> Self {
        Self { code, detail: None }
    }

    pub fn with_detail(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: Some(detail.into()),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{:?}: {detail}", self.code),
            None => write!(f, "{:?}", self.code),
        }
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        Self::with_detail(ErrorCode::Io, value.to_string())
    }
}
