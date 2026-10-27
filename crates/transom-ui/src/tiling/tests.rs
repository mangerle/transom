//! tiling 模块单元测试。

use super::layout::{best_grid, build_tiling_grid, expand_grid, tiling_layout_options};
use super::strip::build_strip_entries;
use super::tabs::TiledTabs;
use super::types::{MAX_TILING_SLOTS, StripEntry, TilingNode};
use crate::pane_tree::PaneDirection;
use transom_core::{GroupId, SplitAxis};

fn groups(count: usize) -> Vec<GroupId> {
    (0..count).map(|_| GroupId::new()).collect()
}

#[test]
fn grid_tree_covers_every_slot_and_split() {
    let tree = build_tiling_grid(2, 2);
    let mut slots = Vec::new();
    let mut stack = vec![&tree];
    while let Some(node) = stack.pop() {
        match node {
            TilingNode::Slot(slot) => slots.push(*slot),
            TilingNode::Stack { children, .. } => {
                stack.extend(children.iter());
            }
        }
    }
    slots.sort_unstable();
    assert_eq!(slots, vec![0, 1, 2, 3]);
    assert_eq!(tree.split_ids().len(), 3);
}

#[test]
fn tiled_tabs_bind_slots_and_remove_with_focus() {
    let ids = groups(4);
    let mut tiled = TiledTabs::new(ids.clone(), 2, 2, false).expect("合法 2x2");
    assert_eq!(tiled.occupied_count(), 4);
    assert_eq!(tiled.slot_of(ids[2]), Some(2));
    assert_eq!(tiled.focused_group(), Some(ids[0]));

    tiled.focused = 2;
    assert!(tiled.remove_group(ids[2]));
    assert_eq!(tiled.occupied_count(), 3);
    assert_eq!(tiled.focused_group(), Some(ids[3]));
}

#[test]
fn adjacent_slot_walks_grid_and_skips_empty_cells() {
    let ids = groups(4);
    let mut tiled = TiledTabs::new(ids.clone(), 2, 2, false).expect("合法 2x2");
    assert_eq!(tiled.adjacent_slot(0, PaneDirection::Right), Some(1));
    assert_eq!(tiled.adjacent_slot(0, PaneDirection::Down), Some(2));
    assert_eq!(tiled.adjacent_slot(3, PaneDirection::Left), Some(2));
    assert_eq!(tiled.adjacent_slot(3, PaneDirection::Up), Some(1));
    assert_eq!(tiled.adjacent_slot(1, PaneDirection::Right), None);

    tiled.slots[1] = None;
    assert_eq!(tiled.adjacent_slot(0, PaneDirection::Right), None);
}

#[test]
fn saved_slots_reject_duplicates_and_short_grids() {
    let ids = groups(3);
    assert!(
        TiledTabs::from_saved_slots(vec![Some(ids[0]), Some(ids[0]), None, None], 2, 2, 0, false)
            .is_none()
    );
    assert!(TiledTabs::from_saved_slots(vec![Some(ids[0]), None], 1, 2, 0, false).is_none());
    assert!(
        TiledTabs::from_saved_slots(vec![Some(ids[0]); MAX_TILING_SLOTS + 1], 5, 4, 0, false)
            .is_none()
    );
    let tiled =
        TiledTabs::from_saved_slots(vec![Some(ids[0]), None, Some(ids[1]), None], 2, 2, 1, false)
            .expect("两个有效槽");
    assert_eq!(tiled.focused_group(), Some(ids[0]));
}

#[test]
fn layout_options_cover_capacity_and_stay_within_limit() {
    let options = tiling_layout_options(3);
    assert!(options.iter().all(|(rows, cols)| rows * cols >= 3));
    assert!(
        options
            .iter()
            .all(|(rows, cols)| rows * cols <= MAX_TILING_SLOTS)
    );
    assert!(options.contains(&(2, 2)));
    assert!(options.contains(&(1, 3)));
    assert!(options.contains(&(4, 4)));
    assert!(!options.contains(&(1, 2)));
}

