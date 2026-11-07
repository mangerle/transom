//! 提交面板的改动树：三个分组、按目录折叠、分组头与目录行。
//!
//! 设计原理（Why）：分组渲染是提交面板里最重的一段（分组头 + 目录行 + 文件行三层
//! 嵌套），单独成模块后，改折叠或改分组顺序只碰这一处；它只依赖 `ChangeGroup` 与
//! 父模块的可视件（勾选框、状态字母、配色），不反向依赖对话框本体。

use super::group_menu::render_group_menu;
use super::{ChangeGroup, GroupDisplay, check_box, check_placeholder, hint};
use crate::app::AppShell;
use crate::git::actions::GitDiffRequest;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{ChangeCode, GitProject, GitSide, StatusEntry, StatusRecord};

impl AppShell {
    /// 三组改动清单（模态框与概览屏共用）。
    pub(crate) fn render_git_group_list(
        &mut self,
        project: &GitProject,
        display: &GroupDisplay,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let checked = &display.checked;
        let Some(status) = self
            .git_state_of(project.id)
            .and_then(|state| state.status.clone())
        else {
            return hint(rust_i18n::t!("git.overview.loading").to_string(), p);
        };
        let active_diff = self
            .git_state_of(project.id)
            .and_then(|state| state.last_diff.clone());
        let staged: Vec<StatusEntry> = status
            .entries
            .iter()
            .filter(|entry| match entry.record {
                StatusRecord::Tracked { index, .. } => index != ChangeCode::Unmodified,
                StatusRecord::Conflict => true,
                _ => false,
            })
            .cloned()
            .collect();
        let unstaged: Vec<StatusEntry> = status
            .entries
            .iter()
            .filter(|entry| {
                matches!(entry.record, StatusRecord::Tracked { worktree, .. } if worktree != ChangeCode::Unmodified)
            })
            .cloned()
            .collect();
        let untracked: Vec<StatusEntry> = status
            .entries
            .iter()
            .filter(|entry| matches!(entry.record, StatusRecord::Untracked))
            .cloned()
            .collect();
        // 忽略组只有打开了侧栏「显示忽略的文件」才会由 status 命令带回数据：
        // 引擎默认不加 --ignored，大仓库的忽略清单可能上万条。
        let ignored: Vec<StatusEntry> = status
            .entries
            .iter()
            .filter(|entry| matches!(entry.record, StatusRecord::Ignored))
            .cloned()
            .collect();

        let mut list = v_flex().gap_1().flex_1().min_h_0().overflow_y_scrollbar();
        for (group, entries) in [
            (ChangeGroup::Staged, &staged),
            (ChangeGroup::Unstaged, &unstaged),
            (ChangeGroup::Untracked, &untracked),
            (ChangeGroup::Ignored, &ignored),
        ] {
            if entries.is_empty() {
                continue;
            }
            let key = group.key();
            let group_collapsed = display.collapsed_groups.contains(key);
            let all_checked = entries.iter().all(|entry| checked.contains(&entry.path));
            list = list.child(self.git_group_header(
                project,
                &GroupView {
                    group,
                    entries,
                    all_checked,
                    collapsed: group_collapsed,
                },
                p,
                cx,
            ));
            if group_collapsed {
                continue;
            }
            if display.by_directory {
                list = list.children(self.git_dir_rows(project, group, entries, display, p, cx));
                continue;
            }
            for entry in entries.iter() {
                list = list.child(self.git_file_row(
                    &super::file_row::GitFileRowProps {
                        project,
                        entry,
                        group,
                        checked,
                        indent: 24.0,
                        selected: diff_matches(active_diff.as_ref(), &entry.path, group),
                    },
                    p,
                    cx,
                ));
            }
        }
        if staged.is_empty() && unstaged.is_empty() && untracked.is_empty() && ignored.is_empty() {
            list = list.child(hint(rust_i18n::t!("git.group.clean").to_string(), p));
        }
        list.into_any_element()
    }

    /// 按目录模式：每个目录一行（可折叠、可整目录勾选），后接该目录的文件行。
    fn git_dir_rows(
        &mut self,
        project: &GitProject,
        group: ChangeGroup,
        entries: &[StatusEntry],
        display: &GroupDisplay,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let active_diff = self
            .git_state_of(project.id)
            .and_then(|state| state.last_diff.clone());
        let mut buckets: Vec<(String, Vec<&StatusEntry>)> = Vec::new();
        for entry in entries {
            let dir = match entry.path.rfind('/') {
                Some(at) => entry.path[..at].to_string(),
                None => String::new(),
            };
            match buckets.iter_mut().find(|(name, _)| *name == dir) {
                Some((_, list)) => list.push(entry),
                None => buckets.push((dir, vec![entry])),
            }
        }
        buckets.sort_by(|left, right| left.0.cmp(&right.0));
        let mut rows = Vec::with_capacity(buckets.len());
        for (dir, items) in buckets {
            let key = format!("{}|{}", group.key(), dir);
            let paths: Vec<String> = items.iter().map(|entry| entry.path.clone()).collect();
            let all_checked = paths.iter().all(|path| display.checked.contains(path));
            let collapsed = display.collapsed_dirs.contains(&key);
            let view = DirView {
                group,
                project,
                path: &dir,
                element_key: &key,
                paths: &paths,
                all_checked,
                collapsed,
            };
            rows.push(self.git_dir_header(&view, p, cx));
            if collapsed {
                continue;
            }
            for entry in items {
                rows.push(self.git_file_row(
                    &super::file_row::GitFileRowProps {
                        project,
                        entry,
                        group,
                        checked: &display.checked,
                        indent: 32.0,
                        selected: diff_matches(active_diff.as_ref(), &entry.path, group),
                    },
                    p,
                    cx,
                ));
            }
        }
        rows
    }

