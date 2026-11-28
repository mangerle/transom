//! `AiStore` 会话索引与树状 JSONL 读写单元测试。

use super::super::AiStore;
use super::{MAX_SESSION_ENTRY_BYTES, MAX_SESSION_JSONL_BYTES};
use transom_ai_contracts::{
    ChatRole, ConversationSummary, SessionEntry, SessionEntryKind, SessionScope,
};

#[cfg(unix)]
#[test]
fn session_jsonl_is_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let store = AiStore::with_root(dir.path().to_path_buf());
    let entry = SessionEntry::new(
        None,
        SessionEntryKind::Reasoning {
            text: "私有会话内容".to_owned(),
        },
    );
    store
        .append_session_entry("private-conv", &entry)
        .expect("追加会话失败");
    let path = dir.path().join("sessions").join("private-conv.jsonl");
    let mode = std::fs::metadata(path)
        .expect("读取会话文件属性失败")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn rejects_oversized_session_entry_before_write() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let store = AiStore::with_root(dir.path().to_path_buf());
    let entry = SessionEntry::new(
        None,
        SessionEntryKind::Message {
            role: ChatRole::User,
            text: "x".repeat(MAX_SESSION_ENTRY_BYTES + 1),
            images: vec![],
            model: None,
            tokens_per_sec: None,
            usage: None,
        },
    );
    let error = store
        .append_session_entry("oversized", &entry)
        .expect_err("超长记录必须被拒绝");
    assert!(error.to_string().contains("单条会话记录超过体积上限"));
    assert!(!dir.path().join("sessions").join("oversized.jsonl").exists());
}

#[test]
fn rejects_oversized_session_file_before_read() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let store = AiStore::with_root(dir.path().to_path_buf());
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions).expect("创建会话目录失败");
    let path = sessions.join("oversized.jsonl");
    let file = std::fs::File::create(&path).expect("创建会话文件失败");
    file.set_len(MAX_SESSION_JSONL_BYTES + 1)
        .expect("扩展会话文件失败");
    let error = store
        .load_session_tree("oversized")
        .expect_err("超大文件必须被拒绝");
    assert!(error.to_string().contains("会话文件超过体积上限"));
}

#[test]
fn session_tree_restores_active_branch_from_index() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let store = AiStore::with_root(dir.path().to_path_buf());
    let header = SessionEntry::new(
        None,
        SessionEntryKind::Header {
            title: "分支会话".to_owned(),
            scope: SessionScope::Global,
            model_id: "m1".to_owned(),
        },
    );
    let user = SessionEntry::new(
        Some(header.id.clone()),
        SessionEntryKind::Message {
            role: ChatRole::User,
            text: "问题".to_owned(),
            images: vec![],
            model: None,
            tokens_per_sec: None,
            usage: None,
        },
    );
    let kept = SessionEntry::new(
        Some(user.id.clone()),
        SessionEntryKind::Message {
            role: ChatRole::Assistant,
            text: "保留分支".to_owned(),
            images: vec![],
            model: None,
            tokens_per_sec: None,
            usage: None,
        },
    );
    let abandoned = SessionEntry::new(
        Some(user.id.clone()),
        SessionEntryKind::Message {
            role: ChatRole::Assistant,
            text: "放弃分支".to_owned(),
            images: vec![],
            model: None,
            tokens_per_sec: None,
            usage: None,
        },
    );
    for entry in [&header, &user, &kept, &abandoned] {
        store
            .append_session_entry("branch-conv", entry)
            .expect("追加会话失败");
    }
    store
        .upsert_conversation(ConversationSummary {
            id: "branch-conv".to_owned(),
            title: "分支会话".to_owned(),
            scope: SessionScope::Global,
            model_id: "m1".to_owned(),
            effort: None,
            updated_at: "2026-11-28T00:00:00Z".to_owned(),
            active_leaf_id: Some(kept.id.clone()),
            archived: false,
        })
        .expect("写入索引失败");

    let tree = store
        .load_session_tree("branch-conv")
        .expect("加载会话失败");
    let branch = tree.active_branch();
    assert_eq!(
        branch.last().map(|entry| entry.id.as_str()),
        Some(kept.id.as_str())
    );
    assert!(!branch.iter().any(|entry| entry.id == abandoned.id));
}

