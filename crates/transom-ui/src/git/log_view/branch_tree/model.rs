//! 引用栏分支树的纯数据模型与构建规则。
//!
//! 设计原理（Why）：树结构、排序、筛选与默认展开都只依赖 Git 引用数据，
//! 与 GPUI 渲染解耦后可以独立测试，也避免界面文件继续膨胀。

use super::super::current_head_name;
use std::collections::BTreeMap;
use transom_core::TagEntry;

type State = crate::git::GitWorkspaceState;

/// 引用栏的四个引用分组。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefGroup {
    Head,
    Local,
    Remote,
    Tags,
}

impl RefGroup {
    /// 稳定的展开态键，不随界面语言变化。
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Head => "HEAD",
            Self::Local => "git.refs.local",
            Self::Remote => "git.refs.remote",
            Self::Tags => "git.refs.tags",
        }
    }

    /// 分组标题。
    pub(super) fn title(self) -> String {
        match self {
            Self::Head => rust_i18n::t!("git.refs.head_current").to_string(),
            Self::Local => rust_i18n::t!("git.refs.local").to_string(),
            Self::Remote => rust_i18n::t!("git.refs.remote").to_string(),
            Self::Tags => rust_i18n::t!("git.refs.tags").to_string(),
        }
    }

    /// 未保存用户选择时的默认展开状态。
    pub(super) fn default_expanded(self) -> bool {
        matches!(self, Self::Head | Self::Local | Self::Remote)
    }
}

/// 树节点种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RefNodeKind {
    Folder,
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
}

/// 引用栏中的一个树节点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RefNode {
    pub(super) key: String,
    pub(super) label: String,
    pub(super) full_name: Option<String>,
    pub(super) kind: RefNodeKind,
    pub(super) current: bool,
    pub(super) favorite: bool,
    pub(super) default_expanded: bool,
    pub(super) children: Vec<RefNode>,
}

/// 前缀树构建过程中的分支项。
#[derive(Debug, Clone)]
struct PrefixItem {
    full_name: String,
    rest: String,
    current: bool,
    favorite: bool,
}

/// 按分组构建树节点。
pub(super) fn build_nodes(state: &State, group: RefGroup, query: &str) -> Vec<RefNode> {
    match group {
        RefGroup::Head => build_head_nodes(state, query),
        RefGroup::Local => build_local_nodes(state, query),
        RefGroup::Remote => build_remote_nodes(state, query),
        RefGroup::Tags => build_tag_nodes(state, query),
    }
}

/// HEAD 分组只展示当前分支；搜索时按 IDEA 口径隐藏该快捷入口。
fn build_head_nodes(state: &State, query: &str) -> Vec<RefNode> {
    if !query.is_empty() {
        return Vec::new();
    }
    current_head_name(state)
        .map(|name| {
            vec![RefNode {
                key: format!("head:{name}"),
                label: name.clone(),
                full_name: Some(name),
                kind: RefNodeKind::Head,
                current: true,
                favorite: false,
                default_expanded: false,
                children: Vec::new(),
            }]
        })
        .unwrap_or_default()
}

/// 本地分支按名称前缀构建目录树。
fn build_local_nodes(state: &State, query: &str) -> Vec<RefNode> {
    let items: Vec<PrefixItem> = state
        .branches
        .iter()
        .filter(|branch| !branch.remote && matches_query(&branch.name, query))
        .map(|branch| PrefixItem {
            full_name: branch.name.clone(),
            rest: branch.name.clone(),
            current: branch.current,
            favorite: false,
        })
        .collect();
    build_prefix_level(&items, "local", "", RefNodeKind::LocalBranch)
}

/// 远程分支先按远端名分组，再按分支路径构建目录树。
fn build_remote_nodes(state: &State, query: &str) -> Vec<RefNode> {
    let mut by_remote: BTreeMap<String, Vec<PrefixItem>> = BTreeMap::new();
    for branch in state
        .branches
        .iter()
        .filter(|branch| branch.remote && matches_query(&branch.name, query))
    {
        let Some((remote, rest)) = branch.name.split_once('/') else {
            continue;
        };
        if rest.is_empty() || rest == "HEAD" {
            continue;
        }
        by_remote
            .entry(remote.to_string())
            .or_default()
            .push(PrefixItem {
                full_name: branch.name.clone(),
                rest: rest.to_string(),
                current: false,
                favorite: matches!(rest, "main" | "master"),
            });
    }
    by_remote
        .into_iter()
        .map(|(remote, items)| RefNode {
            key: format!("remote-folder:{remote}"),
            label: remote.clone(),
            full_name: None,
            kind: RefNodeKind::Folder,
            current: false,
            favorite: false,
            default_expanded: true,
            children: build_prefix_level(
                &items,
                &format!("remote:{remote}"),
                "",
                RefNodeKind::RemoteBranch,
            ),
        })
        .collect()
}

