//! 多端目录三路合并：以共同祖先为基准，尽量保留两端各自的改动。
//!
//! 设计原理（Why）：
//! - 纯"后写者胜"会让另一台设备上刚改好的会话被静默覆盖；三路合并用共同
//!   祖先判断"哪一端真的改了"，两端都改才视为冲突；
//! - 冲突裁决优先看会话的 `updated_at` 时间戳（较新者胜），时间戳缺失或
//!   相等时保留本机版本：宁可让用户在本机再改一次，也不静默替换他眼前的
//!   配置；
//! - 删除同样参与合并：一端删除、另一端未改（等于祖先）才真正删除；
//!   另一端改过则保留修改，避免"另一台设备删了会话，这台设备刚编辑的内容
//!   直接消失"。
//!
//! 代价与局限：文件夹与凭据模板没有逐条时间戳，冲突一律本机优先；合并按
//! 稳定 ID 对齐，重名条目不会被合并。

use crate::catalog::SessionCatalog;
use serde::Serialize;
use std::collections::HashMap;
use std::hash::Hash;

/// 合并统计，供界面提示"带回了多少远端改动"。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncMergeReport {
    pub sessions_added: usize,
    pub sessions_updated: usize,
    pub sessions_removed: usize,
    pub folders_added: usize,
    pub folders_updated: usize,
    pub identities_added: usize,
    pub identities_updated: usize,
    /// 两端都改动且无法自动裁决、最终按本机版本保留的条目数。
    pub conflicts: usize,
}

/// 三路合并两个目录快照；`base` 为上一次成功同步的快照（没有则退化为两路）。
pub fn merge_catalogs(
    base: Option<&SessionCatalog>,
    local: &SessionCatalog,
    remote: &SessionCatalog,
) -> (SessionCatalog, SyncMergeReport) {
    let mut report = SyncMergeReport::default();
    let sessions = merge_keyed(
        base.map(|catalog| &catalog.sessions[..]),
        &local.sessions,
        &remote.sessions,
        |session| session.id,
        |local, remote| {
            // 时间戳只做"谁更新"的判据；缺时间戳时本机优先。
            if remote.updated_at > local.updated_at {
                remote.clone()
            } else {
                local.clone()
            }
        },
        &mut report.conflicts,
    );
    report.sessions_added = sessions.added;
    report.sessions_updated = sessions.updated;
    report.sessions_removed = sessions.removed;
    let folders = merge_keyed(
        base.map(|catalog| &catalog.folders[..]),
        &local.folders,
        &remote.folders,
        |folder| folder.id,
        |local, _remote| local.clone(),
        &mut report.conflicts,
    );
    report.folders_added = folders.added;
    report.folders_updated = folders.updated;
    let identities = merge_keyed(
        base.map(|catalog| &catalog.identities[..]),
        &local.identities,
        &remote.identities,
        |identity| identity.id,
        |local, _remote| local.clone(),
        &mut report.conflicts,
    );
    report.identities_added = identities.added;
    report.identities_updated = identities.updated;
    (
        SessionCatalog {
            folders: folders.items,
            sessions: sessions.items,
            identities: identities.items,
        },
        report,
    )
}

struct MergedItems<T> {
    items: Vec<T>,
    added: usize,
    updated: usize,
    removed: usize,
}

