//! 提交面板的改动树：三个分组、按目录折叠、分组头与目录行。
//!
//! 设计原理（Why）：分组渲染是提交面板里最重的一段（分组头 + 目录行 + 文件行三层
//! 嵌套），单独成模块后，改折叠或改分组顺序只碰这一处；它只依赖 `ChangeGroup` 与
//! 父模块的可视件（勾选框、状态字母、配色），不反向依赖对话框本体。

use super::dir_tree::{DirTreeNode, build_dir_tree, compute_check_state};
use super::group_menu::render_group_menu;
use super::{ChangeGroup, CheckState, GroupDisplay, check_box_state, check_placeholder, hint};
use crate::app::AppShell;
use crate::git::actions::GitDiffRequest;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_core::{ChangeCode, GitProject, GitSide, StatusEntry, StatusRecord};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

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
            .state
            .git_workspace_of_project(project.id, cx)
            .and_then(|workspace| workspace.read(cx).state.status.clone())
        else {
            return hint(rust_i18n::t!("git.overview.loading").to_string(), p);
        };
        let active_diff = self
            .state
            .git_workspace_of_project(project.id, cx)
            .and_then(|workspace| workspace.read(cx).state.last_diff.clone());
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

        let mut list = v_flex().gap_0().flex_1().min_h_0().overflow_y_scrollbar();

        let conflict_count = status.conflicts().len();
        if conflict_count > 0 {
            let conflict_project = project.clone();
            let app_resolve = cx.entity().clone();
            let banner = h_flex()
                .w_full()
                .h(px(28.0))
                .px_2()
                .items_center()
                .justify_between()
                .bg(p.tone.warning_subtle)
                .border_1()
                .border_color(p.warning.opacity(0.35))
                .rounded(px(4.0))
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .child(
                            AppIconElement::new(AppIcon::Ui(UiIcon::WARNING))
                                .xsmall()
                                .text_color(p.warning),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.warning)
                                .child(
                                    rust_i18n::t!("git.conflict.banner", count = conflict_count)
                                        .to_string(),
                                ),
                        ),
                )
                .child(
                    Button::new("btn-open-conflict-resolver")
                        .primary()
                        .xsmall()
                        .label(rust_i18n::t!("git.conflict.resolve_btn").to_string())
                        .on_click(move |_, _, cx| {
                            app_resolve.update(cx, |this, cx| {
                                this.open_git_conflict_modal(&conflict_project, cx);
                            });
                        }),
                );
            list = list.child(banner);
        }

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
            let group_paths: Vec<String> = entries.iter().map(|entry| entry.path.clone()).collect();
            let check_state = compute_check_state(&group_paths, checked);
            list = list.child(self.git_group_header(
                project,
                &GroupView {
                    group,
                    entries,
                    check_state,
                    collapsed: group_collapsed,
                },
                p,
                cx,
            ));
            if group_collapsed {
                continue;
            }
            if display.by_directory {
                let dir_ctx = DirRenderCtx {
                    project,
                    group,
                    display,
                    active_diff: active_diff.as_ref(),
                    palette: p,
                };
                list = list.children(self.git_dir_rows(&dir_ctx, entries, cx));
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
                        show_dir_prefix: true,
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

    /// 按目录模式：构建多级压缩目录树，按层级缩进渲染目录行与文件行。
    fn git_dir_rows(
        &mut self,
        ctx: &DirRenderCtx<'_>,
        entries: &[StatusEntry],
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let tree = build_dir_tree(entries);
        let mut rows = Vec::with_capacity(entries.len());
        self.append_dir_tree_rows(ctx, &tree, 0, &mut rows, cx);
        rows
    }

    fn append_dir_tree_rows(
        &mut self,
        ctx: &DirRenderCtx<'_>,
        nodes: &[DirTreeNode<'_>],
        depth: usize,
        rows: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let dir_indent = 16.0 + depth as f32 * 20.0;
        // 文件行没有左侧折叠箭头（14px 图标 + 6px 间距），补齐 20px 使同级复选框与子级对齐父文件夹图标。
        let file_indent = dir_indent + 20.0;
        for node in nodes {
            match node {
                DirTreeNode::Directory {
                    name,
                    dir_path,
                    all_paths,
                    children,
                } => {
                    let key = format!("{}|{dir_path}", ctx.group.key());
                    let check_state = compute_check_state(all_paths, &ctx.display.checked);
                    let collapsed = ctx.display.collapsed_dirs.contains(&key);
                    let view = DirView {
                        group: ctx.group,
                        project: ctx.project,
                        name,
                        element_key: &key,
                        paths: all_paths,
                        indent: dir_indent,
                        check_state,
                        collapsed,
                    };
                    rows.push(self.git_dir_header(&view, ctx.palette, cx));
                    if !collapsed {
                        self.append_dir_tree_rows(ctx, children, depth + 1, rows, cx);
                    }
                }
                DirTreeNode::File(entry) => {
                    rows.push(self.git_file_row(
                        &super::file_row::GitFileRowProps {
                            project: ctx.project,
                            entry,
                            group: ctx.group,
                            checked: &ctx.display.checked,
                            indent: file_indent,
                            selected: diff_matches(ctx.active_diff, &entry.path, ctx.group),
                            show_dir_prefix: false,
                        },
                        ctx.palette,
                        cx,
                    ));
                }
            }
        }
    }

    /// 目录行：仅点击折叠箭头展开/收起，点击多选框仅切换整目录选中态。
    fn git_dir_header(
        &mut self,
        view: &DirView<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (group, project, name, key, paths) = (
            view.group,
            view.project,
            view.name,
            view.element_key,
            view.paths,
        );
        let (check_state, collapsed, indent) = (view.check_state, view.collapsed, view.indent);
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
            .h(px(24.0))
            .pl(px(indent))
            .pr_2()
            .gap_1p5()
            .items_center()
            .hover(|style| style.bg(p.list_hover))
            .child(
                div()
                    .id(ElementId::Name(
                        format!("git-dir-toggle-{display_key}").into(),
                    ))
                    .flex_none()
                    .cursor_pointer()
                    .child(
                        AppIconElement::new(if collapsed {
                            AppIcon::Ui(UiIcon::CHEVRON_RIGHT)
                        } else {
                            AppIcon::Ui(UiIcon::CHEVRON_DOWN)
                        })
                        .xsmall(),
                    )
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        cx.stop_propagation();
                        this.git_toggle_dir(&toggle_project, &toggle_key, cx);
                    })),
            );
        if group.actionable() {
            row = row.child(
                div()
                    .id(ElementId::Name(
                        format!("git-dir-check-{display_key}").into(),
                    ))
                    .flex_none()
                    .cursor_pointer()
                    .child(check_box_state(check_state, p))
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        cx.stop_propagation();
                        let target = check_state != CheckState::Checked;
                        this.git_set_group_checked(&select_project, &select_paths, target, cx);
                    })),
            );
        } else {
            row = row.child(check_placeholder());
        }
        let row = row
            .child(
                AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER))
                    .with_rems(rems(crate::theme::tokens::control::ICON)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(p.foreground)
                    .child(name.to_string()),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("git.commit.file_count", count = count).to_string()),
            );
        // 右键菜单会换掉元素类型，只能在链尾按可操作性挂载。
        if group.actionable() {
            let menu_project = view.project.clone();
            row.context_menu(move |menu, _window, _cx| {
                render_group_menu(menu, app.clone(), &menu_project, group, &menu_paths)
            })
            .into_any_element()
        } else {
            row.into_any_element()
        }
    }

    /// 分组头：折叠箭头 + 三态勾选 + 名称 + 数量。
    fn git_group_header(
        &mut self,
        project: &GitProject,
        view: &GroupView<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group = view.group;
        let entries = view.entries;
        let check_state = view.check_state;
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
            .h(px(24.0))
            .px_2()
            .gap_1p5()
            .items_center()
            .bg(p.surface.active)
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.foreground)
            .child(
                div()
                    .id(ElementId::Name(
                        format!("git-group-toggle-{}", group.key()).into(),
                    ))
                    .flex_none()
                    .cursor_pointer()
                    .child(
                        AppIconElement::new(if collapsed {
                            AppIcon::Ui(UiIcon::CHEVRON_RIGHT)
                        } else {
                            AppIcon::Ui(UiIcon::CHEVRON_DOWN)
                        })
                        .xsmall(),
                    )
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        cx.stop_propagation();
                        this.git_toggle_group(&toggle_project, &group_key, cx);
                    })),
            );
        // 忽略组只读：不摆勾选框与整组菜单，用同宽占位保持与其它组左对齐。
        if group.actionable() {
            header = header.child(
                div()
                    .id(ElementId::Name(
                        format!("git-group-check-{}", group.key()).into(),
                    ))
                    .flex_none()
                    .cursor_pointer()
                    .child(check_box_state(check_state, p))
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        cx.stop_propagation();
                        let target = check_state != CheckState::Checked;
                        this.git_set_group_checked(&select_project, &select_paths, target, cx);
                    })),
            );
        } else {
            header = header.child(check_placeholder());
        }
        let header = header.child(div().flex_none().child(group.title())).child(
            div()
                .flex_1()
                .text_xs()
                .font_weight(FontWeight::NORMAL)
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("git.commit.file_count", count = entries.len()).to_string()),
        );
        if group.actionable() {
            let menu_project = project.clone();
            header
                .context_menu(move |menu, _window, _cx| {
                    render_group_menu(menu, app.clone(), &menu_project, group, &menu_paths)
                })
                .into_any_element()
        } else {
            header.into_any_element()
        }
    }
}

/// 递归渲染目录树的上下文参数包。
struct DirRenderCtx<'a> {
    project: &'a GitProject,
    group: ChangeGroup,
    display: &'a GroupDisplay,
    active_diff: Option<&'a GitDiffRequest>,
    palette: &'a Palette,
}

/// 目录行的展示参数包。
struct DirView<'a> {
    group: ChangeGroup,
    project: &'a GitProject,
    name: &'a str,
    element_key: &'a str,
    paths: &'a [String],
    indent: f32,
    check_state: CheckState,
    collapsed: bool,
}

/// 分组头的展示参数包。
struct GroupView<'a> {
    group: ChangeGroup,
    entries: &'a [StatusEntry],
    check_state: CheckState,
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
