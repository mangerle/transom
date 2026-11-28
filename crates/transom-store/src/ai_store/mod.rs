//! AI 助手本地持久化：供应商配置、树状 JSONL 会话、三层记忆与审计日志路径。
//!
//! 设计原理（Why）：
//! - 配置文件 `ai/config.json` 仅保存供应商端点与模型能力，API Key 统一走 [`SecretStore`] 钥匙串；
//! - 会话采用轻量 `index.json` + 追加写 `sessions/<id>.jsonl`，兼顾侧栏秒开与追加低 IO；
//! - 所有动态文件名均经过 `resolve_sandbox_path` 规范化校验，杜绝路径穿越。

mod audit;
mod checkpoint;
mod memory_usage;
mod session;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use transom_ai_contracts::{AiError, AiSettings};
use transom_core::{AppError, SecretKey, SecretStore};
use zeroize::Zeroizing;

use crate::file_io::write_atomic;
use crate::paths::config_dir;

/// AI 助手本地数据目录管理器。
#[derive(Clone, Debug)]
pub struct AiStore {
    pub(super) root: PathBuf,
    pub(super) index_lock: Arc<Mutex<()>>,
    pub(super) index_cache: Arc<Mutex<Option<Vec<transom_ai_contracts::ConversationSummary>>>>,
    pub(super) usage_lock: Arc<Mutex<()>>,
    pub(super) checkpoint_lock: Arc<Mutex<()>>,
    pub(super) session_append_lock: Arc<Mutex<()>>,
    pub(super) audit_lock: Arc<Mutex<()>>,
}

impl Default for AiStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AiStore {
    /// 使用平台默认配置目录下的 `ai/` 子目录构造存储实例。
    pub fn new() -> Self {
        Self::with_root(config_dir().join("ai"))
    }

    /// 使用指定根目录构造存储实例（供单元测试隔离使用）。
    pub fn with_root(root: PathBuf) -> Self {
        Self {
            root,
            index_lock: Arc::new(Mutex::new(())),
            index_cache: Arc::new(Mutex::new(None)),
            usage_lock: Arc::new(Mutex::new(())),
            checkpoint_lock: Arc::new(Mutex::new(())),
            session_append_lock: Arc::new(Mutex::new(())),
            audit_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 读取 AI 配置（不存在时返回默认值）。
    pub fn load_settings(&self) -> Result<AiSettings, AiError> {
        let path = self.root.join("config.json");
        if !path.exists() {
            return Ok(AiSettings::default());
        }
        let text = fs::read_to_string(&path).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        serde_json::from_str(&text).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
    }

    /// 原子写入 AI 配置（不含明文 API Key）。
    pub fn save_settings(&self, settings: &AiSettings) -> Result<(), AiError> {
        let path = self.root.join("config.json");
        let payload =
            serde_json::to_string_pretty(settings).map_err(|err| AiError::StorageFailed {
                target: path.display().to_string(),
                reason: err.to_string(),
            })?;
        write_atomic(&path, payload.as_bytes()).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
    }

    /// 从 OS Keyring 读取指定供应商的 API Key。
    pub fn load_provider_api_key(
        &self,
        secrets: &dyn SecretStore,
        provider_id: &str,
    ) -> Result<Option<Zeroizing<String>>, AppError> {
        secrets.get(&SecretKey::ai_provider_key(provider_id))
    }

    /// 将指定供应商的 API Key 写入 OS Keyring。
    pub fn save_provider_api_key(
        &self,
        secrets: &dyn SecretStore,
        provider_id: &str,
        api_key: &str,
    ) -> Result<(), AppError> {
        let key = SecretKey::ai_provider_key(provider_id);
        if api_key.trim().is_empty() {
            secrets.delete(&key)
        } else {
            secrets.set(&key, api_key.trim())
        }
    }
}

pub(super) fn sanitize_id_filename(raw_id: &str, ext: &str) -> String {
    let cleaned: String = raw_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = if cleaned.is_empty() {
        "default"
    } else {
        cleaned.as_str()
    };
    format!("{stem}.{ext}")
}
