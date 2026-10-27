//! 标签栏并排条目构建逻辑。

use super::tabs::TiledTabs;
use super::types::{StripEntry, TilingId};
use std::collections::{HashMap, HashSet};
use transom_core::GroupId;

/// 标签栏的可见条目：每张并排网格占一条，其余标签各占一条。
pub(super) fn build_strip_entries(groups: &[GroupId], sets: &[TiledTabs]) -> Vec<StripEntry> {
    let mut hidden: HashSet<GroupId> = HashSet::with_capacity(groups.len());
    let mut representatives: HashMap<usize, (TilingId, GroupId)> =
        HashMap::with_capacity(sets.len());
    for set in sets {
        let alive: Vec<usize> = set
            .occupied_groups()
            .iter()
            .filter_map(|group| groups.iter().position(|item| item == group))
            .collect();
        if alive.len() < 2 {
            continue;
        }
        let Some(representative) = alive.iter().copied().min() else {
            continue;
        };
        let focus = set
            .focused_group()
            .filter(|group| groups.iter().any(|item| item == group))
            .unwrap_or(groups[representative]);
        for index in &alive {
            hidden.insert(groups[*index]);
        }
        representatives.insert(representative, (set.id, focus));
    }
    let mut entries = Vec::with_capacity(groups.len());
    for (index, group) in groups.iter().enumerate() {
        if let Some(&(id, focus)) = representatives.get(&index) {
            entries.push(StripEntry::Tiled { id, focus });
        } else if hidden.contains(group) {
            continue;
        } else {
            entries.push(StripEntry::Single(*group));
        }
    }
    entries
}
