//! 多端目录合并：以共同祖先为基准，尽量保留两端各自的改动。
//!
//! 设计原理（Why）：
//! - 会话带 `updated_at`：无论两路还是三路，始终按时间戳较新者胜；
//!   时间戳相等时按 [`MergePreference`] 裁决；
//! - 文件夹与凭据模板没有逐条时间戳，两端都改时只能按操作方向裁决：
//!   拉取/导入以远端为目标（Remote），推送/Git 以本机为提交者（Local）；
//! - 三路合并用共同祖先判断"哪一端真的改了"，只有两端都改才按上述规则
//!   裁决；删除同样参与合并，一端删除、另一端未改才真正删除。
//!
//! 代价与局限：没有共同祖先时（推送/拉取）无法区分"远端删除"与"本机新增"，
//! 删除不会传播；文件夹覆盖仍保留本机折叠态。合并按稳定 ID 对齐。

use super::{EngineFolder, SessionCatalog, SessionFolder};
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
    /// 引擎连接的新增/更新计数。
    pub engines_added: usize,
    pub engines_updated: usize,
    /// 引擎分组的新增/更新计数。
    pub engine_folders_added: usize,
    pub engine_folders_updated: usize,
    /// 需要人工关注的冲突条目数：三路两端都改，或无共同祖先时最终保留本机版本。
    pub conflicts: usize,
}

/// 合并冲突的裁决方向。
///
/// 设计原理（Why）：会话有 `updated_at`，始终按时间戳裁决；文件夹与凭据
/// 模板没有逐条时间戳，冲突只能按"这次操作以哪一侧为目标状态"决定。
/// 拉取/导入把远端快照带入本机，取远端；推送/Git 以本机为提交者，取本机。
/// 把方向显式化为参数，三条通道共用一套合并实现。
///
/// 代价与局限：方向参数不解决删除传播；删除仍需要共同祖先快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergePreference {
    /// 冲突时保留本机版本（推送、Git、以及无时间戳类型的本机改动）。
    Local,
    /// 冲突时采用远端版本（拉取、备份导入）。
    Remote,
}

/// 合并两个目录快照；`base` 为上一次成功同步的快照（没有则退化为两路）。
///
/// `preference` 只在"两端都改且无法用时间戳裁决"时生效：会话时间戳相等，
/// 或文件夹/凭据模板冲突。它表达本次操作的目标方向，不改变三路合并对
/// "单端改动"的判定。
pub fn merge_catalogs(
    base: Option<&SessionCatalog>,
    local: &SessionCatalog,
    remote: &SessionCatalog,
    preference: MergePreference,
) -> (SessionCatalog, SyncMergeReport) {
    let mut report = SyncMergeReport::default();
    let sessions = merge_keyed(
        base.map(|catalog| &catalog.sessions[..]),
        &local.sessions,
        &remote.sessions,
        |session| session.id,
        |local, remote| {
            if remote.updated_at > local.updated_at {
                (remote.clone(), true)
            } else if remote.updated_at < local.updated_at {
                (local.clone(), false)
            } else {
                match preference {
                    MergePreference::Local => (local.clone(), false),
                    MergePreference::Remote => (remote.clone(), true),
                }
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
        |local, remote| match preference {
            MergePreference::Local => (local.clone(), false),
            // 文件夹覆盖保留本机折叠态：收纳是每台设备的界面偏好。
            MergePreference::Remote => (
                SessionFolder {
                    collapsed: local.collapsed,
                    ..remote.clone()
                },
                true,
            ),
        },
        &mut report.conflicts,
    );
    report.folders_added = folders.added;
    report.folders_updated = folders.updated;
    let identities = merge_keyed(
        base.map(|catalog| &catalog.identities[..]),
        &local.identities,
        &remote.identities,
        |identity| identity.id,
        |local, remote| match preference {
            MergePreference::Local => (local.clone(), false),
            MergePreference::Remote => (remote.clone(), true),
        },
        &mut report.conflicts,
    );
    report.identities_added = identities.added;
    report.identities_updated = identities.updated;
    let engines = merge_keyed(
        base.map(|catalog| &catalog.engines[..]),
        &local.engines,
        &remote.engines,
        |engine| engine.id,
        |local, remote| {
            if remote.updated_at > local.updated_at {
                (remote.clone(), true)
            } else if remote.updated_at < local.updated_at {
                (local.clone(), false)
            } else {
                match preference {
                    MergePreference::Local => (local.clone(), false),
                    MergePreference::Remote => (remote.clone(), true),
                }
            }
        },
        &mut report.conflicts,
    );
    report.engines_added = engines.added;
    report.engines_updated = engines.updated;
    let engine_folders = merge_keyed(
        base.map(|catalog| &catalog.engine_folders[..]),
        &local.engine_folders,
        &remote.engine_folders,
        |folder| folder.id,
        |local, remote| match preference {
            MergePreference::Local => (local.clone(), false),
            // 与会话文件夹同一口径：折叠是每台设备的界面偏好，覆盖时保留本机值。
            MergePreference::Remote => (
                EngineFolder {
                    collapsed: local.collapsed,
                    ..remote.clone()
                },
                true,
            ),
        },
        &mut report.conflicts,
    );
    report.engine_folders_added = engine_folders.added;
    report.engine_folders_updated = engine_folders.updated;
    (
        SessionCatalog {
            folders: folders.items,
            sessions: sessions.items,
            identities: identities.items,
            engines: engines.items,
            engine_folders: engine_folders.items,
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
    N: Fn(&T, &T) -> (T, bool),
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
                        let (chosen, _remote_chosen) = prefer(item, remote_item);
                        items.push(chosen);
                        *conflicts += 1;
                    }
                } else {
                    // 无共同祖先：虽然无法判断"谁改了什么"，但只要最终采用
                    // 远端版本，对界面就是一次更新；保留本机版本才计为冲突。
                    let (chosen, remote_chosen) = prefer(item, remote_item);
                    if remote_chosen {
                        updated += 1;
                    } else {
                        *conflicts += 1;
                    }
                    items.push(chosen);
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
mod tests;
