//! AI 配置的唯一事实源、串行保存与运行快照。
//!
//! 设计原理（Why）：
//! - UI 编辑副本、磁盘文件和运行期全局状态曾经是三份来源，保存尚未完成时启动运行会读到旧配置；
//! - 控制平面持有唯一内存快照，保存命令先串行落盘、成功后替换快照，再向 UI 发送确认；
//! - 加载失败时显式保留告警并使用默认配置，运行期不再反复读取磁盘或猜测配置状态。

use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::Mutex;
use transom_ai::{AiError, AiSettings};
use transom_store::AiStore;

/// 配置服务对外提供的只读快照。
pub(in crate::controller) struct AiConfigSnapshot {
    /// 与当前版本对应的序列化配置。
    pub(in crate::controller) settings_json: String,
    /// 成功保存后单调递增的版本号。
    pub(in crate::controller) revision: u64,
    /// 启动加载失败时保留的告警；成功保存后清空。
    pub(in crate::controller) load_error: Option<String>,
}

#[derive(Clone)]
struct AiConfigState {
    settings: AiSettings,
    revision: u64,
    load_error: Option<String>,
}

/// 控制平面 AI 配置服务。
pub(in crate::controller) struct AiConfigService {
    store: Arc<AiStore>,
    state: RwLock<AiConfigState>,
    write_lock: Mutex<()>,
}

impl AiConfigService {
    /// 从磁盘加载配置并建立进程内快照。
    ///
    /// 读取失败不会阻止应用启动，而是保留告警并回退到默认配置；调用方可通过
    /// [`Self::snapshot_json`] 将告警与默认快照一起下发给 UI。
    pub(in crate::controller) async fn load(store: Arc<AiStore>) -> Self {
        let load_store = store.clone();
        let (settings, load_error) =
            match tokio::task::spawn_blocking(move || load_store.load_settings()).await {
                Ok(Ok(settings)) => (settings, None),
                Ok(Err(err)) => {
                    log::warn!("加载 AI 配置失败，使用默认配置继续运行: {err}");
                    (AiSettings::default(), Some(err.to_string()))
                }
                Err(err) => {
                    log::warn!("AI 配置加载任务异常结束，使用默认配置继续运行: {err}");
                    (
                        AiSettings::default(),
                        Some(format!("AI 配置加载任务异常结束: {err}")),
                    )
                }
            };
        Self::from_parts(store, settings, load_error)
    }

    /// 使用指定配置建立服务，供测试或明确的降级路径使用。
    #[cfg(test)]
    pub(in crate::controller) fn new(store: Arc<AiStore>, settings: AiSettings) -> Self {
        Self::from_parts(store, settings, None)
    }

    fn from_parts(store: Arc<AiStore>, settings: AiSettings, load_error: Option<String>) -> Self {
        Self {
            store,
            state: RwLock::new(AiConfigState {
                settings,
                revision: 0,
                load_error,
            }),
            write_lock: Mutex::new(()),
        }
    }

    /// 获取当前配置快照。
    pub(in crate::controller) fn snapshot(&self) -> AiSettings {
        self.state.read().settings.clone()
    }

    /// 获取当前配置版本号。
    pub(in crate::controller) fn revision(&self) -> u64 {
        self.state.read().revision
    }

    /// 获取当前一致的版本化 JSON 快照。
    pub(in crate::controller) fn snapshot_json(&self) -> Result<AiConfigSnapshot, AiError> {
        let state = self.state.read();
        let settings_json =
            serde_json::to_string(&state.settings).map_err(|err| AiError::StorageFailed {
                target: "序列化 AI 设置".to_owned(),
                reason: err.to_string(),
            })?;
        Ok(AiConfigSnapshot {
            settings_json,
            revision: state.revision,
            load_error: state.load_error.clone(),
        })
    }

    /// 解析并保存 JSON 配置，成功后更新内存快照。
    pub(in crate::controller) async fn replace_json(
        &self,
        settings_json: &str,
    ) -> Result<u64, AiError> {
        let settings: AiSettings =
            serde_json::from_str(settings_json).map_err(|err| AiError::StorageFailed {
                target: "解析 AI 设置".to_owned(),
                reason: err.to_string(),
            })?;
        self.replace(settings).await
    }

