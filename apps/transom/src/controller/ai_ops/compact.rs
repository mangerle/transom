//! 手动上下文压缩（`/compact [保留重点]`）处理模块。
//!
//! 设计原理（Why）：
//! - 优先调用当前配置的默认大模型生成高质量语义摘要并突出用户的保留重点；
//! - 若未配置模型、凭证缺失或接口调用失败，平滑降级为本地结构化压缩摘要，确保 `/compact` 永不阻塞失败。

use std::sync::Arc;

use transom_ai::{
    AiSettings, SessionEntry, SessionEntryKind, build_local_compaction_summary,
    summarize_for_compaction,
};
use transom_core::{AiAppEvent, AppEvent, SecretStore};
use transom_store::AiStore;

use super::AiConfigService;

pub(super) fn spawn_compact_conversation(
    ai_set: &mut tokio::task::JoinSet<()>,
    store: Arc<AiStore>,
    ai_config: Arc<AiConfigService>,
    secret_store: Arc<dyn SecretStore>,
    event_tx: flume::Sender<AppEvent>,
    conversation_id: String,
    focus_instructions: String,
) {
    ai_set.spawn(async move {
        let Ok(tree) = store.load_session_tree(&conversation_id) else {
            return;
        };
        let branch: Vec<&SessionEntry> = tree.active_branch();
        let first_kept = branch.last().map(|e| e.id.clone()).unwrap_or_default();
        let local_fallback = build_local_compaction_summary(&branch, &focus_instructions);
        let settings = ai_config.snapshot();
        let summary = try_llm_summary(
            &store,
            secret_store.as_ref(),
            &settings,
            &branch,
            &focus_instructions,
        )
        .await
        .unwrap_or(local_fallback);

        let parent_id = tree.active_leaf_id().map(str::to_owned);
        let entry = SessionEntry::new(
            parent_id,
            SessionEntryKind::Compaction {
                summary: summary.clone(),
                first_kept_entry_id: first_kept,
            },
        );
        let conv_id_write = conversation_id.clone();
        let store_write = store.clone();
        let leaf_id = entry.id.clone();
        let append_res = tokio::task::spawn_blocking(move || {
            store_write.append_session_entry(&conv_id_write, &entry)?;
            let now = chrono::Utc::now().to_rfc3339();
            let _ = store_write.update_conversation_leaf(&conv_id_write, Some(leaf_id), now);
            Ok::<(), transom_ai::AiError>(())
        })
        .await;
        match append_res {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                log::warn!("追加上下文压缩节点失败: {err}");
                return;
            }
            Err(err) => {
                log::warn!("上下文压缩落盘任务异常: {err}");
                return;
            }
        }
        let _ = event_tx
            .send_async(AppEvent::Ai(Box::new(AiAppEvent::ContextCompacted {
                conversation_id,
                summary,
            })))
            .await;
    });
}

async fn try_llm_summary(
    store: &AiStore,
    secret_store: &dyn SecretStore,
    settings: &AiSettings,
    entries: &[&SessionEntry],
    focus: &str,
) -> Option<String> {
    let (provider, model_cfg) = settings.resolve_model("")?;
    let api_key = store
        .load_provider_api_key(secret_store, &provider.id)
        .ok()
        .flatten()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let target = settings.build_target(&model_cfg.id, None, api_key).ok()?;
    let transcript = build_local_compaction_summary(entries, focus);
    summarize_for_compaction(&target, &transcript, focus)
        .await
        .ok()
}
