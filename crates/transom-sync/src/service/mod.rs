//! 云同步编排：目录快照、远端检查、加密信封与 Git 同步入口。
//!
//! 设计说明（Why）：
//! - 上云内容一律加密：口令派生与信封格式复用本地备份的同一条实现
//!   （`encrypt_document`/`decrypt_document`），云端与本地备份文件可互换；
//! - 检查与上传都返回目录指纹，界面据此判断本机/远端是否偏离上次成功基线；
//! - 常规上传会把远端合并结果带回界面，显式“上传本地版本”则跳过合并，
//!   避免界面显示成功但本机实际上仍落后；
//! - Argon2 派生、钥匙串访问与 Git 子进程都是阻塞操作，统一放进阻塞线程池。

use std::sync::Arc;
use std::time::Instant;
use transom_core::{
    AppError, CloudPushMode, CloudSyncTarget, ErrorCode, GitSyncResult, MergePreference, SecretKey,
    SecretStore, SessionCatalog, SyncCheckReport, SyncDocumentSummary, SyncEndpoint,
    SyncPullReport, SyncPushReport, SyncSettings, SyncTestReport, is_sync_conflict, merge_catalogs,
    sync_catalog_hash,
};
use transom_crypto::backup::decrypt_document;
use zeroize::Zeroizing;

use super::git::{GitAuth, sync_git_blocking};
use helpers::{
    build_envelope, build_target, bytes_hash, elapsed_ms, passphrase_from, required_endpoint,
    resolve_relation, run_blocking,
};

mod helpers;

/// 云同步服务：持有全局唯一的 HTTP 客户端（连接池复用）与钥匙串访问。
pub struct CloudSyncService {
    http: reqwest::Client,
    secrets: Arc<dyn SecretStore>,
}