#[test]
fn session_index_and_jsonl_roundtrip() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let store = AiStore::with_root(dir.path().to_path_buf());

    let conv_id = "conv-test-1";
    store
        .upsert_conversation(ConversationSummary {
            id: conv_id.to_owned(),
            title: "巡检主机".to_owned(),
            scope: SessionScope::Host("host-a".to_owned()),
            model_id: "m1".to_owned(),
            effort: None,
            updated_at: "2026-11-26T10:00:00Z".to_owned(),
            active_leaf_id: None,
            archived: false,
        })
        .expect("写入索引失败");

    let entry = SessionEntry::new(
        None,
        SessionEntryKind::Message {
            role: ChatRole::User,
            text: "看看负载".to_owned(),
            images: vec![],
            model: None,
            tokens_per_sec: None,
            usage: None,
        },
    );
    store
        .append_session_entry(conv_id, &entry)
        .expect("追加 JSONL 失败");

    let list = store
        .list_conversations_for_scope(&SessionScope::Host("host-a".to_owned()))
        .expect("查询索引失败");
    assert_eq!(list.len(), 1);
    assert!(!list[0].archived);

    store
        .set_conversation_archived(conv_id, true)
        .expect("归档失败");
    assert!(
        store
            .list_all_conversations()
            .expect("读取会话索引失败")[0]
            .archived
    );
    store
        .set_conversation_archived(conv_id, false)
        .expect("取消归档失败");
    assert!(
        !store
            .list_all_conversations()
            .expect("读取会话索引失败")[0]
            .archived
    );

    store
        .rename_conversation(conv_id, "生产巡检记录")
        .expect("重命名失败");
    assert_eq!(
        store
            .list_all_conversations()
            .expect("读取会话索引失败")[0]
            .title,
        "生产巡检记录"
    );

    let tree = store.load_session_tree(conv_id).expect("加载会话树失败");
    assert_eq!(tree.all_entries().len(), 1);
}

#[test]
fn incremental_index_ops_replay_and_compact_at_threshold() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let root = dir.path().to_path_buf();
    let store = AiStore::with_root(root.clone());

    store
        .upsert_conversation(ConversationSummary {
            id: "c-1".to_owned(),
            title: "初始标题".to_owned(),
            scope: SessionScope::Global,
            model_id: "m1".to_owned(),
            effort: None,
            updated_at: "2026-11-28T01:00:00Z".to_owned(),
            active_leaf_id: None,
            archived: false,
        })
        .expect("写入初始会话失败");
    store
        .rename_conversation("c-1", "重命名标题")
        .expect("重命名失败");
    assert!(root.join("index.ops.jsonl").exists());

    // 新建存储实例模拟冷启动回放增量日志
    let reopened = AiStore::with_root(root.clone());
    let items = reopened.list_all_conversations().expect("冷启动加载失败");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "重命名标题");

    // 连续写入直到恰好触发阈值合并（前面已有 upsert + rename 共 2 条增量）
    for idx in 0..(super::INDEX_COMPACT_THRESHOLD - 2) {
        store
            .update_conversation_leaf(
                "c-1",
                Some(format!("leaf-{idx}")),
                format!("2026-11-28T02:00:{idx:02}Z"),
            )
            .expect("更新叶节点失败");
    }
    assert!(root.join("index.json").exists());
    assert!(!root.join("index.ops.jsonl").exists());

    let after = AiStore::with_root(root)
        .list_all_conversations()
        .expect("合并后读取失败");
    assert_eq!(after[0].active_leaf_id.as_deref(), Some("leaf-61"));
}
