//! Transom 跨平台自更新封装。
//!
//! # 设计原理（Why）
//! - 应用层只依赖本 crate 的稳定事件与信息模型，不直接依赖 `shipup` 的
//!   版本迭代细节；上游升级时改动面收敛在这里。
//! - 检查、下载与安装全部提供阻塞入口，由 GPUI 侧的统一阻塞线程调度器调用；
//!   自更新是低频重操作，不为它额外引入异步运行时。
//! - 清单必须通过内置 Ed25519 公钥验签，下载包再做 SHA-256 校验；未签名的
//!   伪造清单在进入安装流程前就会被拒绝。
//!
//! # 代价与局限
//! 公钥随二进制编译，轮换签名密钥必须发一个新版本；发布端私钥只保存在
//! GitHub Secret 与维护者本机，不进入仓库。

use shipup::{HealthCheckStatus, Update, UpdateEvent as ShipupEvent, Updater};
use std::convert::Infallible;
use std::time::Duration;
use thiserror::Error;

/// 自更新领域错误。
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum UpdaterError {
    /// 更新器构建、检查、下载、安装或崩溃回滚失败。
    #[error("自更新操作失败，原因: {reason}")]
    Operation {
        /// 可定位的中文错误描述。
        reason: String,
    },
}

/// 构造带操作上下文的更新错误。
fn updater_error(context: &str, error: impl std::fmt::Display) -> UpdaterError {
    UpdaterError::Operation {
        reason: format!("{context}: {error}"),
    }
}

/// Transom 官方发布包 Ed25519 签名验证公钥（Base64 编码）。
///
/// 与 snapin 使用独立密钥：两项目不共用签名身份，任一项目私钥泄漏都不会
/// 让另一项目的更新通道被伪造。
pub const UPDATE_PUBLIC_KEY: &str = "kp7zz1TbrsgNbx4E7HezQqdkWUrEmM8sH5X1EFAsbHI=";

/// Transom 官方 GitHub Releases 最新版本清单端点。
pub const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/mangerle/transom/releases/latest/download/latest.json";

/// 根据配置与编译期版本构建 `shipup::Updater` 实例。
///
/// `file://` 仅用于本地清单核验与测试；正式客户端只接受 HTTPS 端点。
///
/// # Errors
/// 若版本号格式非法或更新器参数校验失败，返回 [`UpdaterError::Operation`]。
fn build_updater(custom_manifest_url: Option<&str>) -> Result<Updater, UpdaterError> {
    let manifest_url = custom_manifest_url
        .filter(|url| !url.trim().is_empty())
        .unwrap_or(DEFAULT_MANIFEST_URL);

    let allow_file = manifest_url.starts_with("file://");

    Updater::builder()
        .current_version(env!("CARGO_PKG_VERSION"))
        .map_err(|error| updater_error("解析当前客户端版本失败", error))?
        .manifest_url(manifest_url)
        .public_key(UPDATE_PUBLIC_KEY)
        .allow_file_protocol(allow_file)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| updater_error("构建自更新引擎失败", error))
}

/// 同步检查是否有可用新版本（供 GPUI 阻塞线程池调用）。
///
/// # Errors
/// 当网络超时、Manifest 解析异常或签名校验失败时返回 [`UpdaterError`]。
pub fn check_update(custom_manifest_url: Option<&str>) -> Result<Option<UpdateInfo>, UpdaterError> {
    let updater = build_updater(custom_manifest_url)?;
    updater
        .check()
        .map(|update| update.map(UpdateInfo::new))
        .map_err(|error| updater_error("检查更新失败", error))
}

/// 已确认可用的新版本信息。
///
/// 对外只暴露版本、发布日期与更新日志，不把 `shipup::Update` 泄漏到应用层；
/// 下载与安装也必须经过本 crate 的统一入口。
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    inner: Update,
}

impl UpdateInfo {
    fn new(inner: Update) -> Self {
        Self { inner }
    }

    /// 新版本号。
    pub fn version(&self) -> String {
        self.inner.version().to_string()
    }

    /// 更新日志。
    pub fn notes(&self) -> Option<String> {
        self.inner.notes().map(str::to_string)
    }

    /// 发布日期。
    pub fn pub_date(&self) -> Option<String> {
        self.inner.pub_date().map(str::to_string)
    }

    /// 同步下载并验签；回调只接收本 crate 的稳定事件类型。
    ///
    /// # Errors
    /// 网络失败、哈希或签名校验失败时返回 [`UpdaterError`]。
    pub fn download_blocking<F>(&self, mut callback: F) -> Result<DownloadedUpdate, UpdaterError>
    where
        F: FnMut(UpdateEvent),
    {
        self.inner
            .download(|event| callback(map_event(event)))
            .map(DownloadedUpdate::new)
            .map_err(|error| updater_error("下载或验签更新包失败", error))
    }
}