    /// 串行保存完整配置，成功后才替换运行期快照并递增版本。
    pub(in crate::controller) async fn replace(
        &self,
        settings: AiSettings,
    ) -> Result<u64, AiError> {
        let _guard = self.write_lock.lock().await;
        let store = self.store.clone();
        let persisted = settings.clone();
        tokio::task::spawn_blocking(move || store.save_settings(&persisted))
            .await
            .map_err(|err| AiError::StorageFailed {
                target: "AI 配置保存任务".to_owned(),
                reason: err.to_string(),
            })??;
        let mut state = self.state.write();
        let revision = state.revision.saturating_add(1);
        *state = AiConfigState {
            settings,
            revision,
            load_error: None,
        };
        Ok(revision)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use transom_ai::AiSettings;
    use transom_store::AiStore;

    use super::AiConfigService;

    #[tokio::test]
    async fn replace_updates_snapshot_only_after_successful_persist() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let store = Arc::new(AiStore::with_root(dir.path().join("ai")));
        let service = AiConfigService::new(store.clone(), AiSettings::default());
        let next = AiSettings {
            default_model: "model-1".to_owned(),
            ..AiSettings::default()
        };

        let revision = service.replace(next.clone()).await.expect("保存配置失败");
        assert_eq!(revision, 1);
        assert_eq!(service.snapshot().default_model, "model-1");
        assert_eq!(service.snapshot_json().expect("读取快照失败").revision, 1);
        assert_eq!(
            store.load_settings().expect("读取配置失败").default_model,
            "model-1"
        );
    }

    #[tokio::test]
    async fn failed_replace_keeps_snapshot_and_revision_unchanged() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let bad_root = dir.path().join("not-a-directory");
        std::fs::write(&bad_root, "file").expect("创建测试文件失败");
        let broken = AiConfigService::new(
            Arc::new(AiStore::with_root(bad_root)),
            AiSettings::default(),
        );
        let next = AiSettings {
            default_model: "must-not-apply".to_owned(),
            ..AiSettings::default()
        };

        assert!(broken.replace(next).await.is_err());
        let snapshot = broken.snapshot_json().expect("读取失败快照失败");
        assert!(snapshot.settings_json.contains("\"default_model\":\"\""));
        assert_eq!(snapshot.revision, 0);
    }

    #[tokio::test]
    async fn load_failure_exposes_default_snapshot_and_warning() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let root = dir.path().join("ai");
        std::fs::create_dir_all(&root).expect("创建 AI 目录失败");
        std::fs::write(root.join("config.json"), "{invalid").expect("写入损坏配置失败");
        let service = AiConfigService::load(Arc::new(AiStore::with_root(root))).await;

        let snapshot = service.snapshot_json().expect("读取失败快照失败");
        assert_eq!(snapshot.revision, 0);
        assert!(snapshot.load_error.is_some());
        assert!(snapshot.settings_json.contains("\"default_model\":\"\""));
    }

    #[tokio::test]
    async fn concurrent_saves_keep_snapshot_consistent_with_disk() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let store = Arc::new(AiStore::with_root(dir.path().join("ai")));
        let service = AiConfigService::new(store.clone(), AiSettings::default());
        let first = AiSettings {
            default_model: "model-a".to_owned(),
            ..AiSettings::default()
        };
        let second = AiSettings {
            default_model: "model-b".to_owned(),
            ..AiSettings::default()
        };

        let (first_result, second_result) =
            tokio::join!(service.replace(first), service.replace(second));
        first_result.expect("第一次并发保存失败");
        second_result.expect("第二次并发保存失败");

        let snapshot = service.snapshot_json().expect("读取并发快照失败");
        let disk = store.load_settings().expect("读取并发配置失败");
        assert_eq!(snapshot.revision, 2);
        assert_eq!(
            snapshot.settings_json,
            serde_json::to_string(&disk).expect("序列化磁盘配置失败")
        );
    }
}