impl CloudSyncService {
    /// 构造服务；HTTP 客户端随服务生存，长生命周期复用连接池。
    ///
    /// # Errors
    /// TLS 后端初始化失败时返回 `AppError`。
    pub fn new(secrets: Arc<dyn SecretStore>) -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            // 16 MiB 信封在家庭宽带上行下的最坏耗时预留，含目录创建与认证挑战。
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .map_err(|error| {
                AppError::with_detail(
                    ErrorCode::CloudSyncFailed,
                    format!("初始化同步传输层失败: {error}"),
                )
            })?;
        Ok(Self { http, secrets })
    }

    /// 把目录快照加密上传到配置的端点。
    ///
    /// `mode` 为常规合并时，远端内容先并入本机快照；为强制本地时，只以当前
    /// 目录生成新对象。两种模式都返回最终目录，调用方必须回填到本地状态。
    ///
    /// # Errors
    /// 未配置端点、同步口令未设置、端点凭据缺失、加密失败或网络失败时
    /// 返回 `AppError`。
    pub async fn push(
        &self,
        catalog: &SessionCatalog,
        settings: &SyncSettings,
        mode: CloudPushMode,
    ) -> Result<SyncPushReport, AppError> {
        let endpoint = required_endpoint(settings)?;
        let target = self.build_target_async(&endpoint).await?;
        let passphrase = self.read_passphrase_async().await?;
        let include_secrets = settings.include_secrets;
        let secrets = self.secrets.clone();

        // 乐观锁：先读远端版本并合并，再带 If-Match 上传；并发设备抢先时
        // 重新拉取合并，最多三次。远端缺失按空目录处理，首次推送不受阻。
        let mut last_conflict = None;
        for attempt in 0..3 {
            let (remote_data, version) = match target.get_document_versioned().await {
                Ok(result) => result,
                Err(error) if error.code == ErrorCode::CloudSyncMissing => (Vec::new(), None),
                Err(error) => return Err(error),
            };
            let merged = if mode == CloudPushMode::ForceLocal || remote_data.is_empty() {
                catalog.clone()
            } else {
                let passphrase = passphrase.clone();
                let remote_catalog = tokio::task::spawn_blocking(move || {
                    decrypt_document(&remote_data, passphrase.as_str())
                })
                .await
                .map_err(|error| {
                    AppError::with_detail(
                        ErrorCode::CloudSyncFailed,
                        format!("解密远端备份任务失败: {error}"),
                    )
                })??
                .into_parts()
                .0;
                merge_catalogs(None, catalog, &remote_catalog, MergePreference::Local).0
            };
            let sessions = merged.sessions.len();
            let folders = merged.folders.len();
            let catalog_hash = sync_catalog_hash(&merged)?;
            let merged_for_build = merged.clone();
            let secrets_for_build = secrets.clone();
            let passphrase_for_build = passphrase.clone();
            let (data, secret_count) = tokio::task::spawn_blocking(move || {
                build_envelope(
                    &merged_for_build,
                    include_secrets,
                    secrets_for_build.as_ref(),
                    passphrase_for_build.as_str(),
                )
            })
            .await
            .map_err(|error| {
                AppError::with_detail(
                    ErrorCode::CloudSyncFailed,
                    format!("加密备份任务失败: {error}"),
                )
            })??;
            let remote_hash = bytes_hash(&data);

            match target
                .put_document_if_match(&data, version.as_deref())
                .await
            {
                Ok(()) => {
                    let report = SyncPushReport {
                        catalog: merged,
                        sessions,
                        folders,
                        secrets: secret_count,
                        bytes: data.len() as u64,
                        at: chrono::Utc::now().timestamp(),
                        catalog_hash,
                        remote_hash,
                    };
                    log::info!(
                        "云同步备份上传完成，尝试次数: {}，会话数: {}，分组数: {}，字节数: {}",
                        attempt + 1,
                        report.sessions,
                        report.folders,
                        report.bytes
                    );
                    return Ok(report);
                }
                Err(error) if is_sync_conflict(&error) => {
                    log::warn!(
                        "云同步版本冲突，将重新拉取合并后重试，第 {} 次",
                        attempt + 1
                    );
                    last_conflict = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_conflict.unwrap_or_else(|| {
            AppError::with_detail(
                ErrorCode::CloudSyncConflict,
                "云同步连续版本冲突，请稍后重试",
            )
        }))
    }

    /// 只读取远端并计算本机/远端相对变化，不修改本地目录。
    ///
    /// # Errors
    /// 端点、口令或凭据缺失，远端不可达，或密文无法解密时返回 `AppError`。
    pub async fn check(
        &self,
        catalog: &SessionCatalog,
        settings: &SyncSettings,
    ) -> Result<SyncCheckReport, AppError> {
        let started = Instant::now();
        let endpoint = required_endpoint(settings)?;
        let target = self.build_target_async(&endpoint).await?;
        let passphrase = self.read_passphrase_async().await?;
        let local_hash = sync_catalog_hash(catalog)?;
        let local_summary = SyncDocumentSummary::from_catalog(catalog);
        let remote = match target.get_document_versioned().await {
            Ok((data, version)) => Some((data, version)),
            Err(error) if error.code == ErrorCode::CloudSyncMissing => None,
            Err(error) => return Err(error),
        };
        let (remote_hash, remote_summary, remote_version) = match remote {
            Some((data, version)) => {
                let remote_hash = bytes_hash(&data);
                let passphrase = passphrase.clone();
                let document = tokio::task::spawn_blocking(move || {
                    decrypt_document(&data, passphrase.as_str())
                })
                .await
                .map_err(|error| {
                    AppError::with_detail(
                        ErrorCode::CloudSyncFailed,
                        format!("解密远端检查任务失败: {error}"),
                    )
                })??;
                (
                    Some(remote_hash),
                    Some(SyncDocumentSummary::from_document(&document)),
                    version,
                )
            }
            None => (None, None, None),
        };
        let (local_changed, remote_changed, relation) = resolve_relation(
            settings,
            &local_hash,
            &local_summary,
            remote_hash.as_deref(),
            remote_summary.as_ref(),
        );
        Ok(SyncCheckReport {
            provider: endpoint.provider_key().to_string(),
            at: chrono::Utc::now().timestamp(),
            duration_ms: elapsed_ms(started),
            local_hash,
            remote_hash,
            remote_present: remote_summary.is_some(),
            local_changed,
            remote_changed,
            relation,
            local_summary,
            remote_summary,
            remote_version,
        })
    }

    /// 只探测端点、凭据与固定对象，不下载密文正文。
    ///
    /// # Errors
    /// 网络失败、认证失败或服务端拒绝请求时返回 `AppError`。
    pub async fn test_connection(
        &self,
        settings: &SyncSettings,
    ) -> Result<SyncTestReport, AppError> {
        let started = Instant::now();
        let endpoint = required_endpoint(settings)?;
        let target = self.build_target_async(&endpoint).await?;
        let version = target.probe_document().await?;
        Ok(SyncTestReport {
            provider: endpoint.provider_key().to_string(),
            at: chrono::Utc::now().timestamp(),
            duration_ms: elapsed_ms(started),
            remote_present: version.is_some(),
            remote_version: version,
        })
    }

    /// 从端点下载并解密备份文档；合并与口令写回由调用方编排。
    ///
    /// # Errors
    /// 未配置端点、同步口令未设置、远端无备份、解密失败或网络失败时
    /// 返回 `AppError`。
    pub async fn pull(&self, settings: &SyncSettings) -> Result<SyncPullReport, AppError> {
        let endpoint = required_endpoint(settings)?;
        let target = self.build_target_async(&endpoint).await?;
        let passphrase = self.read_passphrase_async().await?;

        let (data, remote_version) = target.get_document_versioned().await?;
        let remote_hash = bytes_hash(&data);
        let bytes = data.len() as u64;
        let document = tokio::task::spawn_blocking(move || decrypt_document(&data, &passphrase))
            .await
            .map_err(|error| {
                AppError::with_detail(
                    ErrorCode::CloudSyncFailed,
                    format!("解密备份任务失败: {error}"),
                )
            })??;
        Ok(SyncPullReport {
            document,
            remote_version,
            remote_hash,
            bytes,
            at: chrono::Utc::now().timestamp(),
        })
    }

    /// 按端点配置读取凭据并构造远端目标。
    ///
    /// 钥匙串访问可能走 D-Bus 并阻塞数秒，必须放进阻塞线程池：控制面运行时
    /// 只有两个工作线程，一次同步读取就可能拖住全部连接与停机。
    async fn build_target_async(
        &self,
        endpoint: &SyncEndpoint,
    ) -> Result<Box<dyn CloudSyncTarget>, AppError> {
        let http = self.http.clone();
        let secrets = self.secrets.clone();
        let endpoint = endpoint.clone();
        run_blocking("读取同步端点凭据", move || {
            build_target(&http, secrets.as_ref(), &endpoint)
        })
        .await
    }

    async fn read_passphrase_async(&self) -> Result<Zeroizing<String>, AppError> {
        let secrets = self.secrets.clone();
        run_blocking("读取同步口令", move || {
            passphrase_from(secrets.as_ref())
        })
        .await
    }

    /// 执行一次私有 Git 仓库同步（阻塞，调用方必须放进 `spawn_blocking`）。
    ///
    /// 同步口令与 PAT 复用云同步的钥匙串槽位：用户只维护一份秘密，
    /// Git 与 WebDAV/S3 的凭据不会因为两条通道而分裂。
    ///
    /// # Errors
    /// 未配置 Git、缺少同步口令或专用令牌、外部 `git` 失败时返回 `AppError`。
    pub fn run_git_sync(
        &self,
        settings: &SyncSettings,
        catalog: &SessionCatalog,
        cache_dir: &std::path::Path,
    ) -> Result<GitSyncResult, AppError> {
        let git = settings.git.as_ref().ok_or_else(|| {
            AppError::with_detail(ErrorCode::Configuration, "尚未配置 Git 同步仓库")
        })?;
        let passphrase = self.read_passphrase()?;
        // Git 只接受专用令牌：云同步凭据在 WebDAV 下是网盘应用口令、S3 下是
        // Secret Key，复用它会把第三方服务的凭据当成 Basic Auth 发给 Git 主机。
        let token = self
            .secrets
            .get(&SecretKey::git_sync_token())?
            .ok_or_else(|| {
                AppError::with_detail(
                    ErrorCode::Configuration,
                    "Git 同步需要单独填写访问令牌；云同步凭据不会发送给 Git 主机",
                )
            })?;
        let auth = GitAuth {
            token: Some(token),
            username: None,
        };
        sync_git_blocking(git, catalog, passphrase.as_str(), cache_dir, &auth)
    }

    fn read_passphrase(&self) -> Result<Zeroizing<String>, AppError> {
        passphrase_from(self.secrets.as_ref())
    }
}

#[cfg(test)]
mod tests;
