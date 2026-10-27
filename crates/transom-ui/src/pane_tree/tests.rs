//! 分割树单元测试。

use super::types::{LayoutNode, MAX_PANES, PaneDirection, RemoveOutcome};
use transom_core::{SavedLayout, SplitAxis, TabId};

fn tab() -> TabId {
    TabId::new()
}

/// 生成 `count` 个窗格、沿同一方向链式切分的形状：`X(X(...(p, p), p), p)`。
fn chain_saved(count: usize, axis: SplitAxis) -> SavedLayout {
    let mut shape = SavedLayout::Pane;
    for _ in 1..count {
        shape = SavedLayout::Split {
            axis,
            first: Box::new(shape),
            second: Box::new(SavedLayout::Pane),
        };
    }
    shape
}

/// 生成 `count` 个窗格、沿同一方向链式切分的树。
fn chain(count: usize, axis: SplitAxis) -> (LayoutNode, Vec<TabId>) {
    let ids: Vec<TabId> = (0..count).map(|_| tab()).collect();
    let shape = chain_saved(count, axis);
    let mut cursor = ids.iter().copied();
    (LayoutNode::from_saved(&shape, &mut cursor).unwrap(), ids)
}

/// 构造 `H(a, V(b, c))`：a 在左，b 在右上，c 在右下。
fn nested() -> (LayoutNode, [TabId; 3]) {
    let [a, b, c] = [tab(), tab(), tab()];
    let mut root = LayoutNode::pane(a);
    assert!(root.split_leaf(a, SplitAxis::Horizontal, b));
    assert!(root.split_leaf(b, SplitAxis::Vertical, c));
    (root, [a, b, c])
}

#[test]
fn single_pane_split_places_new_pane_after_the_focused_one() {
    let (focused, fresh) = (tab(), tab());
    let mut root = LayoutNode::pane(focused);
    assert!(root.split_leaf(focused, SplitAxis::Vertical, fresh));
    assert_eq!(root.pane_ids(), vec![focused, fresh]);
    assert_eq!(root.root_axis(), SplitAxis::Vertical);
    assert_eq!(root.last_pane(), Some(fresh));
}

#[test]
fn split_nests_instead_of_rebalancing_siblings() {
    let (left, right, fresh) = (tab(), tab(), tab());
    let mut root = LayoutNode::pane(left);
    root.split_leaf(left, SplitAxis::Horizontal, right);
    // 焦点仍在左窗格：新窗格应紧贴左窗格，右窗格依旧是根节点的后侧子树。
    assert!(root.split_leaf(left, SplitAxis::Horizontal, fresh));
    assert_eq!(root.pane_ids(), vec![left, fresh, right]);
    match &root {
        LayoutNode::Split {
            axis,
            first,
            second,
            ..
        } => {
            assert_eq!(*axis, SplitAxis::Horizontal);
            assert_eq!(second.pane_ids(), vec![right]);
            assert_eq!(first.root_axis(), SplitAxis::Horizontal);
        }
        _ => panic!("根节点应保持为分割节点"),
    }
}

#[test]
fn split_rejects_unknown_pane_and_overflow() {
    let (mut root, ids) = chain(3, SplitAxis::Horizontal);
    assert!(!root.split_leaf(tab(), SplitAxis::Vertical, tab()));
    assert_eq!(root.pane_ids(), ids);
    let (mut full, _) = chain(MAX_PANES, SplitAxis::Horizontal);
    let focus = full.first_pane().unwrap();
    assert!(!full.split_leaf(focus, SplitAxis::Vertical, tab()));
    assert_eq!(full.pane_count(), MAX_PANES);
}

#[test]
fn remove_collapses_parent_and_focuses_adjacent_pane() {
    let (a, b, c) = (tab(), tab(), tab());
    let mut root = LayoutNode::pane(a);
    root.split_leaf(a, SplitAxis::Horizontal, b);
    root.split_leaf(b, SplitAxis::Horizontal, c);
    // 结构为 H(a, H(b, c))：摘除前侧 a，焦点落到兄弟子树最靠前的 b。
    assert_eq!(
        root.remove_leaf(a),
        RemoveOutcome::Removed { next_focus: b }
    );
    assert_eq!(root.root_axis(), SplitAxis::Horizontal);
    // 摘除后侧 c，焦点回到前侧子树最靠后的 b。
    assert_eq!(
        root.remove_leaf(c),
        RemoveOutcome::Removed { next_focus: b }
    );
    assert_eq!(root, LayoutNode::pane(b));
}

#[test]
fn remove_keeps_surviving_split_identity() {
    // 收起父节点后， surviving 子树必须沿用原 SplitId，否则其可拖拽尺寸会重置。
    let (mut root, ids) = chain(3, SplitAxis::Horizontal);
    let inner_id = match &root {
        LayoutNode::Split { first, .. } => match **first {
            LayoutNode::Split { id, .. } => id,
            _ => panic!("链式布局的左孩子应为分割节点"),
        },
        _ => panic!("三窗格链式布局根节点应为分割节点"),
    };
    root.remove_leaf(ids[2]);
    match &root {
        LayoutNode::Split { id, .. } => assert_eq!(*id, inner_id),
        _ => panic!("剩余两窗格仍应有分割节点"),
    }
}

#[test]
fn remove_last_pane_empties_the_group() {
    let only = tab();
    let mut root = LayoutNode::pane(only);
    assert_eq!(root.remove_leaf(only), RemoveOutcome::Empty);
}

