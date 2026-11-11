//! 离线搁置项（Shelf）面板：IDE 独立离线补丁集管理。
//!
//! 设计原理（Why）：
//! - 形态对齐 IDEA 的「搁置（Shelf）」：与 Git Stash 严格区分，搁置是保存在本地磁盘
//!   的离线补丁集，不依赖 Git 仓库内部的 stash 栈；
//! - 支持一键将勾选变更生成补丁并从工作区抽离，也可随时将补丁应用（Apply）、
//!   弹回（Pop，即应用后删除）或彻底删除（Drop）；
//! - 即使切换了分支或在不同工作树中，离线补丁永不丢失。

use super::commit_modal::split_path;
use crate::app::AppShell;
use crate::git::types::GitDiffRequest;
use crate::palette::Palette;
use crate::widgets::menu_item;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitCommand, GitRequestId, GitSide, HeadState, ShelfEntry, TabId};
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;
use transom_ui_icons::{AppIcon, render_app_icon};

impl AppShell {
    /// 搁置内联面板：提交页左栏的「搁置」页签内容。
    pub(crate) fn render_git_shelf_panel(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return div().into_any_element();
        };
        let (project, entries, expanded) = {
            let state = &workspace.read(cx).state;
            (
                state.project.clone(),
                state.shelves.clone(),
                state.expanded_shelf.clone(),
            )
        };