#[test]
fn auto_grid_prefers_exact_or_balanced_layouts() {
    assert_eq!(best_grid(2, 1.6), (1, 2));
    assert_eq!(best_grid(2, 0.6), (2, 1));
    assert_eq!(best_grid(4, 1.6), (2, 2));
    assert_eq!(best_grid(6, 1.6), (2, 3));
    assert_eq!(best_grid(9, 1.6), (3, 3));
    assert_eq!(best_grid(3, 1.6), (2, 2));
}

#[test]
fn expansion_keeps_dimensions_and_picks_balanced_grid() {
    assert_eq!(expand_grid(2, 2, 5, 1.6), Some((2, 3)));
    assert_eq!(expand_grid(2, 2, 9, 1.6), Some((3, 3)));
    assert_eq!(expand_grid(1, 4, 5, 1.6), Some((2, 4)));
    assert_eq!(expand_grid(4, 4, 17, 1.6), None);
}

#[test]
fn build_grid_keeps_slot_geometry_for_wide_and_tall_layouts() {
    match build_tiling_grid(1, 4) {
        TilingNode::Stack { axis, children, .. } => {
            assert_eq!(axis, SplitAxis::Horizontal);
            assert_eq!(children.len(), 4);
        }
        _ => panic!("1x4 应是一个水平容器"),
    }
    assert_eq!(build_tiling_grid(1, 3).split_ids().len(), 1);
    assert_eq!(build_tiling_grid(3, 1).split_ids().len(), 1);
    assert_eq!(build_tiling_grid(2, 2).split_ids().len(), 3);
    match build_tiling_grid(1, 2) {
        TilingNode::Stack { axis, .. } => assert_eq!(axis, SplitAxis::Horizontal),
        _ => panic!("1x2 应是水平容器"),
    }
}

#[test]
fn strip_entries_merge_each_set_into_one_item() {
    let ids = groups(4);
    let set = TiledTabs::new(vec![ids[0], ids[3]], 1, 2, false).expect("合法 1x2");
    let entries = build_strip_entries(&ids, std::slice::from_ref(&set));
    assert_eq!(entries.len(), 3);
    assert!(matches!(entries[0], StripEntry::Tiled { .. }));
    assert_eq!(entries[1], StripEntry::Single(ids[1]));
    assert_eq!(entries[2], StripEntry::Single(ids[2]));

    let other = TiledTabs::new(vec![ids[1], ids[2]], 1, 2, false).expect("合法 1x2");
    let entries = build_strip_entries(&ids, &[set, other]);
    assert_eq!(entries.len(), 2);
    assert!(matches!(entries[0], StripEntry::Tiled { .. }));
    assert!(matches!(entries[1], StripEntry::Tiled { .. }));
}

#[test]
fn strip_entries_ignore_broken_sets_and_keep_non_contiguous_members_hidden() {
    let ids = groups(4);
    let set = TiledTabs::new(vec![ids[0], ids[2]], 1, 2, false).expect("合法 1x2");
    let entries = build_strip_entries(&ids, &[set]);
    assert_eq!(entries.len(), 3);
    assert!(matches!(entries[0], StripEntry::Tiled { .. }));
    assert_eq!(entries[1], StripEntry::Single(ids[1]));
    assert_eq!(entries[2], StripEntry::Single(ids[3]));

    let broken = TiledTabs::new(vec![ids[0], ids[1]], 1, 2, false).expect("合法 1x2");
    let mut broken = broken;
    broken.remove_group(ids[1]);
    let entries = build_strip_entries(&ids, &[broken]);
    assert_eq!(entries.len(), 4);
    assert!(
        entries
            .iter()
            .all(|entry| matches!(entry, StripEntry::Single(_)))
    );
}