/// 标签按名称倒序排列。
fn build_tag_nodes(state: &State, query: &str) -> Vec<RefNode> {
    let mut tags: Vec<&TagEntry> = state
        .tags
        .iter()
        .filter(|tag| matches_query(&tag.name, query))
        .collect();
    tags.sort_by(|left, right| right.name.cmp(&left.name));
    tags.into_iter()
        .map(|tag| RefNode {
            key: format!("tag:{}", tag.name),
            label: tag.name.clone(),
            full_name: Some(tag.name.clone()),
            kind: RefNodeKind::Tag,
            current: false,
            favorite: false,
            default_expanded: false,
            children: Vec::new(),
        })
        .collect()
}

/// 构建一层前缀树：固定项、文件夹、普通项依次排列。
fn build_prefix_level(
    items: &[PrefixItem],
    key_prefix: &str,
    path: &str,
    leaf_kind: RefNodeKind,
) -> Vec<RefNode> {
    let mut folders: BTreeMap<String, Vec<PrefixItem>> = BTreeMap::new();
    let mut leaves = Vec::new();
    for item in items {
        if let Some((segment, rest)) = item.rest.split_once('/')
            && !segment.is_empty()
            && !rest.is_empty()
        {
            folders
                .entry(segment.to_string())
                .or_default()
                .push(PrefixItem {
                    rest: rest.to_string(),
                    ..item.clone()
                });
            continue;
        }
        leaves.push(RefNode {
            key: format!("{key_prefix}:branch:{}", item.full_name),
            label: item.rest.clone(),
            full_name: Some(item.full_name.clone()),
            kind: leaf_kind,
            current: item.current,
            favorite: item.favorite,
            default_expanded: false,
            children: Vec::new(),
        });
    }
    leaves.sort_by(|left, right| {
        node_rank(left)
            .cmp(&node_rank(right))
            .then_with(|| left.label.cmp(&right.label))
    });
    let mut nodes: Vec<RefNode> = leaves
        .iter()
        .filter(|node| node_rank(node) < 2)
        .cloned()
        .collect();
    for (segment, inner) in folders {
        let folder_path = if path.is_empty() {
            segment.clone()
        } else {
            format!("{path}/{segment}")
        };
        nodes.push(RefNode {
            key: format!("{key_prefix}:folder:{folder_path}"),
            label: segment,
            full_name: None,
            kind: RefNodeKind::Folder,
            current: false,
            favorite: false,
            default_expanded: inner.iter().any(|item| item.current),
            children: build_prefix_level(
                &inner,
                &format!("{key_prefix}:folder:{folder_path}"),
                &folder_path,
                leaf_kind,
            ),
        });
    }
    nodes.extend(leaves.into_iter().filter(|node| node_rank(node) >= 2));
    nodes
}

/// 当前分支优先，其次远端默认分支，其余按名称。
fn node_rank(node: &RefNode) -> u8 {
    if node.current {
        0
    } else if node.favorite {
        1
    } else {
        2
    }
}

/// 名称是否符合当前搜索词。
fn matches_query(name: &str, query: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(query)
}

#[cfg(test)]
mod tests {
    use super::{PrefixItem, RefNodeKind, build_prefix_level};

    fn item(name: &str, current: bool) -> PrefixItem {
        PrefixItem {
            full_name: name.to_string(),
            rest: name.to_string(),
            current,
            favorite: false,
        }
    }

    /// 本地分支中的斜杠必须形成目录，而不是被当成远程分组。
    #[test]
    fn local_slash_branch_is_grouped_into_folder() {
        let items = vec![item("feat/git-panel", false), item("main", true)];
        let nodes = build_prefix_level(&items, "local", "", RefNodeKind::LocalBranch);
        assert_eq!(nodes[0].label, "main");
        assert_eq!(nodes[1].kind, RefNodeKind::Folder);
        assert_eq!(nodes[1].label, "feat");
        assert_eq!(nodes[1].children[0].label, "git-panel");
    }

    /// 当前分支位于目录中时，该目录默认展开。
    #[test]
    fn folder_containing_current_branch_opens_by_default() {
        let items = vec![item("feat/git-panel", true)];
        let nodes = build_prefix_level(&items, "local", "", RefNodeKind::LocalBranch);
        assert!(nodes[0].default_expanded);
    }
}
