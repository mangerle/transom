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

/// 单个文件名允许的最大字节数，为常见文件系统的 255 字节限制预留空间。
const MAX_FILE_NAME_BYTES: usize = 240;

/// 将外部标识编码为可逆、无碰撞的文件名。
///
/// 旧实现把每类非法字符都替换为 `_`，会让不同标识映射到同一路径并互相
/// 覆盖。百分号编码只保留 ASCII 字母、数字、`-` 和 `_`，其余 UTF-8 字节
/// 原样编码，因此既不可能产生路径分隔符，也不会丢失原始身份信息。代价是
/// 非 ASCII 标识的文件名会变长，超限时明确拒绝而不是冒险截断。
pub(super) fn encode_id_filename(raw_id: &str, ext: &str) -> Result<String, AiError> {
    let stem = if raw_id.is_empty() { "default" } else { raw_id };
    let mut encoded =
        String::with_capacity(stem.len().saturating_mul(3).saturating_add(ext.len() + 1));
    for byte in stem.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(&mut encoded, "%{byte:02X}").map_err(|err| AiError::StorageFailed {
                target: raw_id.to_owned(),
                reason: format!("编码 AI 文件名失败: {err}"),
            })?;
        }
    }
    encoded.push('.');
    encoded.push_str(ext);
    if encoded.len() > MAX_FILE_NAME_BYTES {
        return Err(AiError::StorageFailed {
            target: raw_id.to_owned(),
            reason: format!(
                "AI 文件名编码后超过体积上限（实际 {} 字节，上限 {MAX_FILE_NAME_BYTES} 字节）",
                encoded.len()
            ),
        });
    }
    Ok(encoded)
}

/// 解析动态标识对应的沙箱文件，并迁移旧版下划线替换命名。
///
/// 新文件名保持旧版安全标识不变，因此绝大多数存量文件无需迁移；仅当旧版
/// 文件名与新名称不同且旧文件存在时，才原子重命名一次。若两个旧标识曾映射
/// 到同一文件，数据本身已无法区分，只能保留先访问到的会话。
pub(super) fn resolve_id_file_path(
    root: &Path,
    raw_id: &str,
    ext: &str,
) -> Result<PathBuf, AiError> {
    let encoded = encode_id_filename(raw_id, ext)?;
    let path = transom_ai_contracts::resolve_sandbox_path(root, Path::new(&encoded))?;
    if path.exists() {
        return Ok(path);
    }

    let legacy_stem: String = if raw_id.is_empty() {
        "default".to_owned()
    } else {
        raw_id
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect()
    };
    let legacy_name = format!("{legacy_stem}.{ext}");
    let legacy_path = transom_ai_contracts::resolve_sandbox_path(root, Path::new(&legacy_name))?;
    if legacy_path != path && legacy_path.exists() {
        fs::rename(&legacy_path, &path).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: format!("迁移旧版 AI 文件失败: {err}"),
        })?;
    }
    Ok(path)
}

#[cfg(test)]
mod filename_tests {
    use super::{encode_id_filename, resolve_id_file_path};

    #[test]
    fn encoded_ids_are_reversible_and_collision_free() {
        let slash = encode_id_filename("a/b", "jsonl").expect("编码斜杠标识失败");
        let question = encode_id_filename("a?b", "jsonl").expect("编码问号标识失败");
        let literal_escape = encode_id_filename("a%2Fb", "jsonl").expect("编码百分号标识失败");
        let traversal = encode_id_filename("../../escape", "jsonl").expect("编码越权标识失败");

        assert_ne!(slash, question);
        assert_ne!(slash, literal_escape);
        assert!(!traversal.contains('/'));
        assert!(!traversal.contains('\\'));
    }

    #[test]
    fn legacy_encoded_file_is_migrated_without_losing_content() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let legacy = dir.path().join("a_b.jsonl");
        std::fs::write(&legacy, "legacy").expect("写入旧文件失败");

        let migrated = resolve_id_file_path(dir.path(), "a/b", "jsonl").expect("迁移旧文件失败");

        assert!(!legacy.exists());
        assert_eq!(
            std::fs::read_to_string(migrated).expect("读取迁移文件失败"),
            "legacy"
        );
    }
}