#[test]
fn toggle_axis_flips_only_the_pane_own_cut() {
    let (mut root, [a, b, c]) = nested();
    // H(a, V(b, c))：翻转 a 只动根节点，内层上下分屏保持原样。
    assert!(root.toggle_axis(a));
    match &root {
        LayoutNode::Split { axis, second, .. } => {
            assert_eq!(*axis, SplitAxis::Vertical);
            assert_eq!(second.root_axis(), SplitAxis::Vertical);
        }
        _ => panic!("根节点应保持为分割节点"),
    }
    // 翻转 b 只动它所在的内层切分。
    assert!(root.toggle_axis(b));
    match &root {
        LayoutNode::Split { axis, second, .. } => {
            assert_eq!(*axis, SplitAxis::Vertical);
            assert_eq!(second.root_axis(), SplitAxis::Horizontal);
        }
        _ => panic!("根节点应保持为分割节点"),
    }
    assert_eq!(root.pane_ids(), vec![a, b, c]);
    assert!(!root.toggle_axis(tab()));
}

#[test]
fn focus_neighbor_walks_nested_splits_in_the_requested_direction() {
    let (root, [a, b, c]) = nested();
    assert_eq!(root.focus_neighbor(a, PaneDirection::Right), Some(b));
    assert_eq!(root.focus_neighbor(b, PaneDirection::Left), Some(a));
    assert_eq!(root.focus_neighbor(b, PaneDirection::Down), Some(c));
    assert_eq!(root.focus_neighbor(c, PaneDirection::Up), Some(b));
    // a 占满整个左侧高度：它的切分是水平的，外层再无上下切分，向下无处可去。
    assert_eq!(root.focus_neighbor(a, PaneDirection::Down), None);
    assert_eq!(root.focus_neighbor(c, PaneDirection::Down), None);
    // c 向右：内层是上下切分，外层已到边（c 在后侧），无窗格可去。
    assert_eq!(root.focus_neighbor(c, PaneDirection::Right), None);
    assert_eq!(root.focus_neighbor(a, PaneDirection::Left), None);
    assert_eq!(root.focus_neighbor(tab(), PaneDirection::Right), None);
}

#[test]
fn focus_neighbor_across_three_column_row_returns_adjacent_column() {
    let (root, ids) = chain(3, SplitAxis::Horizontal);
    // 链式 H(H(a, b), c)：从 a 向右应命中 b，而不是越过 b 直达 c。
    assert_eq!(
        root.focus_neighbor(ids[0], PaneDirection::Right),
        Some(ids[1])
    );
    assert_eq!(
        root.focus_neighbor(ids[1], PaneDirection::Right),
        Some(ids[2])
    );
    assert_eq!(
        root.focus_neighbor(ids[2], PaneDirection::Left),
        Some(ids[1])
    );
    assert_eq!(root.focus_neighbor(ids[2], PaneDirection::Right), None);
}

#[test]
fn focus_neighbor_needs_at_least_two_panes() {
    let only = tab();
    let root = LayoutNode::pane(only);
    assert_eq!(root.focus_neighbor(only, PaneDirection::Right), None);
}

#[test]
fn saved_round_trip_preserves_nesting_and_order() {
    let (root, ids) = nested_chain();
    let shape = root.to_saved();
    assert_eq!(LayoutNode::saved_pane_count(&shape), Some(3));
    let mut cursor = ids.iter().copied();
    let rebuilt = LayoutNode::from_saved(&shape, &mut cursor).unwrap();
    assert_eq!(rebuilt.pane_ids(), ids);
    assert_eq!(rebuilt.root_axis(), SplitAxis::Horizontal);
}

#[test]
fn from_saved_rejects_bad_shapes() {
    // 叶子数超过上限：损坏或恶意设置不得一次拉起过多连接。
    let huge = chain_saved(MAX_PANES + 1, SplitAxis::Horizontal);
    let mut ids = (0..MAX_PANES + 2).map(|_| tab());
    assert_eq!(LayoutNode::from_saved(&huge, &mut ids), None);

    // 可用窗格标识不足时整棵拒绝，而不是留下半棵树。
    let shape = chain_saved(3, SplitAxis::Horizontal);
    let mut one = std::iter::once(tab());
    assert_eq!(LayoutNode::from_saved(&shape, &mut one), None);
}

#[test]
fn chain_shape_counts_leaves_and_keeps_axis() {
    let single = chain_saved(1, SplitAxis::Horizontal);
    assert_eq!(single, SavedLayout::Pane);
    assert_eq!(LayoutNode::saved_pane_count(&single), Some(1));
    let vertical = chain_saved(3, SplitAxis::Vertical);
    assert_eq!(LayoutNode::saved_pane_count(&vertical), Some(3));
    let mut cursor = (0..3).map(|_| tab());
    let root = LayoutNode::from_saved(&vertical, &mut cursor).unwrap();
    assert_eq!(root.root_axis(), SplitAxis::Vertical);
    assert_eq!(root.pane_count(), 3);
}

/// 与 `nested()` 同形但走持久化通道构造，用于验证形状往返。
fn nested_chain() -> (LayoutNode, Vec<TabId>) {
    let ids = vec![tab(), tab(), tab()];
    let shape = SavedLayout::Split {
        axis: SplitAxis::Horizontal,
        first: Box::new(SavedLayout::Pane),
        second: Box::new(SavedLayout::Split {
            axis: SplitAxis::Vertical,
            first: Box::new(SavedLayout::Pane),
            second: Box::new(SavedLayout::Pane),
        }),
    };
    let mut cursor = ids.iter().copied();
    (LayoutNode::from_saved(&shape, &mut cursor).unwrap(), ids)
}

#[test]
fn contains_matches_only_real_panes() {
    let (root, ids) = chain(3, SplitAxis::Vertical);
    assert!(root.contains(ids[1]));
    assert!(!root.contains(tab()));
}
