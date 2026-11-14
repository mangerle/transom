//! 提交面板按目录分组的多级压缩目录树构建与勾选三态计算。

use super::CheckState;
use std::collections::{BTreeMap, HashSet};
use transom_core::StatusEntry;

/// 提交文件树节点（目录支持单子目录压缩，如 `crates/transom-ui/src`）。
pub(crate) enum DirTreeNode<'a> {
    Directory {
        name: String,
        dir_path: String,
        all_paths: Vec<String>,
        children: Vec<DirTreeNode<'a>>,
    },
    File(&'a StatusEntry),
}

#[derive(Default)]
struct TempDirNode<'a> {
    subdirs: BTreeMap<String, TempDirNode<'a>>,
    files: Vec<&'a StatusEntry>,
}

impl<'a> TempDirNode<'a> {
    fn insert(&mut self, parts: &[&str], entry: &'a StatusEntry) {
        if parts.len() <= 1 {
            self.files.push(entry);
        } else {
            self.subdirs
                .entry(parts[0].to_string())
                .or_default()
                .insert(&parts[1..], entry);
        }
    }

    fn collect_paths(&self, out: &mut Vec<String>) {
        for sub in self.subdirs.values() {
            sub.collect_paths(out);
        }
        for entry in &self.files {
            out.push(entry.path.clone());
        }
    }

    fn compress_single_child(self, current_name: String) -> (String, TempDirNode<'a>) {
        let TempDirNode { files, subdirs } = self;
        if files.is_empty() && subdirs.len() == 1 {
            let (child_name, child_node) = subdirs.into_iter().next().expect("单子目录必存在");
            return child_node.compress_single_child(format!("{current_name}/{child_name}"));
        }
        (current_name, TempDirNode { subdirs, files })
    }

    fn into_tree_nodes(mut self, parent_path: &str) -> Vec<DirTreeNode<'a>> {
        let mut nodes = Vec::new();
        for (name, sub) in self.subdirs {
            let (compressed_name, final_sub) = sub.compress_single_child(name);
            let dir_path = if parent_path.is_empty() {
                compressed_name.clone()
            } else {
                format!("{parent_path}/{compressed_name}")
            };
            let mut all_paths = Vec::new();
            final_sub.collect_paths(&mut all_paths);
            let children = final_sub.into_tree_nodes(&dir_path);
            nodes.push(DirTreeNode::Directory {
                name: compressed_name,
                dir_path,
                all_paths,
                children,
            });
        }
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        for entry in self.files {
            nodes.push(DirTreeNode::File(entry));
        }
        nodes
    }
}

/// 将扁平状态条目构建为多级压缩目录树。
pub(crate) fn build_dir_tree(entries: &[StatusEntry]) -> Vec<DirTreeNode<'_>> {
    let mut root = TempDirNode::default();
    for entry in entries {
        let parts: Vec<&str> = entry.path.split('/').collect();
        root.insert(&parts, entry);
    }
    root.into_tree_nodes("")
}

/// 递归收集所有目录节点的折叠键（用于“全部收起”）。
pub(crate) fn collect_dir_keys(
    group_key: &str,
    nodes: &[DirTreeNode<'_>],
    out: &mut HashSet<String>,
) {
    for node in nodes {
        if let DirTreeNode::Directory {
            dir_path, children, ..
        } = node
        {
            out.insert(format!("{group_key}|{dir_path}"));
            collect_dir_keys(group_key, children, out);
        }
    }
}

/// 计算路径集合的勾选三态（未选 / 部分选中 / 全选）。
pub(crate) fn compute_check_state(paths: &[String], checked: &HashSet<String>) -> CheckState {
    if paths.is_empty() {
        return CheckState::Unchecked;
    }
    let checked_count = paths.iter().filter(|path| checked.contains(*path)).count();
    if checked_count == 0 {
        CheckState::Unchecked
    } else if checked_count == paths.len() {
        CheckState::Checked
    } else {
        CheckState::Indeterminate
    }
}

#[cfg(test)]
mod tests {
    use super::{DirTreeNode, build_dir_tree, compute_check_state};
    use crate::git::commit_modal::CheckState;
    use std::collections::HashSet;
    use transom_core::{ChangeCode, StatusEntry, StatusRecord};

    fn dummy_entry(path: &str) -> StatusEntry {
        StatusEntry {
            path: path.to_string(),
            old_path: None,
            record: StatusRecord::Tracked {
                index: ChangeCode::Unmodified,
                worktree: ChangeCode::Modified,
            },
        }
    }

    #[test]
    fn compresses_single_child_directories_and_sorts() {
        let entries = vec![
            dummy_entry("crates/transom-ui/src/git/commit_page.rs"),
            dummy_entry("crates/transom-ui/src/diff_pane/controls.rs"),
            dummy_entry("crates/transom-ui-icons/src/ui_icons.rs"),
            dummy_entry("Cargo.toml"),
        ];
        let tree = build_dir_tree(&entries);
        assert_eq!(tree.len(), 2);
        match &tree[0] {
            DirTreeNode::Directory { name, children, .. } => {
                assert_eq!(name, "crates");
                assert_eq!(children.len(), 2);
                match &children[0] {
                    DirTreeNode::Directory { name, .. } => assert_eq!(name, "transom-ui/src"),
                    DirTreeNode::File(_) => panic!("expected compressed directory"),
                }
            }
            DirTreeNode::File(_) => panic!("expected directory"),
        }
        match &tree[1] {
            DirTreeNode::File(entry) => assert_eq!(entry.path, "Cargo.toml"),
            DirTreeNode::Directory { .. } => panic!("expected root file"),
        }
    }

    #[test]
    fn computes_tri_state_selection() {
        let paths = vec!["a.rs".to_string(), "b.rs".to_string()];
        let mut checked = HashSet::new();
        assert_eq!(compute_check_state(&paths, &checked), CheckState::Unchecked);
        checked.insert("a.rs".to_string());
        assert_eq!(
            compute_check_state(&paths, &checked),
            CheckState::Indeterminate
        );
        checked.insert("b.rs".to_string());
        assert_eq!(compute_check_state(&paths, &checked), CheckState::Checked);
    }
}
