//! 云同步编排：目录快照 -> Argon2id/XChaCha20 加密信封 -> 远端对象。
//!
//! 设计说明（Why）：
//! - 上云内容一律加密：口令派生与信封格式复用本地备份的同一条实现
//!   （`encrypt_document`/`decrypt_document`），云端与本地备份文件可互换，
//!   也杜绝了"明文备份意外留在网盘"这类单点失误；
//! - 端点凭据与同步口令在编排前从钥匙串读出，内存中经 `Zeroizing` 保活，
//!   不落盘、不进日志；
//! - Argon2 派生（64 MiB 内存）与口令收集（钥匙串同步调用）都是阻塞操作，
//!   统一放进 `spawn_blocking`，不占用异步工作线程。

use std::sync::Arc;
use transom_core::{
    AppError, BackupDocument, CloudSyncTarget, ErrorCode, GitSyncResult, SecretKey, SecretStore,
    SessionCatalog, SyncEndpoint, SyncPushReport, SyncSettings, collect_catalog_secrets,
    is_sync_conflict, merge_catalogs,
};
use transom_crypto::backup::{BackupFormat, decrypt_document, encrypt_document};
use zeroize::Zeroizing;

use super::git::{GitAuth, sync_git_blocking};
use super::s3::S3Target;
use super::webdav::WebDavTarget;

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
            // 16 MiB 信封在家庭宽带上行下的最坏耗时预留，含 MKCOL 重试。
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
    /// # Errors
    /// 未配置端点、同步口令未设置、端点凭据缺失、加密失败或网络失败时
    /// 返回 `AppError`。
    pub async fn push(
        &self,
        catalog: &SessionCatalog,
        settings: &SyncSettings,
    ) -> Result<SyncPushReport, AppError> {
        let endpoint = required_endpoint(settings)?;
        let target = self.build_target(endpoint)?;
        let passphrase = self.read_passphrase()?;
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
            let remote_catalog = if remote_data.is_empty() {
                SessionCatalog::default()
            } else {
                let passphrase = passphrase.clone();
                tokio::task::spawn_blocking(move || {
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
                .0
            };
            let (merged, _) = merge_catalogs(None, catalog, &remote_catalog);
            let sessions = merged.sessions.len();
            let folders = merged.folders.len();
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

            match target
                .put_document_if_match(&data, version.as_deref())
                .await
            {
                Ok(()) => {
                    let report = SyncPushReport {
                        sessions,
                        folders,
                        secrets: secret_count,
                        bytes: data.len() as u64,
                        at: chrono::Utc::now().timestamp(),
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
            AppError::with_detail(ErrorCode::CloudSyncFailed, "云同步连续版本冲突，请稍后重试")
        }))
    }

    /// 从端点下载并解密备份文档；合并与口令写回由调用方编排。
    ///
    /// # Errors
    /// 未配置端点、同步口令未设置、远端无备份、解密失败或网络失败时
    /// 返回 `AppError`。
    pub async fn pull(&self, settings: &SyncSettings) -> Result<BackupDocument, AppError> {
        let endpoint = required_endpoint(settings)?;
        let target = self.build_target(endpoint)?;
        let passphrase = self.read_passphrase()?;

        let data = target.get_document().await?;
        tokio::task::spawn_blocking(move || decrypt_document(&data, &passphrase))
            .await
            .map_err(|error| {
                AppError::with_detail(
                    ErrorCode::CloudSyncFailed,
                    format!("解密备份任务失败: {error}"),
                )
            })?
    }

    /// 按端点配置构造远端目标；端点凭据从钥匙串读取。
    fn build_target(&self, endpoint: &SyncEndpoint) -> Result<Box<dyn CloudSyncTarget>, AppError> {
        match endpoint {
            SyncEndpoint::Webdav {
                url,
                username,
                directory,
            } => {
                let password = self.read_credential()?.map(|password| password.to_string());
                Ok(Box::new(WebDavTarget::new(
                    self.http.clone(),
                    url,
                    username,
                    password.as_deref(),
                    directory,
                )?))
            }
            SyncEndpoint::S3 {
                endpoint,
                region,
                bucket,
                access_key,
                directory,
            } => {
                let secret_key = self.read_credential()?.ok_or_else(|| {
                    AppError::with_detail(
                        ErrorCode::Configuration,
                        "尚未在钥匙串保存 S3 Secret Key，请先在设置中填写并保存",
                    )
                })?;
                Ok(Box::new(S3Target::new(
                    self.http.clone(),
                    endpoint,
                    region,
                    bucket,
                    access_key,
                    secret_key,
                    directory,
                )?))
            }
        }
    }

    /// 执行一次私有 Git 仓库同步（阻塞，调用方必须放进 spawn_blocking）。
    ///
    /// 同步口令与 PAT 复用云同步的钥匙串槽位：用户只维护一份秘密，
    /// Git 与 WebDAV/S3 的凭据不会因为两条通道而分裂。
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
        // Git 专用槽位优先；未单独保存 PAT 时回退云同步凭据槽位，
        // 兼容"同一条令牌同时服务 S3 与 Git"的简单用法。
        let token = self
            .secrets
            .get(&SecretKey::git_sync_token())?
            .or(self.read_credential()?);
        let auth = GitAuth {
            token,
            username: None,
        };
        let outcome = sync_git_blocking(git, catalog, passphrase.as_str(), cache_dir, &auth)?;
        Ok(GitSyncResult {
            catalog: outcome.catalog,
            report: outcome.report,
            pushed: outcome.pushed,
            bytes: outcome.bytes,
            at: outcome.at,
        })
    }

    fn read_credential(&self) -> Result<Option<Zeroizing<String>>, AppError> {
        self.secrets.get(&SecretKey::cloud_sync_credential())
    }

    fn read_passphrase(&self) -> Result<Zeroizing<String>, AppError> {
        self.secrets
            .get(&SecretKey::cloud_sync_passphrase())?
            .ok_or_else(|| {
                AppError::with_detail(
                    ErrorCode::Configuration,
                    "尚未设置同步口令，云端备份必须加密；请先在设置中填写并保存",
                )
            })
    }
}