        let refresh_project = project.clone();
        let toolbar = h_flex()
            .w_full()
            .px_2()
            .py_1()
            .gap_2()
            .items_center()
            .border_color(p.divider(p.border))
            .border_b_1()
            .child(
                Button::new("git-shelf-refresh")
                    .ghost()
                    .xsmall()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
                    .accessibility_label(rust_i18n::t!("git.menu.refresh").to_string())
                    .tooltip(rust_i18n::t!("git.menu.refresh").to_string())
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.git_refresh(&refresh_project, cx);
                        this.git_reload_shelves(tab_id, cx);
                    })),
            )
            .child(
                Button::new("git-shelf-new")
                    .primary()
                    .xsmall()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::PATCH)).small())
                    .label(rust_i18n::t!("git.shelf.create").to_string())
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.git_shelve_checked(tab_id, cx);
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(format!(
                        "{} ({})",
                        rust_i18n::t!("git.shelf.title"),
                        entries.len()
                    )),
            );

        let mut list = v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .gap_1()
            .overflow_y_scrollbar();
        if entries.is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("git.shelf.empty").to_string()),
            );
        }
        for entry in &entries {
            let is_expanded = expanded.as_deref() == Some(entry.id.as_str());
            list = list.child(self.render_shelf_entry_row(tab_id, entry, is_expanded, p, cx));
        }

        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .child(list)
            .into_any_element()
    }

    /// 单个搁置项：标题行与展开的文件列表。
    fn render_shelf_entry_row(
        &mut self,
        tab_id: TabId,
        entry: &ShelfEntry,
        expanded: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry_id = entry.id.clone();
        let toggle_id = entry.id.clone();
        let apply_id = entry.id.clone();
        let pop_id = entry.id.clone();
        let menu_id = entry.id.clone();

        let toggle_area = h_flex()
            .id(ElementId::Name(format!("git-shelf-head-{entry_id}").into()))
            .flex_1()
            .min_w_0()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .child(
                AppIconElement::new(if expanded {
                    AppIcon::Ui(UiIcon::CHEVRON_DOWN)
                } else {
                    AppIcon::Ui(UiIcon::CHEVRON_RIGHT)
                })
                .xsmall()
                .text_color(p.muted_foreground),
            )
            .child(
                AppIconElement::new(AppIcon::Ui(UiIcon::PATCH))
                    .xsmall()
                    .text_color(p.primary),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(p.foreground)
                            .child(entry.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(format!("{} · {}", entry.branch, entry.created_at)),
                    ),
            )
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.git_toggle_shelf_expanded(tab_id, &toggle_id, cx);
            }));

        let title = h_flex()
            .px_2()
            .py_1p5()
            .gap_2()
            .items_center()
            .child(toggle_area)
            .child(
                Button::new(ElementId::Name(
                    format!("git-shelf-apply-{entry_id}").into(),
                ))
                .ghost()
                .xsmall()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::RUN)).small())
                .accessibility_label(rust_i18n::t!("git.shelf.apply").to_string())
                .tooltip(rust_i18n::t!("git.shelf.apply").to_string())
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.git_apply_shelf(tab_id, &apply_id, false, cx);
                })),
            )
            .child(
                Button::new(ElementId::Name(format!("git-shelf-pop-{entry_id}").into()))
                    .ghost()
                    .xsmall()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWNLOAD)).small())
                    .accessibility_label(rust_i18n::t!("git.shelf.pop").to_string())
                    .tooltip(rust_i18n::t!("git.shelf.pop").to_string())
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.git_apply_shelf(tab_id, &pop_id, true, cx);
                    })),
            )
            .context_menu({
                let app: Entity<AppShell> = cx.entity().clone();
                move |menu, _window, _cx| {
                    render_shelf_popup_menu(menu, app.clone(), tab_id, menu_id.clone())
                }
            });

        let mut rows = v_flex()
            .gap_0()
            .border_color(p.divider(p.border))
            .border_b_1()
            .child(title);
        if !expanded {
            return rows.into_any_element();
        }
        if entry.files.is_empty() {
            return rows
                .child(
                    div()
                        .pl_6()
                        .py_1()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("git.shelf.no_files").to_string()),
                )
                .into_any_element();
        }
        for file in &entry.files {
            rows = rows.child(self.render_shelf_file_row(tab_id, entry, file, p, cx));
        }
        rows.into_any_element()
    }

    /// 搁置清单内单行文件项。
    fn render_shelf_file_row(
        &mut self,
        tab_id: TabId,
        entry: &ShelfEntry,
        file_path: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (name, dir_prefix) = split_path(file_path);
        let path_str = file_path.to_string();
        let selected = self.state.git_workspace(tab_id).is_some_and(|workspace| {
            workspace
                .read(cx)
                .state
                .diff
                .as_ref()
                .is_some_and(|diff| diff.path == path_str)
        });
        let tooltip_path = path_str.clone();
        let file_for_diff = path_str.clone();
        let entry_for_diff = entry.clone();
        h_flex()
            .id(ElementId::Name(
                format!("git-shelf-file-{}-{path_str}", entry.id).into(),
            ))
            .h(px(22.0))
            .pl_6()
            .pr_2()
            .gap_2()
            .items_center()
            .overflow_hidden()
            .cursor_pointer()
            .hover(|style| style.bg(p.list_hover))
            .when(selected, |el| el.bg(p.list_active))
            .tooltip(move |window, cx| Tooltip::new(tooltip_path.clone()).build(window, cx))
            .child(render_app_icon(AppIcon::for_file(&path_str), 16.0))
            .child(
                div()
                    .text_sm()
                    .flex_none()
                    .max_w(px(180.0))
                    .overflow_hidden()
                    .truncate()
                    .child(name),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .truncate()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(dir_prefix),
            )
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.git_open_shelf_file_diff(tab_id, &entry_for_diff, &file_for_diff, cx);
            }))
            .into_any_element()
    }

    /// 展开或折叠一个搁置项。
    pub(crate) fn git_toggle_shelf_expanded(
        &mut self,
        tab_id: TabId,
        entry_id: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.state.git_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                let state = &mut workspace.state;
                state.expanded_shelf = if state.expanded_shelf.as_deref() == Some(entry_id) {
                    None
                } else {
                    Some(entry_id.to_string())
                };
            });
        }
        cx.notify();
    }

    /// 重新读取搁置项列表。
    ///
    /// 读盘经命令总线交给控制器；界面只登记请求号，事件落地时整体替换列表。
    pub(crate) fn git_reload_shelves(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let request_id = GitRequestId::new();
        let project = workspace.update(cx, |workspace, _| {
            workspace.state.pending.push(request_id);
            workspace.state.project.clone()
        });
        self.send_command(GitCommand::LoadShelves {
            project,
            request_id,
        });
        cx.notify();
    }

    /// 把工作区勾选的改动创建为独立离线搁置项（Shelf）。
    ///
    /// 补丁导出、工作区抽离与落盘都在控制器里完成：界面只提交「搁置意图」，
    /// 失败经 [`transom_core::GitAppEvent::ShelfFailed`] 回到状态栏。
    pub(crate) fn git_shelve_checked(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let (project, files, branch) = {
            let state = &workspace.read(cx).state;
            let files: Vec<String> = if state.checked.is_empty() {
                state
                    .status
                    .as_ref()
                    .map(|status| {
                        status
                            .entries
                            .iter()
                            .map(|entry| entry.path.clone())
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                state.checked.iter().cloned().collect()
            };
            let branch = match state.status.as_ref().map(|status| &status.head) {
                Some(HeadState::OnBranch(branch)) => branch.clone(),
                Some(HeadState::Initial { branch }) => branch.clone(),
                _ => "HEAD".to_string(),
            };
            (state.project.clone(), files, branch)
        };
        if files.is_empty() {
            return;
        }
        let name = format!("{} ({})", rust_i18n::t!("git.shelf.title"), files.len());
        self.send_command(GitCommand::Shelve {
            project,
            request_id: GitRequestId::new(),
            name,
            branch,
            files,
        });
        cx.notify();
    }

    /// 应用或弹回搁置项。
    ///
    /// 补丁文本随命令携带：控制器不必再按 id 回读存储，界面手里的条目
    /// 就是用户点击时看到的那一份。
    pub(crate) fn git_apply_shelf(
        &mut self,
        tab_id: TabId,
        entry_id: &str,
        pop: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let (project, patch) = {
            let state = &workspace.read(cx).state;
            let Some(entry) = state.shelves.iter().find(|shelf| shelf.id == entry_id) else {
                return;
            };
            (state.project.clone(), entry.patch.clone())
        };
        self.send_command(GitCommand::ApplyShelf {
            project,
            request_id: GitRequestId::new(),
            shelf_id: entry_id.to_string(),
            patch,
            pop,
        });
        cx.notify();
    }

    /// 删除指定搁置项。
    pub(crate) fn git_drop_shelf(&mut self, tab_id: TabId, entry_id: &str, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let project = workspace.read(cx).state.project.clone();
        self.send_command(GitCommand::DropShelf {
            project,
            request_id: GitRequestId::new(),
            shelf_id: entry_id.to_string(),
        });
        cx.notify();
    }

    /// 打开搁置项中某文件的差异。
    fn git_open_shelf_file_diff(
        &mut self,
        tab_id: TabId,
        entry: &ShelfEntry,
        path: &str,
        cx: &mut Context<Self>,
    ) {
        self.git_open_diff(
            tab_id,
            GitDiffRequest {
                path: path.to_string(),
                old: GitSide::Commit("HEAD".to_string()),
                new: GitSide::Worktree,
                left_label: format!("{} ({})", rust_i18n::t!("git.diff.baseline"), entry.branch),
                right_label: rust_i18n::t!("git.shelf.title").to_string(),
            },
            cx,
        );
    }
}

/// 搁置项右键菜单。
fn render_shelf_popup_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    tab_id: TabId,
    entry_id: String,
) -> PopupMenu {
    let app_apply = app.clone();
    let apply_id = entry_id.clone();
    let app_pop = app.clone();
    let pop_id = entry_id.clone();
    let drop_id = entry_id;
    menu.item(menu_item(
        rust_i18n::t!("git.shelf.apply").to_string(),
        AppIcon::Ui(UiIcon::RUN),
        move |_, _, cx| {
            let id = apply_id.clone();
            app_apply.update(cx, |this, cx| {
                this.git_apply_shelf(tab_id, &id, false, cx);
            });
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.shelf.pop").to_string(),
        AppIcon::Ui(UiIcon::DOWNLOAD),
        move |_, _, cx| {
            let id = pop_id.clone();
            app_pop.update(cx, |this, cx| {
                this.git_apply_shelf(tab_id, &id, true, cx);
            });
        },
    ))
    .item(PopupMenuItem::separator())
    .item(menu_item(
        rust_i18n::t!("git.shelf.drop").to_string(),
        AppIcon::Ui(UiIcon::DELETE),
        move |_, _, cx| {
            let id = drop_id.clone();
            app.update(cx, |this, cx| {
                this.git_drop_shelf(tab_id, &id, cx);
            });
        },
    ))
}
