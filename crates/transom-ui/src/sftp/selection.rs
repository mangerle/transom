//! 文件栏多选状态机：主项、集合与锚点的联动规则。
//!
//! 设计原理（Why）：本地栏与远端栏此前各写一份几乎相同的选择算法
//! （点选、Ctrl 切换、Shift 连选、刷新后校正），只有路径类型不同；
//! 抽成与 UI 无关的纯函数后，规则只维护一处，并可在不构造 GPUI 实体
//! 的前提下直接测试。

use std::collections::BTreeSet;

/// 单选：清空集合，主项与锚点都指向该路径。
pub(crate) fn select<T: Ord + Clone>(
    primary: &mut Option<T>,
    selection: &mut BTreeSet<T>,
    anchor: &mut Option<T>,
    path: T,
) {
    *primary = Some(path.clone());
    *anchor = Some(path.clone());
    selection.clear();
    selection.insert(path);
}

/// 只移动主项并加入集合，保留已有多选（右键菜单定位单项操作用）。
pub(crate) fn set_primary<T: Ord + Clone>(
    primary: &mut Option<T>,
    selection: &mut BTreeSet<T>,
    path: T,
) {
    *primary = Some(path.clone());
    selection.insert(path);
}

/// Ctrl 切换：移除时若主项正是它，则把主项退到集合末项。
pub(crate) fn toggle<T: Ord + Clone>(
    primary: &mut Option<T>,
    selection: &mut BTreeSet<T>,
    anchor: &mut Option<T>,
    path: T,
) {
    if selection.remove(&path) {
        if primary.as_ref() == Some(&path) {
            *primary = selection.iter().next_back().cloned();
        }
    } else {
        selection.insert(path.clone());
        *primary = Some(path.clone());
    }
    *anchor = Some(path);
}

/// Shift 连选：用新集合整体替换，并指定主项。
pub(crate) fn select_range<T: Ord + Clone>(
    primary: &mut Option<T>,
    selection: &mut BTreeSet<T>,
    paths: impl IntoIterator<Item = T>,
    path: T,
) {
    *selection = paths.into_iter().collect();
    selection.insert(path.clone());
    *primary = Some(path);
}

/// 清空主项、集合与锚点。
pub(crate) fn clear<T>(
    primary: &mut Option<T>,
    selection: &mut BTreeSet<T>,
    anchor: &mut Option<T>,
) {
    *primary = None;
    selection.clear();
    *anchor = None;
}

/// 列表刷新后剔除已消失的路径，并修正主项与锚点。
pub(crate) fn sync<T: Ord + Clone>(
    primary: &mut Option<T>,
    selection: &mut BTreeSet<T>,
    anchor: &mut Option<T>,
    valid: &BTreeSet<T>,
) {
    selection.retain(|path| valid.contains(path));
    if primary.as_ref().is_some_and(|path| !valid.contains(path)) {
        *primary = selection.iter().next_back().cloned();
    }
    if anchor.as_ref().is_some_and(|path| !valid.contains(path)) {
        *anchor = primary.clone();
    }
    if selection.is_empty() {
        *primary = None;
        *anchor = None;
    }
}

/// 批量操作的目标路径：无多选时退化为单选主项。
pub(crate) fn paths<T: Clone>(primary: &Option<T>, selection: &BTreeSet<T>) -> Vec<T> {
    if selection.is_empty() {
        primary.iter().cloned().collect()
    } else {
        selection.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{clear, paths, select, select_range, set_primary, sync, toggle};
    use std::collections::BTreeSet;

    fn state() -> (Option<String>, BTreeSet<String>, Option<String>) {
        (None, BTreeSet::new(), None)
    }

    #[test]
    fn select_replaces_set_and_moves_anchor() {
        let (mut primary, mut selection, mut anchor) = state();
        select(&mut primary, &mut selection, &mut anchor, "a".into());
        select(&mut primary, &mut selection, &mut anchor, "b".into());
        assert_eq!(primary.as_deref(), Some("b"));
        assert_eq!(anchor.as_deref(), Some("b"));
        assert_eq!(selection, BTreeSet::from(["b".to_string()]));
    }

    #[test]
    fn toggle_adds_and_removes_with_primary_fallback() {
        let (mut primary, mut selection, mut anchor) = state();
        toggle(&mut primary, &mut selection, &mut anchor, "a".into());
        toggle(&mut primary, &mut selection, &mut anchor, "b".into());
        assert_eq!(primary.as_deref(), Some("b"));
        assert_eq!(selection.len(), 2);

        toggle(&mut primary, &mut selection, &mut anchor, "b".into());
        assert_eq!(primary.as_deref(), Some("a"), "移除主项后应退到集合末项");
        assert_eq!(selection, BTreeSet::from(["a".to_string()]));
        assert_eq!(anchor.as_deref(), Some("b"), "锚点始终跟随最近点击");
    }

    #[test]
    fn set_primary_keeps_existing_selection() {
        let (mut primary, mut selection, _) = state();
        select_range(
            &mut primary,
            &mut selection,
            ["a".to_string(), "b".to_string()],
            "b".into(),
        );
        set_primary(&mut primary, &mut selection, "c".into());
        assert_eq!(selection.len(), 3);
        assert_eq!(primary.as_deref(), Some("c"));
    }

    #[test]
    fn sync_drops_missing_paths_and_fixes_primary() {
        let (mut primary, mut selection, _) = state();
        select_range(
            &mut primary,
            &mut selection,
            ["a".to_string(), "b".to_string()],
            "b".into(),
        );
        // Shift 连选沿用既有锚点（首次点击时写入），这里模拟该前置状态。
        let mut anchor = Some("b".to_string());
        let valid = BTreeSet::from(["a".to_string()]);
        sync(&mut primary, &mut selection, &mut anchor, &valid);
        assert_eq!(selection, BTreeSet::from(["a".to_string()]));
        assert_eq!(primary.as_deref(), Some("a"));
        assert_eq!(anchor.as_deref(), Some("a"));

        sync(
            &mut primary,
            &mut selection,
            &mut anchor,
            &BTreeSet::from(["c".to_string()]),
        );
        assert!(selection.is_empty());
        assert_eq!(primary, None);
        assert_eq!(anchor, None);
    }

    #[test]
    fn paths_falls_back_to_primary() {
        let (mut primary, mut selection, mut anchor) = state();
        select(&mut primary, &mut selection, &mut anchor, "a".into());
        assert_eq!(paths(&primary, &selection), vec!["a".to_string()]);

        clear(&mut primary, &mut selection, &mut anchor);
        assert!(paths(&primary, &selection).is_empty());
    }
}
