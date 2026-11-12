//! Git 变更文件层级目录树（对齐 IntelliJ IDEA 变更树布局与视觉呈现）。
//!
//! 设计原理（Why）：
//! - 将冗长的文件路径按目录结构聚合成层级树，公共前缀自动压缩合并（如 `crates/transom-ui/src 9 个文件`）；
//! - 仅展示纯文件名，彻底解决长路径换行拥挤杂乱的问题，使修改范围一目了然；
//! - 状态图标与文字统一采用 `git::change_accent_color` 语义色（修改蓝、新增绿、删除灰、冲突红），对齐 IDEA；
//! - 目录可折叠；单击与双击文件都只更新右栏差异，不额外弹出对比框。

use crate::app::AppShell;
use crate::git::{change_accent_color, change_visual};
use crate::palette::Palette;
use crate::theme::tokens::{control, size};
use gpui_kit::base::h_flex;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::collections::{BTreeMap, HashSet};
use transom_core::{ChangeCode, CommitFile, GitProject, TabId};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::menus::render_file_tree_menu;

/// 单个树节点行高：通过 Rems 令牌随 UI 密度缩放，与 14px 文件名保持足够的垂直呼吸空间。
const TREE_ROW_HEIGHT: Rems = rems(size::COMPACT_TREE_ROW_HEIGHT);

/// 变更文件树节点定义。
pub(crate) enum FileTreeNode {
    /// 目录节点（含压缩公共路径与包含的文件总数）。
    Directory {
        name: String,
        path: String,
        file_count: usize,
        children: Vec<FileTreeNode>,
    },
    /// 具体文件节点。
    File {
        name: String,
        full_path: String,
        change: ChangeCode,
        old_path: Option<String>,
    },
}

impl FileTreeNode {
    /// 从文件列表构建多层级目录树，并自动压缩单子目录。
    pub(crate) fn build_tree(files: &[CommitFile]) -> Vec<FileTreeNode> {
        let mut root_dir = TempDirNode::default();
        for file in files {
            let parts: Vec<&str> = file.path.split('/').collect();
            root_dir.insert(&parts, file);
        }
        root_dir.into_tree_nodes("")
    }
}

/// 辅助树构建的内部中间节点。
#[derive(Default)]
struct TempDirNode {
    subdirs: BTreeMap<String, TempDirNode>,
    files: Vec<CommitFile>,
}

impl TempDirNode {
    fn insert(&mut self, parts: &[&str], file: &CommitFile) {
        if parts.len() <= 1 {
            self.files.push(file.clone());
        } else {
            let dir_name = parts[0].to_string();
            self.subdirs
                .entry(dir_name)
                .or_default()
                .insert(&parts[1..], file);
        }
    }

    fn count_files(&self) -> usize {
        let mut total = self.files.len();
        for sub in self.subdirs.values() {
            total += sub.count_files();
        }
        total
    }

    fn into_tree_nodes(self, parent_path: &str) -> Vec<FileTreeNode> {
        let mut result = Vec::new();

        for (name, sub) in self.subdirs {
            let (compressed_name, final_sub) = sub.compress_single_child(name);
            let path = join_tree_path(parent_path, &compressed_name);
            let count = final_sub.count_files();
            let children = final_sub.into_tree_nodes(&path);
            result.push(FileTreeNode::Directory {
                name: compressed_name,
                path,
                file_count: count,
                children,
            });
        }

        for file in self.files {
            let name = file
                .path
                .rsplit_once('/')
                .map(|(_, f)| f.to_string())
                .unwrap_or_else(|| file.path.clone());
            result.push(FileTreeNode::File {
                name,
                full_path: file.path.clone(),
                change: file.change,
                old_path: file.old_path.clone(),
            });
        }

        result
    }

    /// 递归压缩单路径目录（如 `crates` 下只有 `transom-ui` 则压缩为 `crates/transom-ui`）。
    fn compress_single_child(self, current_name: String) -> (String, TempDirNode) {
        let TempDirNode { files, mut subdirs } = self;
        if files.is_empty()
            && subdirs.len() == 1
            && let Some((sub_name, sub_node)) = subdirs.pop_first()
        {
            let merged_name = format!("{current_name}/{sub_name}");
            return sub_node.compress_single_child(merged_name);
        }
        (current_name, TempDirNode { files, subdirs })
    }
}

fn join_tree_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// 渲染变更树所需的只读参数，避免递归调用继续平铺七个参数。
#[derive(Clone, Copy)]
pub(crate) struct FileTreeRenderParams<'a> {
    pub nodes: &'a [FileTreeNode],
    pub depth: usize,
    pub active_path: Option<&'a str>,
    pub app: &'a Entity<AppShell>,
    pub project: &'a GitProject,
    pub tab_id: TabId,
    pub collapsed_dirs: &'a HashSet<String>,
    pub palette: &'a Palette,
}