/// 已下载并验签、等待安装的更新包。
#[derive(Debug, Clone)]
pub struct DownloadedUpdate {
    inner: shipup::DownloadedUpdate,
}

impl DownloadedUpdate {
    fn new(inner: shipup::DownloadedUpdate) -> Self {
        Self { inner }
    }

    /// 目标版本号。
    pub fn version(&self) -> String {
        self.inner.version().to_string()
    }

    /// 执行物理安装；回调接收统一的安装阶段事件。
    ///
    /// # Errors
    /// 归档解压、二进制替换或安装器派生失败时返回 [`UpdaterError`]。
    pub fn install<F>(&self, mut callback: F) -> Result<(), UpdaterError>
    where
        F: FnMut(UpdateEvent),
    {
        self.inner
            .install(|event| callback(map_event(event)))
            .map_err(|error| updater_error("应用更新包失败", error))
    }
}

/// 更新下载/安装阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdateStage {
    /// 正在读取远端清单。
    CheckingManifest,
    /// 下载已开始。
    DownloadStarted,
    /// 正在校验 SHA-256。
    VerifyingChecksum,
    /// 正在校验 Ed25519 签名。
    VerifyingSignature,
    /// 正在解压归档。
    ExtractingArchive,
    /// 正在写入安装结果。
    Installing,
    /// 已就绪，等待重启。
    ReadyToRestart,
    /// 已推迟到系统重启后生效。
    DeferredToReboot,
    /// 安装完成。
    Completed,
    /// 上游新增但本版本尚未建模的阶段。
    Other,
}

/// 自更新进度事件。
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum UpdateEvent {
    /// 下载进度。
    DownloadProgress {
        /// 已下载字节数。
        downloaded_bytes: u64,
        /// 总字节数；服务端未提供时为空。
        total_bytes: Option<u64>,
        /// 完成百分比。
        percent: Option<f32>,
        /// 当前下载速率。
        speed_bytes_per_sec: Option<u64>,
    },
    /// 阶段切换。
    Stage(UpdateStage),
    /// 失败重试中。
    Retrying {
        /// 当前重试次数。
        attempt: u32,
        /// 最大重试次数。
        max_retries: u32,
        /// 最近一次失败原因。
        error: String,
    },
    /// 不可恢复失败。
    Failed {
        /// 失败原因。
        reason: String,
    },
}

/// 把 shipup 事件映射为本 crate 的稳定事件。
fn map_event(event: ShipupEvent) -> UpdateEvent {
    match event {
        ShipupEvent::CheckingManifest => UpdateEvent::Stage(UpdateStage::CheckingManifest),
        ShipupEvent::DownloadStarted { .. } => UpdateEvent::Stage(UpdateStage::DownloadStarted),
        ShipupEvent::DownloadProgress {
            downloaded_bytes,
            total_bytes,
            percent,
            speed_bytes_per_sec,
            ..
        } => UpdateEvent::DownloadProgress {
            downloaded_bytes,
            total_bytes,
            percent,
            speed_bytes_per_sec,
        },
        ShipupEvent::VerifyingChecksum => UpdateEvent::Stage(UpdateStage::VerifyingChecksum),
        ShipupEvent::VerifyingSignature => UpdateEvent::Stage(UpdateStage::VerifyingSignature),
        ShipupEvent::ExtractingArchive => UpdateEvent::Stage(UpdateStage::ExtractingArchive),
        ShipupEvent::Installing => UpdateEvent::Stage(UpdateStage::Installing),
        ShipupEvent::ReadyToRestart => UpdateEvent::Stage(UpdateStage::ReadyToRestart),
        ShipupEvent::DeferredToReboot { .. } => UpdateEvent::Stage(UpdateStage::DeferredToReboot),
        ShipupEvent::Retrying {
            attempt,
            max_retries,
            error,
            ..
        } => UpdateEvent::Retrying {
            attempt,
            max_retries,
            error,
        },
        ShipupEvent::Failed { reason } => UpdateEvent::Failed { reason },
        ShipupEvent::Completed => UpdateEvent::Stage(UpdateStage::Completed),
    }
}

/// 启动健康自检的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecoveryStatus {
    /// 没有待确认的更新，正常启动。
    Normal,
    /// 新版本处于健康观察期。
    PendingConfirmation {
        /// 当前启动尝试次数。
        attempts: u32,
    },
    /// 连续启动失败达到阈值，已自动回滚。
    RolledBack {
        /// 回滚前的故障版本号。
        from_version: String,
    },
}

