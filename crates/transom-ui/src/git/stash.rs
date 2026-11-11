//! 贮藏（Stash）面板：原生 Git Stash 栈管理。
//!
//! 设计原理（Why）：
//! - 形态对齐 IDEA 的 Git Stash：一列贮藏栈项，每项可应用 / 弹出 / 丢弃，
//!   顶部一个「新建贮藏」入口；
//! - 「应用」与「弹出」分列两个按钮：前者保留栈项、可反复应用到多个分支，
//!   后者会把栈项从 stash 栈移除——界面必须区分明确；
//! - 删除走独立确认，不与列表点击混在一起。

use super::commit_modal::{accent_color, split_path};
use crate::app::AppShell;
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
use transom_core::{GitAction, GitProject, TabId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

impl AppShell {
    /// 贮藏内联面板：提交页左栏的「贮藏」页签内容。
    pub(crate) fn render_git_stash_panel(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return div().into_any_element();
        };
        let (project, entries, expanded, files, loading) = {
            let state = &workspace.read(cx).state;
            (
                state.project.clone(),
                state.stashes.clone(),
                state.expanded_stash.clone(),
                state.stash_files.clone(),
                state.stash_files_loading,
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
                Button::new("git-stash-refresh")
                    .ghost()
                    .xsmall()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
                    .accessibility_label(rust_i18n::t!("git.menu.refresh").to_string())
                    .tooltip(rust_i18n::t!("git.menu.refresh").to_string())
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.git_refresh(&refresh_project, cx);
                        this.git_reload_stashes(cx);
                    })),
            )
            .child(
                Button::new("git-stash-new")
                    .primary()
                    .xsmall()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::PATCH)).small())
                    .label(rust_i18n::t!("git.stash.create").to_string())
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.git_stash_checked(tab_id, cx);
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(format!(
                        "{} ({})",
                        rust_i18n::t!("git.stash.title"),
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
                    .child(rust_i18n::t!("git.stash.empty").to_string()),
            );
        }
        for entry in entries {
            let shown = if expanded.as_deref() == Some(entry.selector.as_str()) {
                files.as_slice()
            } else {
                &[]
            };
            let view = StashRowView {
                project: &project,
                selector: &entry.selector,
                display: entry.display(),
                expanded: expanded.as_deref() == Some(entry.selector.as_str()),
                loading,
                files: shown,
                tab_id,
            };
            list = list.child(self.stash_row(&view, p, cx));
        }

        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .child(list)
            .into_any_element()
    }

    /// 一条贮藏项：标题行 + 展开后的文件清单。
    fn stash_row(
        &mut self,
        view: &StashRowView<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selector = view.selector.to_string();
        let toggle_area = h_flex()
            .id(ElementId::Name(format!("git-stash-head-{selector}").into()))
            .flex_1()
            .min_w_0()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .child(
                AppIconElement::new(if view.expanded {
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
                            .child(view.display.to_string()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(selector.clone()),
                    ),
            )
            .on_click(cx.listener({
                let project = view.project.clone();
                let target = selector.clone();
                move |this, _, _window, cx| {
                    this.git_toggle_stash_files(&project, &target, cx);
                }
            }));
        let title = h_flex()
            .px_2()
            .py_1p5()
            .gap_2()
            .items_center()
            .child(toggle_area)
            .child(
                Button::new(ElementId::Name(
                    format!("git-stash-apply-{selector}").into(),
                ))
                .ghost()
                .xsmall()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::RUN)).small())
                .accessibility_label(rust_i18n::t!("git.stash.apply").to_string())
                .tooltip(rust_i18n::t!("git.stash.apply").to_string())
                .on_click(cx.listener({
                    let project = view.project.clone();
                    let target = selector.clone();
                    move |this, _, _window, cx| {
                        this.git_stash_action(&project, GitAction::StashApply, target.clone(), cx);
                    }
                })),
            )
            .child(
                Button::new(ElementId::Name(format!("git-stash-pop-{selector}").into()))
                    .ghost()
                    .xsmall()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWNLOAD)).small())
                    .accessibility_label(rust_i18n::t!("git.stash.pop").to_string())
                    .tooltip(rust_i18n::t!("git.stash.pop").to_string())
                    .on_click(cx.listener({
                        let project = view.project.clone();
                        let target = selector.clone();
                        move |this, _, _window, cx| {
                            this.git_stash_action(
                                &project,
                                GitAction::StashPop,
                                target.clone(),
                                cx,
                            );
                        }
                    })),
            )
            .context_menu({
                let app: Entity<AppShell> = cx.entity().clone();
                let project = view.project.clone();
                let target = selector.clone();
                move |menu, _window, _cx| {
                    render_stash_menu(menu, app.clone(), &project, target.clone())
                }
            });

        let mut rows = v_flex()
            .gap_0()
            .border_color(p.divider(p.border))
            .border_b_1()
            .child(title);
        if !view.expanded {
            return rows.into_any_element();
        }
        if view.files.is_empty() {
            let text = if view.loading {
                rust_i18n::t!("git.stash.files_loading").to_string()
            } else {
                rust_i18n::t!("git.stash.no_files").to_string()
            };
            return rows
                .child(
                    div()
                        .pl_6()
                        .py_1()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(text),
                )
                .into_any_element();
        }
        for file in view.files {
            rows = rows.child(self.stash_file_row(view, file, p, cx));
        }
        rows.into_any_element()
    }

    /// 贮藏清单里的一行文件：点击即看「贮藏前 vs 贮藏后」的差异。
    fn stash_file_row(
        &mut self,
        view: &StashRowView<'_>,
        file: &transom_core::StashFile,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let visual = crate::git::change_visual(file.change);
        let (name, dir_prefix) = split_path(&file.path);
        let selector = view.selector.to_string();
        let tab_id = view.tab_id;
        let open_file = file.clone();
        let selected = self.state.git_workspace(tab_id).is_some_and(|workspace| {
            workspace
                .read(cx)
                .state
                .diff
                .as_ref()
                .is_some_and(|diff| diff.path == file.path)
        });
        let tooltip_path = file.path.clone();
        h_flex()
            .id(ElementId::Name(
                format!("git-stash-file-{selector}-{}", file.path).into(),
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
                AppIconElement::new(visual.icon)
                    .xsmall()
                    .text_color(accent_color(visual.accent, p)),
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
            .child(
                file.old_path
                    .as_ref()
                    .map(|old| {
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(old.clone())
                            .into_any_element()
                    })
                    .unwrap_or_else(|| div().into_any_element()),
            )
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.git_open_stash_diff(tab_id, &selector, &open_file, cx);
            }))
            .into_any_element()
    }

    /// 把改动保存到 Git Stash 栈。
    pub(crate) fn git_stash_checked(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let (message, project) = {
            let state = &workspace.read(cx).state;
            let message = if state.checked.is_empty() {
                rust_i18n::t!("git.stash.auto_message").to_string()
            } else {
                format!(
                    "{} ({})",
                    rust_i18n::t!("git.stash.title"),
                    state.checked.len()
                )
            };
            (message, state.project.clone())
        };
        self.git_stash_save(&project, message, false, cx);
        workspace.update(cx, |workspace, _| {
            workspace.state.checked.clear();
        });
        cx.notify();
    }

    /// 重新拉取贮藏列表。
    pub(crate) fn git_reload_stashes(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.active_git_tab() else {
            return;
        };
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let project = workspace.read(cx).state.project.clone();
        let request_id = transom_core::GitRequestId::new();
        workspace.update(cx, |workspace, _| {
            workspace.state.pending.push(request_id);
        });
        self.send_command(transom_core::GitCommand::ListStashes {
            project,
            request_id,
        });
        cx.notify();
    }
}