/// 通用按 ID 合并：保持本机顺序，远端新增项追加在末尾。
///
/// 用序列化指纹代替 `PartialEq`：目录条目类型分属多个模块，有的并未实现
/// `PartialEq`；JSON 指纹与持久化口径一致，字段新增也不会漏比较。
fn merge_keyed<T, K, F, N>(
    base: Option<&[T]>,
    local: &[T],
    remote: &[T],
    key: F,
    prefer: N,
    conflicts: &mut usize,
) -> MergedItems<T>
where
    T: Clone + Serialize,
    K: Eq + Hash + Copy,
    F: Fn(&T) -> K,
    N: Fn(&T, &T) -> T,
{
    fn fingerprint<T: Serialize>(item: &T) -> Vec<u8> {
        serde_json::to_vec(item).unwrap_or_default()
    }
    let base: HashMap<K, &T> = base
        .unwrap_or_default()
        .iter()
        .map(|item| (key(item), item))
        .collect();
    let remote: HashMap<K, &T> = remote.iter().map(|item| (key(item), item)).collect();
    let local_keys: std::collections::HashSet<K> = local.iter().map(&key).collect();
    let mut items = Vec::with_capacity(local.len() + remote.len());
    let mut added = 0;
    let mut updated = 0;
    let mut removed = 0;
    for item in local {
        let id = key(item);
        let local_fp = fingerprint(item);
        match remote.get(&id) {
            Some(remote_item) => {
                let remote_fp = fingerprint(*remote_item);
                if local_fp == remote_fp {
                    items.push(item.clone());
                } else if let Some(ancestor) = base.get(&id) {
                    let ancestor_fp = fingerprint(*ancestor);
                    if local_fp == ancestor_fp {
                        // 本机未改、远端改了：采用远端。
                        items.push((*remote_item).clone());
                        updated += 1;
                    } else if remote_fp == ancestor_fp {
                        // 远端未改、本机改了：保留本机。
                        items.push(item.clone());
                    } else {
                        items.push(prefer(item, remote_item));
                        *conflicts += 1;
                    }
                } else {
                    items.push(prefer(item, remote_item));
                    *conflicts += 1;
                }
            }
            None => {
                // 远端没有：只有"本机也没改（等于祖先）"才视为远端删除。
                if base
                    .get(&id)
                    .is_some_and(|ancestor| local_fp == fingerprint(*ancestor))
                {
                    removed += 1;
                } else {
                    items.push(item.clone());
                }
            }
        }
    }
    for (id, remote_item) in &remote {
        if !local_keys.contains(id) {
            let remote_fp = fingerprint(*remote_item);
            let deleted_here = base
                .get(id)
                .is_some_and(|ancestor| fingerprint(*ancestor) == remote_fp);
            if !deleted_here {
                items.push((*remote_item).clone());
                added += 1;
            }
        }
    }
    MergedItems {
        items,
        added,
        updated,
        removed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::SessionFolder;
    use crate::identity::Identity;
    use crate::ids::{FolderId, IdentityId, SessionId};
    use crate::session::{AuthMethod, Connection, Session, SshConnection};

    // 让编译器确认键类型与目录条目字段一致（改字段时会在这里报错）。
    fn _key_types(
        _: SessionId,
        _: FolderId,
        _: IdentityId,
        _: &Session,
        _: &SessionFolder,
        _: &Identity,
    ) {
    }

    fn session(name: &str, host: &str, updated_at: u64) -> Session {
        let mut session = Session::new(
            name,
            Connection::Ssh(SshConnection {
                host: host.into(),
                port: 22,
                username: "deploy".into(),
                auth: AuthMethod::Agent,
                options: Default::default(),
                proxy_jump: None,
            }),
        );
        session.updated_at = updated_at;
        session
    }

    fn catalog(sessions: Vec<Session>) -> SessionCatalog {
        SessionCatalog {
            sessions,
            ..Default::default()
        }
    }

    #[test]
    fn remote_only_and_local_only_sessions_are_kept() {
        let local = catalog(vec![session("web", "web.local", 10)]);
        let remote = catalog(vec![session("db", "db.remote", 20)]);
        let (merged, report) = merge_catalogs(None, &local, &remote);
        let names: Vec<&str> = merged.sessions.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["web", "db"]);
        assert_eq!(report.sessions_added, 1);
    }

    #[test]
    fn unchanged_local_takes_remote_update() {
        let ancestor = session("web", "old", 10);
        let local = catalog(vec![ancestor.clone()]);
        let mut remote_session = ancestor.clone();
        remote_session.name = "web-renamed".into();
        remote_session.updated_at = 20;
        let remote = catalog(vec![remote_session]);
        let (merged, report) = merge_catalogs(Some(&catalog(vec![ancestor])), &local, &remote);
        assert_eq!(merged.sessions[0].name, "web-renamed");
        assert_eq!(report.sessions_updated, 1);
        assert_eq!(report.conflicts, 0);
    }

    #[test]
    fn local_change_wins_when_remote_is_unchanged() {
        let ancestor = session("web", "old", 10);
        let mut local_session = ancestor.clone();
        local_session.name = "local-name".into();
        let local = catalog(vec![local_session]);
        let remote = catalog(vec![ancestor.clone()]);
        let (merged, _) = merge_catalogs(Some(&catalog(vec![ancestor])), &local, &remote);
        assert_eq!(merged.sessions[0].name, "local-name");
    }

    #[test]
    fn both_changed_prefers_newer_timestamp() {
        let ancestor = session("web", "old", 10);
        let mut local_session = ancestor.clone();
        local_session.name = "local".into();
        local_session.updated_at = 50;
        let mut remote_session = ancestor.clone();
        remote_session.name = "remote".into();
        remote_session.updated_at = 60;
        let (merged, report) = merge_catalogs(
            Some(&catalog(vec![ancestor])),
            &catalog(vec![local_session]),
            &catalog(vec![remote_session]),
        );
        assert_eq!(merged.sessions[0].name, "remote");
        assert_eq!(report.conflicts, 1);
    }

    #[test]
    fn both_changed_equal_timestamp_keeps_local() {
        let ancestor = session("web", "old", 10);
        let mut local_session = ancestor.clone();
        local_session.name = "local".into();
        local_session.updated_at = 50;
        let mut remote_session = ancestor.clone();
        remote_session.name = "remote".into();
        remote_session.updated_at = 50;
        let (merged, _) = merge_catalogs(
            Some(&catalog(vec![ancestor])),
            &catalog(vec![local_session]),
            &catalog(vec![remote_session]),
        );
        assert_eq!(merged.sessions[0].name, "local");
    }

    #[test]
    fn deletion_applies_only_to_unmodified_side() {
        let ancestor = session("web", "old", 10);
        // 本机未改、远端删除：应从合并结果中消失。
        let local = catalog(vec![ancestor.clone()]);
        let remote = catalog(vec![]);
        let (merged, report) =
            merge_catalogs(Some(&catalog(vec![ancestor.clone()])), &local, &remote);
        assert!(merged.sessions.is_empty());
        assert_eq!(report.sessions_removed, 1);

        // 本机改过、远端删除：保留本机修改。
        let mut changed = ancestor.clone();
        changed.name = "kept".into();
        let local = catalog(vec![changed]);
        let (merged, _) = merge_catalogs(Some(&catalog(vec![ancestor])), &local, &remote);
        assert_eq!(merged.sessions[0].name, "kept");
    }

    #[test]
    fn no_base_conflict_prefers_newer_timestamp() {
        let origin = session("web", "local", 10);
        let mut local_session = origin.clone();
        local_session.name = "local-name".into();
        let mut remote_session = origin;
        remote_session.name = "remote-name".into();
        remote_session.updated_at = 20;
        let (merged, report) = merge_catalogs(
            None,
            &catalog(vec![local_session]),
            &catalog(vec![remote_session]),
        );
        assert_eq!(
            merged.sessions[0].name, "remote-name",
            "无共同祖先时按时间戳取较新者"
        );
        assert_eq!(report.conflicts, 1);
    }
}
