//! 领域 10：AI 长期记忆读写、自定义 Slash 技能管理与 BM25 知识检索。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use transom_ai::{
    Bm25Document, Bm25Index, KnowledgeSearcher, MemoryProposal, MemoryWriter, SkillLoader,
    ToolError, append_host_fact, delete_custom_skill, load_skills_with_disabled,
    read_skill_resource, save_custom_skill,
};
use transom_store::AiStore;

use crate::controller::ai_ops::AiConfigService;

use super::ControllerAppActionBridge;

/// 按需加载技能 SOP 与同目录参考资源的桥接器（实现渐进式披露）。
pub(in crate::controller::ai_ops) struct StoreSkillLoader {
    pub(in crate::controller::ai_ops) store: Arc<AiStore>,
    pub(in crate::controller::ai_ops) ai_config: Arc<AiConfigService>,
}

#[async_trait]
impl SkillLoader for StoreSkillLoader {
    async fn load_skill(&self, name: &str, resource: Option<&str>) -> Result<String, ToolError> {
        let store = self.store.clone();
        let disabled = self.ai_config.snapshot().disabled_skills;
        let wanted = name.trim().trim_start_matches('/').to_owned();
        let res_rel = resource
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        tokio::task::spawn_blocking(move || {
            let skills = load_skills_with_disabled(Some(&store.custom_skills_dir()), &disabled);
            let skill = skills
                .iter()
                .find(|s| {
                    s.id.eq_ignore_ascii_case(&wanted)
                        || s.command
                            .trim_start_matches('/')
                            .eq_ignore_ascii_case(&wanted)
                })
                .ok_or_else(|| ToolError::InvalidArguments {
                    tool: "load_skill",
                    reason: format!("未找到名为 `{wanted}` 的技能"),
                })?;
            if let Some(rel) = res_rel {
                return read_skill_resource(skill, &rel).map_err(|e| ToolError::ExecutionFailed {
                    tool: "load_skill",
                    reason: e.to_string(),
                });
            }
            let res_section = if skill.resources.is_empty() {
                String::new()
            } else {
                format!(
                    "\n\n### 附带参考资源（可再次调用 load_skill 并传入 resource 读取）\n- {}",
                    skill.resources.join("\n- ")
                )
            };
            Ok(format!(
                "# {} ({})\n{}\n\n{}{}",
                skill.title, skill.command, skill.description, skill.prompt_template, res_section
            ))
        })
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "load_skill",
            reason: e.to_string(),
        })?
    }
}

/// 写入主机长期记忆的存储桥接。
pub(in crate::controller::ai_ops) struct StoreMemoryBridge {
    pub(in crate::controller::ai_ops) store: Arc<AiStore>,
}

#[async_trait]
impl MemoryWriter for StoreMemoryBridge {
    async fn propose_host_memory(&self, proposal: MemoryProposal) -> Result<String, ToolError> {
        let snap = self
            .store
            .load_memory(Some(&proposal.host_id))
            .map_err(|err| ToolError::ExecutionFailed {
                tool: "propose_host_memory",
                reason: err.to_string(),
            })?;
        let updated = append_host_fact(&snap.host_markdown, &proposal.fact, &proposal.category);
        self.store
            .save_host_memory(&proposal.host_id, &updated)
            .map_err(|err| ToolError::ExecutionFailed {
                tool: "propose_host_memory",
                reason: err.to_string(),
            })?;
        Ok(format!(
            "已将经验写入主机 {} 的长期记忆。",
            proposal.host_name
        ))
    }
}

