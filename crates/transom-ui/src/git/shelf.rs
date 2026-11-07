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
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitSide, HeadState, ShelfEntry, TabId};
use transom_store::{load_project_shelves, save_project_shelves};

impl AppShell {
    /// 搁置内联面板：提交页左栏的「搁置」页签内容。
    pub(crate) fn render_git_shelf_panel(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.git_state_of_tab(tab_id) else {
            return div().into_any_element();
        };
        let project = state.project.clone();
        let entries = state.shelves.clone();
        let expanded = state.expanded_shelf.clone();

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
                    .icon(IconName::RotateCw)
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
                    .icon(IconName::PackagePlus)
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
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .xsmall()
                .text_color(p.muted_foreground),
            )
            .child(Icon::new(IconName::Package).xsmall().text_color(p.primary))
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
                .icon(IconName::Play)
                .tooltip(rust_i18n::t!("git.shelf.apply").to_string())
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.git_apply_shelf(tab_id, &apply_id, false, cx);
                })),
            )
            .child(
                Button::new(ElementId::Name(format!("git-shelf-pop-{entry_id}").into()))
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowDownToLine)
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
        let selected = self
            .git_state_of_tab(tab_id)
            .and_then(|state| state.diff.as_ref())
            .is_some_and(|diff| diff.path == path_str);
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
            .child(
                Icon::new(IconName::FileText)
                    .xsmall()
                    .text_color(p.muted_foreground),
            )
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
        if let Some(state) = self.git_state_of_tab_mut(tab_id) {
            state.expanded_shelf = if state.expanded_shelf.as_deref() == Some(entry_id) {
                None
            } else {
                Some(entry_id.to_string())
            };
        }
        cx.notify();
    }

    /// 重新从本地存储中加载搁置项列表。
    pub(crate) fn git_reload_shelves(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        let project_id = state.project.id.to_string();
        if let Ok(shelves) = load_project_shelves(&project_id) {
            state.shelves = shelves;
        }
        cx.notify();
    }

    /// 把工作区勾选的改动创建为独立离线搁置项（Shelf）。
    pub(crate) fn git_shelve_checked(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(state) = self.git_state_of_tab(tab_id) else {
            return;
        };
        let project = state.project.clone();
        let checked_files: Vec<String> = if state.checked.is_empty() {
            state
                .status
                .as_ref()
                .map(|s| s.entries.iter().map(|e| e.path.clone()).collect())
                .unwrap_or_default()
        } else {
            state.checked.iter().cloned().collect()
        };
        if checked_files.is_empty() {
            return;
        }

        let branch = match state.status.as_ref().map(|s| &s.head) {
            Some(HeadState::OnBranch(b)) => b.clone(),
            Some(HeadState::Initial { branch }) => branch.clone(),
            _ => "HEAD".to_string(),
        };
        let project_id_str = project.id.to_string();
        let files_count = checked_files.len();
        let name = format!("{} ({files_count})", rust_i18n::t!("git.shelf.title"));

        cx.spawn(async move |this, cx| {
            let repo = transom_git::Repo::at(project.path.clone());
            let Ok(patch) = repo.export_patch(&checked_files).await else {
                return;
            };
            if patch.trim().is_empty() {
                return;
            }
            // 撤销工作区已搁置的已跟踪修改
            let _ = repo.discard(&checked_files).await;

            let now = chrono::Local::now();
            let created_at = now.format("%Y-%m-%d %H:%M:%S").to_string();
            let entry = ShelfEntry {
                id: format!("shelf_{}", now.timestamp_millis()),
                name,
                branch,
                created_at,
                files: checked_files,
                patch,
            };

            let _ = this.update(cx, |this, cx| {
                if let Some(state) = this.git_state_of_tab_mut(tab_id) {
                    state.shelves.insert(0, entry.clone());
                    state.checked.clear();
                    state.commit_tab = crate::git::GitCommitTab::Shelf;
                    let _ = save_project_shelves(&project_id_str, &state.shelves);
                }
                this.git_refresh(&project, cx);
            });
        })
        .detach();
    }

    /// 应用或弹回搁置项。
    pub(crate) fn git_apply_shelf(
        &mut self,
        tab_id: TabId,
        entry_id: &str,
        pop: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_state_of_tab(tab_id) else {
            return;
        };
        let project = state.project.clone();
        let Some(entry) = state.shelves.iter().find(|s| s.id == entry_id).cloned() else {
            return;
        };
        let project_id_str = project.id.to_string();
        let target_id = entry_id.to_string();

        cx.spawn(async move |this, cx| {
            let repo = transom_git::Repo::at(project.path.clone());
            if let Ok(()) = repo.apply_patch(&entry.patch).await {
                let _ = this.update(cx, |this, cx| {
                    if pop {
                        if let Some(state) = this.git_state_of_tab_mut(tab_id) {
                            state.shelves.retain(|s| s.id != target_id);
                            let _ = save_project_shelves(&project_id_str, &state.shelves);
                        }
                    }
                    this.git_refresh(&project, cx);
                });
            }
        })
        .detach();
    }

    /// 删除指定搁置项。
    pub(crate) fn git_drop_shelf(&mut self, tab_id: TabId, entry_id: &str, cx: &mut Context<Self>) {
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        let project_id_str = state.project.id.to_string();
        state.shelves.retain(|s| s.id != entry_id);
        let _ = save_project_shelves(&project_id_str, &state.shelves);
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
        IconName::Play,
        move |_, _, cx| {
            let id = apply_id.clone();
            app_apply.update(cx, |this, cx| {
                this.git_apply_shelf(tab_id, &id, false, cx);
            });
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.shelf.pop").to_string(),
        IconName::ArrowDownToLine,
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
        IconName::Trash,
        move |_, _, cx| {
            let id = drop_id.clone();
            app.update(cx, |this, cx| {
                this.git_drop_shelf(tab_id, &id, cx);
            });
        },
    ))
}
