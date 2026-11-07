//! Git 变更文件层级目录树（对齐 IntelliJ IDEA 变更树布局与视觉呈现）。
//!
//! 设计原理（Why）：
//! - 将冗长的文件路径按目录结构聚合成层级树，公共前缀自动压缩合并（如 `crates/transom-ui/src 9 个文件`）；
//! - 仅展示纯文件名，彻底解决长路径换行拥挤杂乱的问题，使修改范围一目了然；
//! - 状态图标采用 IDEA 同款细致微型徽标（修改蓝圈、新增绿加、删除红减），文字按状态语义着色；
//! - 支持单击选中、双击呼出独立全屏差异比较。

use crate::app::AppShell;
use crate::git::{ChangeAccent, change_visual};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::collections::BTreeMap;
use transom_core::{ChangeCode, CommitFile, GitProject, TabId};

use super::menus::render_file_tree_menu;

/// 单个树节点行高（紧凑 IDE 规范）。
const TREE_ROW_HEIGHT: f32 = 22.0;

/// 变更文件树节点定义。
pub(crate) enum FileTreeNode {
    /// 目录节点（含压缩公共路径与包含的文件总数）。
    Directory {
        name: String,
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
        root_dir.into_tree_nodes()
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

    fn into_tree_nodes(self) -> Vec<FileTreeNode> {
        let mut result = Vec::new();

        for (name, sub) in self.subdirs {
            let (compressed_name, final_sub) = sub.compress_single_child(name);
            let count = final_sub.count_files();
            let children = final_sub.into_tree_nodes();
            result.push(FileTreeNode::Directory {
                name: compressed_name,
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

/// 递归渲染变更树。
pub(crate) fn render_tree_nodes(
    nodes: &[FileTreeNode],
    depth: usize,
    active_path: Option<&str>,
    app: &Entity<AppShell>,
    project: &GitProject,
    tab_id: TabId,
    p: &Palette,
) -> Vec<AnyElement> {
    let mut elements = Vec::new();
    for node in nodes {
        match node {
            FileTreeNode::Directory {
                name,
                file_count,
                children,
            } => {
                let indent = depth as f32 * 14.0 + 4.0;
                let dir_row =
                    h_flex()
                        .w_full()
                        .h(px(TREE_ROW_HEIGHT))
                        .items_center()
                        .pl(px(indent))
                        .pr_2()
                        .gap_1()
                        .text_xs()
                        .child(
                            Icon::new(IconName::ChevronDown)
                                .xsmall()
                                .text_color(p.muted_foreground),
                        )
                        .child(
                            Icon::new(IconName::Folder)
                                .xsmall()
                                .text_color(p.muted_foreground),
                        )
                        .child(
                            div()
                                .text_color(p.foreground)
                                .font_weight(FontWeight::MEDIUM)
                                .child(name.clone()),
                        )
                        .child(div().text_color(p.muted_foreground.opacity(0.6)).child(
                            rust_i18n::t!("git.files.count", count = *file_count).to_string(),
                        ));
                elements.push(dir_row.into_any_element());
                // 默认展开子节点
                let child_elements =
                    render_tree_nodes(children, depth + 1, active_path, app, project, tab_id, p);
                elements.extend(child_elements);
            }
            FileTreeNode::File {
                name,
                full_path,
                change,
                old_path,
            } => {
                let indent = depth as f32 * 14.0 + 4.0;
                let is_selected = active_path == Some(full_path.as_str());
                let (status_icon, status_color) = change_status_badge(*change, p);

                let click_app = app.clone();
                let dbl_click_app = app.clone();
                let menu_app = app.clone();
                let menu_project = project.clone();
                let path_clone = full_path.clone();
                let dbl_path = full_path.clone();
                let menu_path = full_path.clone();

                let file_row = h_flex()
                    .id(ElementId::Name(format!("git-tree-file-{full_path}").into()))
                    .w_full()
                    .h(px(TREE_ROW_HEIGHT))
                    .items_center()
                    .pl(px(indent))
                    .pr_2()
                    .gap_1p5()
                    .cursor_pointer()
                    .rounded(px(2.0))
                    .when(is_selected, |style| style.bg(p.list_active))
                    .when(!is_selected, |style| {
                        style.hover(|hover| hover.bg(p.list_hover))
                    })
                    .child(Icon::new(status_icon).xsmall().text_color(status_color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .truncate()
                            .text_xs()
                            .font_weight(if is_selected {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if is_selected {
                                p.foreground
                            } else {
                                status_color
                            })
                            .child(name.clone()),
                    )
                    .when_some(old_path.as_ref(), |el, old| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground.opacity(0.6))
                                .child(format!("← {old}")),
                        )
                    })
                    .on_click(move |event, _, cx| {
                        if event.click_count() >= 2 {
                            dbl_click_app.update(cx, |this, cx| {
                                this.git_open_diff_modal(tab_id, &dbl_path, cx);
                            });
                        } else {
                            click_app.update(cx, |this, cx| {
                                this.git_diff_commit_file(tab_id, &path_clone, cx);
                            });
                        }
                    })
                    .context_menu(move |menu, _window, _cx| {
                        render_file_tree_menu(
                            menu,
                            menu_app.clone(),
                            menu_project.clone(),
                            tab_id,
                            menu_path.clone(),
                        )
                    });

                elements.push(file_row.into_any_element());
            }
        }
    }
    elements
}

/// 匹配 IDEA 风格的状态徽标与语义颜色。
fn change_status_badge(change: ChangeCode, p: &Palette) -> (IconName, Hsla) {
    let visual = change_visual(change);
    let color = match visual.accent {
        ChangeAccent::Added => p.success,
        ChangeAccent::Modified => p.primary,
        ChangeAccent::Deleted => p.danger,
        ChangeAccent::Conflict => p.warning,
        ChangeAccent::Neutral => p.muted_foreground,
    };
    (visual.icon, color)
}
