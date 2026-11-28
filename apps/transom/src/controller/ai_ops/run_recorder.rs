//! AI 运行期间的会话、审计与 Token 统计落盘器。
//!
//! 设计原理（Why）：
//! - 事件泵只负责协议事件到 UI/持久化动作的转换，磁盘细节集中在 `RunRecorder`；
//! - 每次落盘都通过 `spawn_blocking` 执行并返回强类型错误，禁止 `let _ = append(...)`；
//! - 用户消息、轮次结果、工具记录和最终活动叶节点按事件消费顺序被等待写入，
//!   从结构上保证 JSONL 父子链与审计顺序一致。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::OnceCell;
use transom_ai::{AiAuditEntry, AiError, SessionEntry};
use transom_store::AiStore;

/// 绑定单个对话的运行期落盘器（缓存沙箱路径并聚合 Token 统计至轮次结束批量落盘）。
#[derive(Clone)]
pub(super) struct RunRecorder {
    store: Arc<AiStore>,
    conversation_id: String,
    prepared_path: Arc<OnceCell<PathBuf>>,
    baseline_totals: Arc<OnceCell<(u64, u64)>>,
    pending_tokens: Arc<AtomicU64>,
}

impl RunRecorder {
    /// 为指定对话创建落盘器。
    pub(super) fn new(store: Arc<AiStore>, conversation_id: String) -> Self {
        Self {
            store,
            conversation_id,
            prepared_path: Arc::new(OnceCell::new()),
            baseline_totals: Arc::new(OnceCell::new()),
            pending_tokens: Arc::new(AtomicU64::new(0)),
        }
    }

    async fn ensure_path(&self, operation: &'static str) -> Result<PathBuf, AiError> {
        let store = self.store.clone();
        let conversation_id = self.conversation_id.clone();
        let path = self
            .prepared_path
            .get_or_try_init(|| async move {
                tokio::task::spawn_blocking(move || store.prepare_session_path(&conversation_id))
                    .await
                    .map_err(|err| join_error(operation, err))?
            })
            .await?;
        Ok(path.clone())
    }

    /// 追加单条会话记录。
    pub(super) async fn append_entry(
        &self,
        operation: &'static str,
        entry: SessionEntry,
    ) -> Result<(), AiError> {
        let store = self.store.clone();
        let conversation_id = self.conversation_id.clone();
        let path = self.ensure_path(operation).await?;
        tokio::task::spawn_blocking(move || {
            store.append_session_entries_prepared(&conversation_id, &path, &[entry])
        })
        .await
        .map_err(|err| join_error(operation, err))?
    }

    /// 追加一轮推理产生的全部记录，并在内存累计 Token 用量（`finalize` 时统一写盘）。
    pub(super) async fn append_turn(
        &self,
        entries: Vec<SessionEntry>,
        added_tokens: u64,
    ) -> Result<(u64, u64), AiError> {
        let store = self.store.clone();
        let conversation_id = self.conversation_id.clone();
        let path = self.ensure_path("追加 AI 轮次记录").await?;
        let base_cell = self.baseline_totals.clone();
        let store_for_base = store.clone();

        tokio::task::spawn_blocking(move || {
            store.append_session_entries_prepared(&conversation_id, &path, &entries)
        })
        .await
        .map_err(|err| join_error("追加 AI 轮次记录", err))??;

        let (base_today, base_month) = *self
            .baseline_totals
            .get_or_try_init(|| async move {
                let _ = &base_cell;
                tokio::task::spawn_blocking(move || store_for_base.load_usage_totals())
                    .await
                    .map_err(|err| join_error("读取 AI Token 用量基线", err))?
            })
            .await?;

        let total_pending = self
            .pending_tokens
            .fetch_add(added_tokens, Ordering::Relaxed)
            .saturating_add(added_tokens);
        Ok((
            base_today.saturating_add(total_pending),
            base_month.saturating_add(total_pending),
        ))
    }

    /// 追加工具会话记录和对应审计记录。
    pub(super) async fn append_tool(
        &self,
        entry: SessionEntry,
        audit: AiAuditEntry,
    ) -> Result<(), AiError> {
        let store = self.store.clone();
        let conversation_id = self.conversation_id.clone();
        let path = self.ensure_path("追加 AI 工具记录").await?;
        tokio::task::spawn_blocking(move || {
            store.append_session_entries_prepared(&conversation_id, &path, &[entry])?;
            store.append_audit(&audit)
        })
        .await
        .map_err(|err| join_error("追加 AI 工具记录", err))?
    }

    /// 一次性落盘本次运行累计的 Token 用量并更新对话索引活动叶节点。
    pub(super) async fn finalize(&self, active_leaf_id: Option<String>) -> Result<(), AiError> {
        let store = self.store.clone();
        let conversation_id = self.conversation_id.clone();
        let updated_at = chrono::Utc::now().to_rfc3339();
        let pending = self.pending_tokens.swap(0, Ordering::AcqRel);
        tokio::task::spawn_blocking(move || {
            if pending > 0 {
                store.record_token_usage(pending)?;
            }
            store.update_conversation_leaf(&conversation_id, active_leaf_id, updated_at)
        })
        .await
        .map_err(|err| join_error("更新 AI 对话活动叶节点", err))?
    }
}

fn join_error(operation: &'static str, err: tokio::task::JoinError) -> AiError {
    AiError::StorageFailed {
        target: operation.to_owned(),
        reason: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use transom_ai::{ChatRole, SessionEntry, SessionEntryKind};
    use transom_store::AiStore;

    use super::RunRecorder;

    fn message(text: &str) -> SessionEntry {
        SessionEntry::new(
            None,
            SessionEntryKind::Message {
                role: ChatRole::User,
                text: text.to_owned(),
                images: Vec::new(),
                model: None,
                tokens_per_sec: None,
                usage: None,
            },
        )
    }

    #[tokio::test]
    async fn recorder_appends_entries_and_reports_storage_failure() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let store = Arc::new(AiStore::with_root(dir.path().join("ai")));
        let recorder = RunRecorder::new(store.clone(), "conv-1".to_owned());
        recorder
            .append_entry("追加测试消息", message("hello"))
            .await
            .expect("追加消息失败");
        let tree = store.load_session_tree("conv-1").expect("读取会话失败");
        assert_eq!(tree.all_entries().len(), 1);

        let bad_root = dir.path().join("not-a-directory");
        std::fs::write(&bad_root, "file").expect("创建测试文件失败");
        let broken = RunRecorder::new(Arc::new(AiStore::with_root(bad_root)), "conv-2".to_owned());
        assert!(
            broken
                .append_entry("追加失败消息", message("fail"))
                .await
                .is_err()
        );
    }
}