/// 贮藏行的展示参数包。
struct StashRowView<'a> {
    project: &'a GitProject,
    selector: &'a str,
    display: &'a str,
    expanded: bool,
    loading: bool,
    files: &'a [transom_core::StashFile],
    tab_id: TabId,
}

/// 贮藏项右键菜单。
fn render_stash_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    project: &GitProject,
    selector: String,
) -> PopupMenu {
    let drop_app = app.clone();
    let drop_project = project.clone();
    let drop_selector = selector.clone();
    menu.item(menu_item(
        rust_i18n::t!("git.stash.apply").to_string(),
        AppIcon::Ui(UiIcon::RUN),
        {
            let item_app = app.clone();
            let item_project = project.clone();
            let item_selector = selector.clone();
            move |_, _, cx| {
                let project = item_project.clone();
                let selector = item_selector.clone();
                item_app.update(cx, |this, cx| {
                    this.git_stash_action(&project, GitAction::StashApply, selector, cx);
                });
            }
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.stash.pop").to_string(),
        AppIcon::Ui(UiIcon::DOWNLOAD),
        {
            let item_app = app.clone();
            let item_project = project.clone();
            move |_, _, cx| {
                let project = item_project.clone();
                let selector = selector.clone();
                item_app.update(cx, |this, cx| {
                    this.git_stash_action(&project, GitAction::StashPop, selector, cx);
                });
            }
        },
    ))
    .item(PopupMenuItem::separator())
    .item(menu_item(
        rust_i18n::t!("git.stash.drop").to_string(),
        AppIcon::Ui(UiIcon::DELETE),
        move |_, _, cx| {
            let project = drop_project.clone();
            let selector = drop_selector.clone();
            drop_app.update(cx, |this, cx| {
                this.git_stash_action(&project, GitAction::StashDrop, selector, cx);
            });
        },
    ))
}