/// 基于目录元数据指纹的跨轮次 BM25 索引缓存（避免每次检索全量读取 `.md` 文件并重建倒排表）。
struct CachedKnowledgeIndex {
    root: std::path::PathBuf,
    fingerprint: KnowledgeFingerprint,
    index: Option<Arc<Bm25Index>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct KnowledgeFingerprint {
    file_count: usize,
    total_bytes: u64,
    latest_mtime_nanos: u128,
}

static KNOWLEDGE_INDEX_CACHE: std::sync::LazyLock<
    parking_lot::Mutex<Option<CachedKnowledgeIndex>>,
> = std::sync::LazyLock::new(|| parking_lot::Mutex::new(None));

fn compute_knowledge_fingerprint(dirs: &[std::path::PathBuf]) -> KnowledgeFingerprint {
    let mut fp = KnowledgeFingerprint {
        file_count: 0,
        total_bytes: 0,
        latest_mtime_nanos: 0,
    };
    let mut stack: Vec<std::path::PathBuf> = dirs.to_vec();
    while let Some(dir) = stack.pop() {
        if fp.file_count >= 128 || !dir.exists() {
            continue;
        }
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            if fp.file_count >= 128 {
                break;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md")
                && let Ok(meta) = entry.metadata()
                && meta.len() <= 128 * 1024
            {
                fp.file_count += 1;
                fp.total_bytes = fp.total_bytes.saturating_add(meta.len());
                if let Ok(mtime) = meta.modified()
                    && let Ok(dur) = mtime.duration_since(std::time::UNIX_EPOCH)
                {
                    fp.latest_mtime_nanos = fp.latest_mtime_nanos.max(dur.as_nanos());
                }
            }
        }
    }
    fp
}

/// 基于纯 Rust BM25 检索本地长期记忆与自定义 Runbook。
pub(in crate::controller) struct StoreKnowledgeBridge {
    pub(in crate::controller) store: Arc<AiStore>,
}

#[async_trait]
impl KnowledgeSearcher for StoreKnowledgeBridge {
    async fn search(&self, query: &str, top_k: usize) -> Result<String, ToolError> {
        let store = self.store.clone();
        let q = query.to_owned();
        tokio::task::spawn_blocking(move || {
            let root = store.root().to_path_buf();
            let dirs = [root.join("memory"), store.custom_skills_dir()];
            let fp = compute_knowledge_fingerprint(&dirs);
            let cached_index = {
                let guard = KNOWLEDGE_INDEX_CACHE.lock();
                guard
                    .as_ref()
                    .filter(|c| c.root == root && c.fingerprint == fp)
                    .map(|c| c.index.clone())
            };

            let index_opt = match cached_index {
                Some(hit) => hit,
                None => {
                    let mut docs = Vec::new();
                    for dir in &dirs {
                        collect_markdown_docs(dir, dir, &mut docs);
                    }
                    let built = if docs.is_empty() {
                        None
                    } else {
                        Some(Arc::new(Bm25Index::build(docs)))
                    };
                    *KNOWLEDGE_INDEX_CACHE.lock() = Some(CachedKnowledgeIndex {
                        root,
                        fingerprint: fp,
                        index: built.clone(),
                    });
                    built
                }
            };

            let Some(index) = index_opt else {
                return "本地尚无主机记忆或自定义运维手册可检索。".to_owned();
            };
            let hits = index.search(&q, top_k);
            if hits.is_empty() {
                return format!("未找到与 `{q}` 相关的本地记忆或手册条目。");
            }
            hits.into_iter()
                .enumerate()
                .map(|(i, h)| {
                    format!(
                        "{}. [{}] ({}, score={:.2})\n{}",
                        i + 1,
                        h.title,
                        h.source,
                        h.score,
                        h.snippet
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .await
        .map_err(|err| ToolError::ExecutionFailed {
            tool: "search_knowledge",
            reason: err.to_string(),
        })
    }
}

fn collect_markdown_docs(
    base: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<Bm25Document>,
) {
    if out.len() >= 128 || !dir.exists() {
        return;
    }
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        if out.len() >= 128 {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_docs(base, &path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md")
            && let Ok(meta) = entry.metadata()
            && meta.len() <= 128 * 1024
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .display()
                .to_string();
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("note")
                .to_owned();
            out.push(Bm25Document {
                source: rel,
                title,
                content,
            });
        }
    }
}

pub(super) async fn handle_memory_skills(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    let store = bridge.deps.ai_store.clone();
    let scope = args["scope"].as_str().unwrap_or("global").to_owned();
    let host_id = args["host_id"].as_str().map(str::to_owned);
    let content = args["content"].as_str().unwrap_or("").to_owned();
    let skill_name = args["skill_name"].as_str().unwrap_or("").to_owned();
    let title = args["title"].as_str().unwrap_or("").to_owned();
    let description = args["description"].as_str().unwrap_or("").to_owned();
    let act = action.to_owned();
    let disabled_skills = bridge.deps.ai_config.snapshot().disabled_skills;

    tokio::task::spawn_blocking(move || match act.as_str() {
        "read_memory" => {
            let snap = store.load_memory(host_id.as_deref())?;
            Ok(serde_json::to_string_pretty(&json!({
                "global_markdown": snap.global_markdown,
                "host_markdown": snap.host_markdown,
            }))
            .unwrap_or_default())
        }
        "append_memory" => {
            if content.trim().is_empty() {
                return Err(transom_ai::AiError::StorageFailed {
                    target: "memory".to_owned(),
                    reason: "写入记忆内容不能为空".to_owned(),
                });
            }
            if scope == "host" {
                let hid = host_id
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| transom_ai::AiError::StorageFailed {
                        target: "memory".to_owned(),
                        reason: "scope=host 必须提供 host_id".to_owned(),
                    })?;
                let snap = store.load_memory(Some(hid))?;
                let updated = append_host_fact(&snap.host_markdown, &content, "AI 记录");
                store.save_host_memory(hid, &updated)?;
                Ok(format!("已追加主机 {hid} 的长期记忆"))
            } else {
                let snap = store.load_memory(None)?;
                let updated = append_host_fact(&snap.global_markdown, &content, "全局记忆");
                store.save_global_memory(&updated)?;
                Ok("已追加全局长期记忆".to_owned())
            }
        }
        "replace_memory" => {
            if scope == "host" {
                let hid = host_id
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| transom_ai::AiError::StorageFailed {
                        target: "memory".to_owned(),
                        reason: "scope=host 必须提供 host_id".to_owned(),
                    })?;
                store.save_host_memory(hid, &content)?;
                Ok(format!("已更新主机 {hid} 的长期记忆"))
            } else {
                store.save_global_memory(&content)?;
                Ok("已更新全局长期记忆".to_owned())
            }
        }
        "list_skills" => {
            let all = load_skills_with_disabled(Some(&store.custom_skills_dir()), &disabled_skills);
            let rows: Vec<Value> = all
                .into_iter()
                .map(|s| {
                    json!({
                        "id": s.id,
                        "command": s.command,
                        "title": s.title,
                        "description": s.description,
                        "builtin": s.builtin,
                        "enabled": s.enabled,
                        "resources": s.resources,
                    })
                })
                .collect();
            Ok(serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_owned()))
        }
        "save_skill" => {
            let title_val = if title.is_empty() {
                &skill_name
            } else {
                &title
            };
            let path = save_custom_skill(
                &store.custom_skills_dir(),
                &skill_name,
                title_val,
                &description,
                &[],
                &content,
            )?;
            Ok(format!("已保存技能到 {}", path.display()))
        }
        "delete_skill" => {
            let removed = delete_custom_skill(&store.custom_skills_dir(), &skill_name)?;
            if removed {
                Ok(format!("已删除自定义技能 {skill_name}"))
            } else {
                Ok(format!("自定义技能 {skill_name} 不存在"))
            }
        }
        other => Err(transom_ai::AiError::StorageFailed {
            target: "ai_memory_skills".to_owned(),
            reason: format!("未知动作: {other}"),
        }),
    })
    .await
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "ai_memory_skills",
        reason: e.to_string(),
    })?
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "ai_memory_skills",
        reason: e.to_string(),
    })
}
