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
    /// 远端拒绝授权本次操作（`SSH_FX_PERMISSION_DENIED`）。
    ///
    /// 单独成码而不是混在 [`Self::SftpFailed`] 里靠文案判断：权限拒绝有一条独立的
    /// 恢复路径（提示提权保存），而"文案里恰好出现 Permission denied"依赖远端
    /// locale 与实现，中文系统的 SFTP 服务器根本不会回这句话。
    PermissionDenied,
    /// SFTP 传输后完整性校验失败（大小不一致或哈希指纹不匹配）。
    SftpVerifyFailed,
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
    /// 备份内容超过体积上限。写侧与读侧共用同一码，避免导出成功却无法读回。
    BackupTooLarge,
    /// 操作被用户或上层任务主动取消。
    Cancelled,
    /// 违反线格式协议契约或数据包解析异常。
    Protocol,
    /// 云端同步失败：端点不可达、认证被拒、远端拒绝写入或内容超限。
    CloudSyncFailed,
    /// 云端备份已被其他设备修改，本次写入被版本条件拒绝。
    ///
    /// 单独成码而不是混在 [`Self::CloudSyncFailed`] 里靠文案区分：冲突要走
    /// "重新拉取、合并、重试"的恢复路径，其他同步失败则直接终止，两者是
    /// 不同的控制流，必须由类型系统而不是字符串内容来判别。
    CloudSyncConflict,
    /// 云端还没有任何备份：首次拉取属预期状态，界面按提示而非故障处理。
    CloudSyncMissing,
}

/// 跨层保留的失败现场。
///
/// 设计原理（Why）：`detail` 是给人看的裸字符串，调用方无法在不解析文案的
/// 前提下区分“认证失败”和“握手失败”。把连接链路的关键字段结构化后，上层
/// 可以按类型选择恢复路径（重试、换认证方式、提示检查 known_hosts），而不必
/// 依赖远端 locale 或错误文案的关键字。
///
/// 代价与局限：这里只保留稳定、可序列化的现场字段，不携带底层 `source`
/// 错误对象；排查完整因果仍以 `detail` 与日志为准。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorContext {
    /// 拨号失败。
    Connect { host: String, port: u16 },
    /// SSH 握手或版本协商失败。
    Handshake {
        host: String,
        port: u16,
        reason: String,
    },
    /// 主机密钥校验未通过。
    HostKey {
        host: String,
        port: u16,
        reason: String,
    },
    /// 认证失败；`reason` 标明失败环节（password / publickey / agent / keyboard）。
    Auth { reason: String },
    /// 出站代理隧道失败。
    Proxy {
        proxy: String,
        target: String,
        reason: String,
    },
    /// 通道或子系统打开失败。
    Channel { reason: String },
    /// 操作被取消。
    Cancelled { reason: String },
    /// 端口拨测超时。
    ProbeTimeout {
        host: String,
        port: u16,
        timeout_ms: u64,
    },
    /// Wake-on-LAN 发包失败。
    Wake { mac: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub detail: Option<String>,
    /// 结构化现场；旧版本错误文件/事件里没有这个字段，反序列化按 `None` 兜底。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ErrorContext>,
}

impl AppError {
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            detail: None,
            context: None,
        }
    }

    pub fn with_detail(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: Some(detail.into()),
            context: None,
        }
    }

    /// 同时携带可读文案与结构化现场。
    pub fn with_context(code: ErrorCode, detail: impl Into<String>, context: ErrorContext) -> Self {
        Self {
            code,
            detail: Some(detail.into()),
            context: Some(context),
        }
    }

    /// 失败现场；调用方按变体选择恢复路径。
    pub fn context(&self) -> Option<&ErrorContext> {
        self.context.as_ref()
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

#[cfg(test)]
mod tests {
    use super::{AppError, ErrorCode, ErrorContext};

    /// 旧错误文件没有 context 字段，必须按 None 反序列化而不是报错。
    #[test]
    fn legacy_error_json_without_context_still_parses() {
        let error: AppError = serde_json::from_str(r#"{"code":"auth_failed","detail":"口令错误"}"#)
            .expect("旧格式错误应可反序列化");
        assert_eq!(error.code, ErrorCode::AuthFailed);
        assert_eq!(error.context(), None);
    }

    /// 结构化现场随错误一起序列化与还原，上层可据变体选择恢复路径。
    #[test]
    fn error_context_roundtrips_through_json() {
        let error = AppError::with_context(
            ErrorCode::AuthFailed,
            "SSH 认证失败: publickey",
            ErrorContext::Auth {
                reason: "publickey".to_string(),
            },
        );
        let encoded = serde_json::to_string(&error).expect("错误应可序列化");
        let decoded: AppError = serde_json::from_str(&encoded).expect("错误应可反序列化");
        assert_eq!(decoded, error);
        assert!(matches!(
            decoded.context(),
            Some(ErrorContext::Auth { reason }) if reason == "publickey"
        ));
    }
}
