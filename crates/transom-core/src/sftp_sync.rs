//! 本地/远端目录差异计算与增量同步计划。

use crate::sftp::TransferDirection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 目录扫描得到的单侧条目元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryMeta {
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<u32>,
}

/// 相对路径在两端的差异状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirDiffStatus {
    /// 仅本地存在，需要上传
    LocalOnly,
    /// 仅远端存在，需要下载
    RemoteOnly,
    /// 两端都存在但本地更新，需要上传
    LocalNewer,
    /// 两端都存在但远端更新，需要下载
    RemoteNewer,
    /// 一端是文件、另一端是目录，不能自动同步
    TypeMismatch,
}

impl DirDiffStatus {
    /// 该差异默认建议的同步方向；类型冲突返回 `None`。
    pub fn suggested_direction(self) -> Option<TransferDirection> {
        match self {
            Self::LocalOnly | Self::LocalNewer => Some(TransferDirection::Upload),
            Self::RemoteOnly | Self::RemoteNewer => Some(TransferDirection::Download),
            Self::TypeMismatch => None,
        }
    }
}

/// 目录对比中的一条差异记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirDiffEntry {
    pub relative_path: String,
    pub local: Option<DirEntryMeta>,
    pub remote: Option<DirEntryMeta>,
    pub status: DirDiffStatus,
}

impl DirDiffEntry {
    /// 根据差异状态生成同步项；类型冲突不产生同步项。
    pub fn sync_item(&self) -> Option<SyncItem> {
        let direction = match self.status {
            DirDiffStatus::LocalOnly | DirDiffStatus::LocalNewer => SyncDirection::Upload,
            DirDiffStatus::RemoteOnly | DirDiffStatus::RemoteNewer => SyncDirection::Download,
            DirDiffStatus::TypeMismatch => return None,
        };
        let meta = match direction {
            SyncDirection::Upload => self.local.as_ref()?,
            SyncDirection::Download => self.remote.as_ref()?,
        };
        Some(SyncItem {
            relative_path: self.relative_path.clone(),
            direction,
            is_dir: meta.is_dir,
            size: if meta.is_dir { 0 } else { meta.size },
        })
    }
}

/// 目录对比结果；`entries` 仅包含存在差异的路径。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirComparison {
    pub entries: Vec<DirDiffEntry>,
    pub skipped: u64,
}

/// 单侧目录扫描条目（相对路径使用 `/` 分隔）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirSnapshotEntry {
    pub relative_path: String,
    pub meta: DirEntryMeta,
}

/// 同步方向，决定某个差异项是上传还是下载。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncDirection {
    Upload,
    Download,
}

/// 用户确认后交给后台执行的单个同步项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncItem {
    pub relative_path: String,
    pub direction: SyncDirection,
    pub is_dir: bool,
    /// 文件字节数；目录为 0，用于预计算整体进度。
    #[serde(default)]
    pub size: u64,
}