/// 取出已启用的端点配置。
fn required_endpoint(settings: &SyncSettings) -> Result<&SyncEndpoint, AppError> {
    settings
        .endpoint
        .as_ref()
        .ok_or_else(|| AppError::with_detail(ErrorCode::Configuration, "尚未配置云同步端点"))
}

/// 构建加密信封：收集口令（可选）-> 快照文档 -> Argon2id 派生加密。
///
/// 返回信封字节与加密口令数；阻塞操作，调用方必须放在阻塞线程池执行。
fn build_envelope(
    catalog: &SessionCatalog,
    include_secrets: bool,
    secrets: &dyn SecretStore,
    passphrase: &str,
) -> Result<(Vec<u8>, usize), AppError> {
    let mut document = BackupDocument::from_catalog(catalog);
    let mut secret_count = 0;
    if include_secrets {
        let collected = collect_catalog_secrets(secrets, catalog)?;
        secret_count = collected.len();
        document.secrets = Some(collected);
    }
    let data = encrypt_document(&document, passphrase, BackupFormat::Json)?;
    Ok((data, secret_count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use transom_core::{AuthMethod, CloudSyncTarget, Connection, Session, SshConnection};

    /// 内存钥匙串：覆盖"已设置/未设置"两种状态，不依赖系统 Secret Service。
    #[derive(Default)]
    struct MemorySecrets {
        entries: Mutex<HashMap<String, String>>,
    }

    impl MemorySecrets {
        fn put(&self, key: &SecretKey, value: &str) {
            self.entries
                .lock()
                .unwrap()
                .insert(key.account(), value.to_string());
        }
    }

    impl SecretStore for MemorySecrets {
        fn get(&self, key: &SecretKey) -> Result<Option<Zeroizing<String>>, AppError> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .get(&key.account())
                .map(|value| Zeroizing::new(value.clone())))
        }

        fn set(&self, key: &SecretKey, secret: &str) -> Result<(), AppError> {
            self.entries
                .lock()
                .unwrap()
                .insert(key.account(), secret.to_string());
            Ok(())
        }

        fn delete(&self, key: &SecretKey) -> Result<(), AppError> {
            self.entries.lock().unwrap().remove(&key.account());
            Ok(())
        }
    }

    /// 内存远端：记录上传内容，可配置为"空端点"模拟首次拉取。
    struct MemoryTarget {
        stored: Mutex<Option<Vec<u8>>>,
    }

    impl MemoryTarget {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                stored: Mutex::new(None),
            })
        }
    }

    #[async_trait::async_trait]
    impl CloudSyncTarget for MemoryTarget {
        async fn put_document(&self, data: &[u8]) -> Result<(), AppError> {
            *self.stored.lock().unwrap() = Some(data.to_vec());
            Ok(())
        }

        async fn get_document(&self) -> Result<Vec<u8>, AppError> {
            self.stored.lock().unwrap().clone().ok_or_else(|| {
                AppError::with_detail(ErrorCode::CloudSyncMissing, "远端还没有同步备份")
            })
        }
    }

    fn catalog() -> SessionCatalog {
        let mut catalog = SessionCatalog::default();
        catalog.sessions.push(Session::new(
            "web-01",
            Connection::Ssh(SshConnection {
                host: "web.example.com".into(),
                port: 22,
                username: "deploy".into(),
                auth: AuthMethod::Password { save: true },
                options: Default::default(),
                proxy_jump: None,
            }),
        ));
        catalog
    }

    fn settings(endpoint: Option<SyncEndpoint>) -> SyncSettings {
        SyncSettings {
            endpoint,
            include_secrets: true,
            ..Default::default()
        }
    }

    fn webdav_endpoint() -> SyncEndpoint {
        SyncEndpoint::Webdav {
            url: "https://dav.example.com/dav".into(),
            username: "user".into(),
            directory: "transom".into(),
        }
    }

    #[tokio::test]
    async fn push_requires_endpoint_and_passphrase() {
        let service = CloudSyncService::new(Arc::new(MemorySecrets::default())).unwrap();

        let error = service.push(&catalog(), &settings(None)).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::Configuration);
        assert!(error.detail.unwrap_or_default().contains("端点"));

        let error = service
            .push(&catalog(), &settings(Some(webdav_endpoint())))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Configuration);
        assert!(error.detail.unwrap_or_default().contains("同步口令"));
    }

    /// 加密信封往返：build_envelope 的产物必须能被 decrypt_document 还原，
    /// 且错误口令被拒。云端与本地备份文件格式一致是产品契约。
    #[tokio::test]
    async fn envelope_round_trips_and_rejects_wrong_passphrase() {
        let secrets = MemorySecrets::default();
        secrets.put(&SecretKey::cloud_sync_credential(), "app-password");

        let (data, secret_count) =
            build_envelope(&catalog(), true, &secrets, "correct horse").unwrap();
        assert_eq!(secret_count, 0, "测试目录未保存任何会话口令");
        assert!(!String::from_utf8_lossy(&data).contains("web.example.com"));

        let remote = MemoryTarget::new();
        remote.put_document(&data).await.unwrap();
        let document =
            decrypt_document(&remote.get_document().await.unwrap(), "correct horse").unwrap();
        assert_eq!(document.sessions.len(), 1);
        assert_eq!(document.sessions[0].name, "web-01");

        let downloaded = remote.get_document().await.unwrap();
        assert!(decrypt_document(&downloaded, "wrong").is_err());
    }

    #[tokio::test]
    async fn empty_remote_reports_missing() {
        let remote = MemoryTarget::new();
        let error = remote.get_document().await.unwrap_err();
        assert_eq!(error.code, ErrorCode::CloudSyncMissing);
    }
}