/// 客户端启动入口的崩溃自检与自动回滚。
///
/// # 机制说明
/// 在应用 `main` 的早期执行。若新版本由于打包动态库缺失或严重缺陷导致连续
/// 启动崩溃达到 2 次，会自动用上一稳定版本的备份覆盖回滚，防止客户端陷入
/// 不可恢复的闪退死锁。
///
/// # Errors
/// 回滚覆盖可执行文件失败时返回 [`UpdaterError`]。
pub fn check_and_recover_on_startup() -> Result<RecoveryStatus, UpdaterError> {
    let status = shipup::check_and_recover_current(2)
        .map_err(|error| updater_error("启动自检与崩溃回滚执行失败", error))?;
    Ok(match status {
        HealthCheckStatus::Normal => RecoveryStatus::Normal,
        HealthCheckStatus::PendingConfirmation { attempts } => {
            RecoveryStatus::PendingConfirmation { attempts }
        }
        HealthCheckStatus::RolledBack { from_version } => {
            RecoveryStatus::RolledBack { from_version }
        }
    })
}

/// 客户端主流程平稳就绪后确认升级成功闭环。
///
/// # 机制说明
/// 在主窗口成功创建并运行一段时间后调用，重置崩溃计数器并安全清理旧版本备份。
///
/// # Errors
/// 状态文件清理失败时返回 [`UpdaterError`]。
pub fn confirm_update_success() -> Result<(), UpdaterError> {
    shipup::confirm_update_success()
        .map(|_| ())
        .map_err(|error| updater_error("确认升级状态闭环失败", error))
}

/// 拉起当前可执行文件的新进程并退出本进程。
///
/// # 机制说明
/// 调用方必须先完成自身优雅停机（断开远端连接、关闭日志等），再调用本函数；
/// 函数成功时不会返回。
///
/// # Errors
/// 拉起新进程失败时返回 [`UpdaterError`]。
pub fn restart_current_process() -> Result<Infallible, UpdaterError> {
    shipup::restart_with(|_| {}).map_err(|error| updater_error("重启应用失败", error))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 外部事件必须映射为稳定的本 crate 事件，供关于页与后续通知共用。
    #[test]
    fn shipup_event_mapping_is_stable() {
        let progress = map_event(ShipupEvent::DownloadProgress {
            downloaded_bytes: 10,
            total_bytes: Some(100),
            percent: Some(10.0),
            speed_bytes_per_sec: Some(1024),
            eta: None,
        });
        assert_eq!(
            progress,
            UpdateEvent::DownloadProgress {
                downloaded_bytes: 10,
                total_bytes: Some(100),
                percent: Some(10.0),
                speed_bytes_per_sec: Some(1024),
            }
        );

        let retry = map_event(ShipupEvent::Retrying {
            attempt: 1,
            max_retries: 3,
            delay: Duration::from_millis(100),
            error: "网络抖动".to_string(),
        });
        assert!(matches!(retry, UpdateEvent::Retrying { attempt: 1, .. }));
        assert!(matches!(
            map_event(ShipupEvent::VerifyingSignature),
            UpdateEvent::Stage(UpdateStage::VerifyingSignature)
        ));
    }

    /// 默认配置下更新器必须能够成功构建。
    #[test]
    fn default_updater_builds() {
        assert!(build_updater(None).is_ok(), "默认更新器应当构建成功");
    }

    /// 自定义 HTTPS 端点必须能够成功构建。
    #[test]
    fn custom_updater_builds() {
        let custom = "https://example.com/mirror/latest.json";
        assert!(
            build_updater(Some(custom)).is_ok(),
            "自定义端点应当构建成功"
        );
    }

    /// 空字符串端点必须安全回退到官方默认地址。
    #[test]
    fn empty_updater_falls_back() {
        assert!(
            build_updater(Some("   ")).is_ok(),
            "空字符串应当回退到默认端点"
        );
    }

    /// 缺少合法 Ed25519 签名的伪造清单必须被拒绝。
    #[test]
    fn unsigned_manifest_is_rejected() {
        let temp_dir = tempfile::tempdir().expect("创建临时目录失败");
        let manifest_path = temp_dir.path().join("latest.json");
        std::fs::write(&manifest_path, r#"{"version":"9.9.9","packages":{}}"#)
            .expect("写入临时清单失败");

        let file_url = format!("file://{}", manifest_path.to_string_lossy());
        assert!(
            check_update(Some(&file_url)).is_err(),
            "缺少合法签名的清单必须被更新器拦截"
        );
    }
}