/// 比较本地与远端扫描快照，输出按路径排序的差异列表。
///
/// 设计原理（Why）：
/// - 实现初衷：目录同步只关心“需要做什么”，两端完全一致的文件不应占用
///   用户注意力，因此 `Same` 项不会进入结果；
/// - 核心优势：以相对路径做 BTreeMap 并集，天然得到稳定顺序，便于 UI
///   展示与测试；所有判断都是纯内存计算，可脱离网络单测；
/// - 代价与局限：修改时间相同但大小不同时无法判断哪端更新，固定选择
///   上传本地版本作为确定性策略，用户可在同步弹窗中取消该行。
pub fn build_dir_comparison(
    local: Vec<DirSnapshotEntry>,
    remote: Vec<DirSnapshotEntry>,
    skipped: u64,
) -> DirComparison {
    let local = local
        .into_iter()
        .map(|entry| (entry.relative_path, entry.meta))
        .collect::<BTreeMap<_, _>>();
    let remote = remote
        .into_iter()
        .map(|entry| (entry.relative_path, entry.meta))
        .collect::<BTreeMap<_, _>>();

    let mut entries = Vec::new();
    for (relative_path, local_meta) in &local {
        if let Some(remote_meta) = remote.get(relative_path) {
            if let Some(status) = compare_meta(local_meta, remote_meta) {
                entries.push(DirDiffEntry {
                    relative_path: relative_path.clone(),
                    local: Some(local_meta.clone()),
                    remote: Some(remote_meta.clone()),
                    status,
                });
            }
        } else {
            entries.push(DirDiffEntry {
                relative_path: relative_path.clone(),
                local: Some(local_meta.clone()),
                remote: None,
                status: DirDiffStatus::LocalOnly,
            });
        }
    }
    for (relative_path, remote_meta) in &remote {
        if !local.contains_key(relative_path) {
            entries.push(DirDiffEntry {
                relative_path: relative_path.clone(),
                local: None,
                remote: Some(remote_meta.clone()),
                status: DirDiffStatus::RemoteOnly,
            });
        }
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    // 祖先类型冲突会让整棵子树无法安全同步：例如本地是文件、远端是同名目录时，
    // 目录内的远端文件若继续按“仅远端”下载，会尝试在本地文件下创建父目录。
    let conflicts = entries
        .iter()
        .filter(|entry| entry.status == DirDiffStatus::TypeMismatch)
        .map(|entry| entry.relative_path.clone())
        .collect::<std::collections::HashSet<_>>();
    if !conflicts.is_empty() {
        for entry in &mut entries {
            if entry.status == DirDiffStatus::TypeMismatch {
                continue;
            }
            let mut ancestor = entry.relative_path.as_str();
            while let Some((parent, _)) = ancestor.rsplit_once('/') {
                if conflicts.contains(parent) {
                    entry.status = DirDiffStatus::TypeMismatch;
                    break;
                }
                ancestor = parent;
            }
        }
    }

    DirComparison { entries, skipped }
}

/// 比较两端都存在且类型一致的条目；完全一致返回 `None`。
fn compare_meta(local: &DirEntryMeta, remote: &DirEntryMeta) -> Option<DirDiffStatus> {
    if local.is_dir != remote.is_dir {
        return Some(DirDiffStatus::TypeMismatch);
    }
    if local.is_dir {
        return None;
    }
    if local.size == remote.size && local.mtime == remote.mtime {
        return None;
    }
    let status = match (local.mtime, remote.mtime) {
        (Some(local_time), Some(remote_time)) if local_time > remote_time => {
            DirDiffStatus::LocalNewer
        }
        (Some(local_time), Some(remote_time)) if remote_time > local_time => {
            DirDiffStatus::RemoteNewer
        }
        (None, Some(_)) => DirDiffStatus::RemoteNewer,
        // 时间相同但大小不同，或两端都缺失时间：固定以本地为更新方
        _ => DirDiffStatus::LocalNewer,
    };
    Some(status)
}

#[cfg(test)]
mod tests {
    use super::{
        DirDiffStatus, DirEntryMeta, DirSnapshotEntry, SyncDirection, build_dir_comparison,
    };
    use crate::sftp::TransferDirection;

    fn file(path: &str, size: u64, mtime: u32) -> DirSnapshotEntry {
        DirSnapshotEntry {
            relative_path: path.to_string(),
            meta: DirEntryMeta {
                is_dir: false,
                size,
                mtime: Some(mtime),
            },
        }
    }

    fn dir(path: &str) -> DirSnapshotEntry {
        DirSnapshotEntry {
            relative_path: path.to_string(),
            meta: DirEntryMeta {
                is_dir: true,
                size: 0,
                mtime: Some(1),
            },
        }
    }

    #[test]
    fn detects_all_diff_classes_and_omits_same_files() {
        let local = vec![
            file("same.txt", 1, 10),
            file("local-newer.txt", 2, 20),
            file("local-only.txt", 3, 10),
            dir("only-local-dir"),
            file("type-mismatch", 4, 10),
            file("equal-time-size-differs.txt", 5, 30),
        ];
        let remote = vec![
            file("same.txt", 1, 10),
            file("local-newer.txt", 2, 15),
            file("remote-only.txt", 6, 10),
            dir("type-mismatch"),
            file("type-mismatch/child", 1, 10),
            file("equal-time-size-differs.txt", 7, 30),
        ];
        let comparison = build_dir_comparison(local, remote, 2);
        assert_eq!(comparison.skipped, 2);
        let find = |path: &str| {
            comparison
                .entries
                .iter()
                .find(|entry| entry.relative_path == path)
                .unwrap_or_else(|| panic!("缺少差异项: {path}"))
        };
        assert_eq!(find("local-newer.txt").status, DirDiffStatus::LocalNewer);
        assert_eq!(find("local-only.txt").status, DirDiffStatus::LocalOnly);
        assert_eq!(find("only-local-dir").status, DirDiffStatus::LocalOnly);
        assert_eq!(find("remote-only.txt").status, DirDiffStatus::RemoteOnly);
        assert_eq!(find("type-mismatch").status, DirDiffStatus::TypeMismatch);
        assert_eq!(
            find("type-mismatch/child").status,
            DirDiffStatus::TypeMismatch,
            "类型冲突目录下的差异项也必须标记为冲突"
        );
        assert!(find("type-mismatch/child").sync_item().is_none());
        assert_eq!(
            find("equal-time-size-differs.txt").status,
            DirDiffStatus::LocalNewer
        );
        assert!(
            !comparison
                .entries
                .iter()
                .any(|entry| entry.relative_path == "same.txt")
        );
        assert!(
            comparison
                .entries
                .windows(2)
                .all(|pair| pair[0].relative_path <= pair[1].relative_path)
        );
    }

    #[test]
    fn sync_items_follow_newer_side_and_skip_conflicts() {
        let local_only = super::DirDiffEntry {
            relative_path: "a.txt".to_string(),
            local: Some(DirEntryMeta {
                is_dir: false,
                size: 10,
                mtime: Some(1),
            }),
            remote: None,
            status: DirDiffStatus::LocalOnly,
        };
        let item = local_only.sync_item().expect("应生成上传项");
        assert_eq!(item.direction, SyncDirection::Upload);
        assert_eq!(item.size, 10);
        assert_eq!(
            local_only.status.suggested_direction(),
            Some(TransferDirection::Upload)
        );

        let conflict = super::DirDiffEntry {
            relative_path: "conflict".to_string(),
            local: Some(DirEntryMeta {
                is_dir: false,
                size: 1,
                mtime: Some(1),
            }),
            remote: Some(DirEntryMeta {
                is_dir: true,
                size: 0,
                mtime: Some(2),
            }),
            status: DirDiffStatus::TypeMismatch,
        };
        assert!(conflict.sync_item().is_none());
        assert!(conflict.status.suggested_direction().is_none());
    }
}