/// 递归渲染变更树。
pub(crate) fn render_tree_nodes(params: FileTreeRenderParams<'_>) -> Vec<AnyElement> {
    let mut elements = Vec::new();
    for node in params.nodes {
        match node {
            FileTreeNode::Directory {
                name,
                path,
                file_count,
                children,
            } => {
                let expanded = !params.collapsed_dirs.contains(path);
                let indent = params.depth as f32 * 14.0 + 4.0;
                let toggle_app = params.app.clone();
                let toggle_key = path.clone();
                let dir_row = h_flex()
                    .id(ElementId::Name(format!("git-tree-dir-{path}").into()))
                    .w_full()
                    .h(TREE_ROW_HEIGHT)
                    .items_center()
                    .pl(px(indent))
                    .pr_2()
                    .gap_1()
                    .text_sm()
                    .cursor_pointer()
                    .rounded(px(2.0))
                    .hover(|style| style.bg(params.palette.list_hover))
                    .child(
                        AppIconElement::new(if expanded {
                            UiIcon::CHEVRON_DOWN
                        } else {
                            UiIcon::CHEVRON_RIGHT
                        })
                        .with_rems(rems(size::TREE_CHEVRON))
                        .text_color(params.palette.muted_foreground),
                    )
                    .child(AppIconElement::new(UiIcon::FOLDER).with_rems(rems(control::ICON)))
                    .child(
                        div()
                            .text_color(params.palette.foreground)
                            .font_weight(FontWeight::MEDIUM)
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .text_color(params.palette.muted_foreground.opacity(0.6))
                            .child(
                                rust_i18n::t!("git.files.count", count = *file_count).to_string(),
                            ),
                    )
                    .on_click(move |_, _, cx| {
                        toggle_app.update(cx, |this, cx| {
                            this.git_toggle_log_file_dir(params.tab_id, &toggle_key, cx);
                        });
                    });
                elements.push(dir_row.into_any_element());
                if expanded {
                    elements.extend(render_tree_nodes(FileTreeRenderParams {
                        nodes: children,
                        depth: params.depth + 1,
                        ..params
                    }));
                }
            }
            FileTreeNode::File {
                name,
                full_path,
                change,
                old_path,
            } => {
                let indent = params.depth as f32 * 14.0 + 4.0;
                let is_selected = params.active_path == Some(full_path.as_str());
                let status_color = change_color(*change, params.palette);

                let click_app = params.app.clone();
                let menu_app = params.app.clone();
                let menu_project = params.project.clone();
                let path_clone = full_path.clone();
                let menu_path = full_path.clone();

                let file_row = h_flex()
                    .id(ElementId::Name(format!("git-tree-file-{full_path}").into()))
                    .w_full()
                    .h(TREE_ROW_HEIGHT)
                    .items_center()
                    .pl(px(indent))
                    .pr_2()
                    .gap_1p5()
                    .cursor_pointer()
                    .rounded(px(2.0))
                    .when(is_selected, |style| style.bg(params.palette.list_active))
                    .when(!is_selected, |style| {
                        style.hover(|hover| hover.bg(params.palette.list_hover))
                    })
                    .child(
                        AppIconElement::new(AppIcon::for_file(full_path))
                            .with_rems(rems(control::ICON)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .truncate()
                            .text_sm()
                            .font_weight(if is_selected {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if is_selected {
                                params.palette.foreground
                            } else {
                                status_color
                            })
                            .child(name.clone()),
                    )
                    .when_some(old_path.as_ref(), |el, old| {
                        el.child(
                            div()
                                .text_sm()
                                .text_color(params.palette.muted_foreground.opacity(0.6))
                                .child(format!("← {old}")),
                        )
                    })
                    .on_click(move |event, _, cx| {
                        // 双击的第一次点击已经完成同一动作；忽略后续点击，避免重复发差异请求。
                        if event.click_count() > 1 {
                            return;
                        }
                        click_app.update(cx, |this, cx| {
                            this.git_diff_commit_file(params.tab_id, &path_clone, cx);
                        });
                    })
                    .context_menu(move |menu, _window, _cx| {
                        render_file_tree_menu(
                            menu,
                            menu_app.clone(),
                            menu_project.clone(),
                            params.tab_id,
                            menu_path.clone(),
                        )
                    });

                elements.push(file_row.into_any_element());
            }
        }
    }
    elements
}

/// 文件名的修改状态语义色；类型图标保持 JetBrains 原始配色。
fn change_color(change: ChangeCode, p: &Palette) -> Hsla {
    change_accent_color(change_visual(change).accent, p)
}

#[cfg(test)]
mod tests {
    use super::FileTreeNode;
    use transom_core::{ChangeCode, CommitFile};

    fn file(path: &str) -> CommitFile {
        CommitFile {
            path: path.to_string(),
            old_path: None,
            change: ChangeCode::Modified,
        }
    }

    #[test]
    fn directory_nodes_keep_full_paths_as_stable_collapse_keys() {
        let tree = FileTreeNode::build_tree(&[
            file("crates/transom-ui/src/main.rs"),
            file("crates/transom-ui/src/lib.rs"),
        ]);
        let FileTreeNode::Directory { path, .. } = &tree[0] else {
            panic!("预期首节点为目录");
        };
        assert_eq!(path, "crates/transom-ui/src");
    }
}