    /// 目录行：折叠箭头 + 整目录勾选 + 目录名 + 数量。
    fn git_dir_header(
        &mut self,
        view: &DirView<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (group, project, dir, key, paths) = (
            view.group,
            view.project,
            view.path,
            view.element_key,
            view.paths,
        );
        let (all_checked, collapsed) = (view.all_checked, view.collapsed);
        let count = paths.len();
        let app: Entity<AppShell> = cx.entity().clone();
        let menu_paths = paths.to_vec();
        let select_paths = paths.to_vec();
        let toggle_project = project.clone();
        let toggle_key = key.to_string();
        let select_project = project.clone();
        let display_key = key.to_string();
        let mut row = h_flex()
            .id(ElementId::Name(format!("git-dir-{display_key}").into()))
            .h(px(22.0))
            .pl(px(24.0))
            .pr_2()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall(),
            );
        if group.actionable() {
            row = row.child(
                div()
                    .id(ElementId::Name(
                        format!("git-dir-check-{display_key}").into(),
                    ))
                    .flex_none()
                    .child(check_box(all_checked, p))
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        let target = !all_checked;
                        this.git_set_group_checked(&select_project, &select_paths, target, cx);
                    })),
            );
        } else {
            row = row.child(check_placeholder());
        }
        let row = row
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(if dir.is_empty() {
                        rust_i18n::t!("git.group.root").to_string()
                    } else {
                        dir.to_string()
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(format!("({count})")),
            )
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.git_toggle_dir(&toggle_project, &toggle_key, cx);
            }));
        // 右键菜单会换掉元素类型，只能在链尾按可操作性挂载。
        if group.actionable() {
            row.context_menu(move |menu, _window, _cx| {
                render_group_menu(menu, app.clone(), group, &menu_paths)
            })
            .into_any_element()
        } else {
            row.into_any_element()
        }
    }

    /// 分组头：折叠箭头 + 整组勾选 + 名称 + 数量。
    ///
    /// 五个展示参数收进 [`GroupView`]：平铺的入参在这一处正好超了规范的 5 个上限。
    fn git_group_header(
        &mut self,
        project: &GitProject,
        view: &GroupView<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group = view.group;
        let entries = view.entries;
        let all_checked = view.all_checked;
        let collapsed = view.collapsed;
        let paths: Vec<String> = entries.iter().map(|entry| entry.path.clone()).collect();
        let toggle_project = project.clone();
        let group_key = group.key().to_string();
        let select_project = project.clone();
        let select_paths = paths.clone();
        let app: Entity<AppShell> = cx.entity().clone();
        let menu_paths = paths.clone();
        let mut header = h_flex()
            .id(ElementId::Name(format!("git-group-{}", group.key()).into()))
            .px_2()
            .py_1()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .bg(p.muted.opacity(0.35))
            .text_xs()
            .font_weight(FontWeight::BOLD)
            .text_color(p.muted_foreground)
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall(),
            );
        // 忽略组只读：不摆勾选框与整组菜单，用同宽占位保持与其它组左对齐。
        if group.actionable() {
            header = header.child(
                div()
                    .id(ElementId::Name(
                        format!("git-group-check-{}", group.key()).into(),
                    ))
                    .flex_none()
                    .child(check_box(all_checked, p))
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        let target = !all_checked;
                        this.git_set_group_checked(&select_project, &select_paths, target, cx);
                    })),
            );
        } else {
            header = header.child(check_placeholder());
        }
        let header = header
            .child(div().flex_1().child(group.title()))
            .child(div().child(format!("({})", entries.len())))
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.git_toggle_group(&toggle_project, &group_key, cx);
            }));
        if group.actionable() {
            header
                .context_menu(move |menu, _window, _cx| {
                    render_group_menu(menu, app.clone(), group, &menu_paths)
                })
                .into_any_element()
        } else {
            header.into_any_element()
        }
    }
}

/// 目录行的展示参数包：七个入参超了规范上限，收成一个视图结构。
struct DirView<'a> {
    group: ChangeGroup,
    project: &'a GitProject,
    path: &'a str,
    element_key: &'a str,
    paths: &'a [String],
    all_checked: bool,
    collapsed: bool,
}

/// 分组头的展示参数包。
struct GroupView<'a> {
    group: ChangeGroup,
    entries: &'a [StatusEntry],
    all_checked: bool,
    collapsed: bool,
}

/// 当前 diff 是否对应当前文件与分组：同一路径在暂存/未暂存两组时只亮真正比对的那一行。
fn diff_matches(active: Option<&GitDiffRequest>, path: &str, group: ChangeGroup) -> bool {
    let Some(request) = active else {
        return false;
    };
    if request.path != path {
        return false;
    }
    match group {
        ChangeGroup::Staged => {
            matches!(&request.old, GitSide::Commit(_)) && matches!(&request.new, GitSide::Index)
        }
        _ => matches!(&request.old, GitSide::Index) && matches!(&request.new, GitSide::Worktree),
    }
}
