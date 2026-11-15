//! 「打开项目」对话框：选目录（应用内浏览）+ 分组 + 图标与环境色。
//!
//! 设计原理（Why）：
//! - 一个对话框同时覆盖「已有仓库」和「普通目录（含空目录）」两种入口：用户的真实
//!   意图都是「把这个目录变成受管项目」，差别只在它当前是不是仓库，而这一步由探测
//!   结果自动决定，确认按钮的文案跟着变，杜绝点完什么都没发生；
//! - 选目录用应用内浏览面板而不是操作系统对话框（见 `dir_browser`）：层级看得见、
//!   配色跟主题一致，路径框仍然可以粘完整路径；
//! - 探测与初始化都必须走命令总线在控制器里跑子进程：目录可能在网络盘上，
//!   `rev-parse` 卡住会冻结整个窗口；
//! - 外观与归组在添加时就给：仓库一旦进了列表，行首图标与环境色就是用户认仓库的
//!   主要线索，事后再补一次编辑对话框不如一次填完（与会话表单同一口径）。

use super::dir_browser::{GitHost, render_git_browser};
use super::folder_field::render_git_folder_field;
use super::form::{GitProjectDraft, GitProjectModal};
use crate::app::AppShell;
use crate::keys::expand_home;
use crate::palette::Palette;
use crate::session_appearance::{render_appearance_color_row, render_appearance_icon_row};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitCommand, GitFolderId, GitRequestId, SessionColor, SessionIcon};

/// 打开「打开项目」对话框；`folder` 是「在此分组添加」带进来的目标分组。
pub(crate) fn open_git_project_modal(
    shell: &mut AppShell,
    folder: Option<GitFolderId>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let placeholder = rust_i18n::t!("git.project.path_placeholder").to_string();
    let folder_placeholder = rust_i18n::t!("git.project.folder_placeholder").to_string();
    // 「在此分组添加」带进来的是分组 id，字段里存的是名字：回填成名字，
    // 用户看到的就是他熟悉的那一行文字，而不是一个内部标识。
    let preset_name = folder
        .and_then(|id| {
            shell
                .state
                .git
                .folders
                .iter()
                .find(|item| item.id == id)
                .map(|item| item.name.clone())
        })
        .unwrap_or_default();
    shell.state.modals.git.project = Some(GitProjectModal {
        picker: super::dir_browser::DirPicker::new(window, cx, &placeholder),
        folder: super::form::GitFolderField::new(window, cx, &folder_placeholder),
        icon: SessionIcon::default(),
        color: SessionColor::default(),
        detected: None,
        probing: false,
        init_if_missing: false,
        request_id: GitRequestId::new(),
    });
    if !preset_name.is_empty() {
        super::folder_field::git_folder_pick(shell, GitHost::Modal, preset_name, window, cx);
    }
    cx.notify();
    // 开窗就按面板起点走一次：用户不该先点一下才知道家目录里有什么。
    if let Some(current) = shell
        .state
        .modals
        .git
        .project
        .as_ref()
        .map(|modal| modal.picker.browser.root.clone())
    {
        crate::git::dir_browser::git_browse_root(shell, GitHost::Modal, current, window, cx);
    }
}

/// 关闭对话框。
pub(crate) fn close_git_project_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell.state.modals.git.project = None;
    cx.notify();
}

/// 提交本机仓库表单：若通过校验则添加项目并关闭弹窗。
pub(crate) fn commit_git_project_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let Some(modal) = shell.state.modals.git.project.as_ref() else {
        return;
    };
    let path = expand_home(&modal.picker.path.read(cx).value());
    let folder_name = modal.folder.name.read(cx).value().trim().to_string();
    let (icon, color, already_repo, init) = (
        modal.icon,
        modal.color,
        modal.detected.is_some(),
        modal.init_if_missing,
    );
    let mut draft = GitProjectDraft::new(path);
    draft.icon = icon;
    draft.color = color;
    if already_repo {
        draft.folder_id = super::folder_field::git_folder_id_for_name(shell, &folder_name, cx);
        crate::git::actions::add_git_project(shell, draft, cx);
        shell.state.modals.git.project = None;
    } else if init {
        draft.folder_id = super::folder_field::git_folder_id_for_name(shell, &folder_name, cx);
        init_git_project_dir(shell, draft, cx);
    } else {
        draft.folder_id = super::folder_field::git_folder_id_for_name(shell, &folder_name, cx);
        crate::git::actions::add_git_project(shell, draft, cx);
        shell.state.modals.git.project = None;
    }
    cx.notify();
}

/// 添加对话框主体（接入 dialog_host）。
pub(crate) fn render_git_project_dialog_body(
    shell: &AppShell,
    app: &Entity<AppShell>,
    _window: &mut Window,
    cx: &App,
    p: &Palette,
) -> AnyElement {
    let Some(modal) = shell.state.modals.git.project.as_ref() else {
        return div().into_any_element();
    };
    let detected = modal.detected.clone();
    let probing = modal.probing;
    let error = modal.picker.error.clone();
    let path_value = modal.picker.path.read(cx).value().trim().to_string();
    let folders = super::folder_field::git_folder_names(shell);
    let folder = modal.folder.clone();
    let icon_choice = modal.icon;
    let color_choice = modal.color;
    let init_if_missing = modal.init_if_missing;

    let status_line = if let Some(info) = &detected {
        format!(
            "{} · {}",
            rust_i18n::t!("git.project.is_repo"),
            info.toplevel.display()
        )
    } else if probing {
        rust_i18n::t!("git.project.probing").to_string()
    } else if path_value.is_empty() {
        rust_i18n::t!("git.project.hint").to_string()
    } else {
        rust_i18n::t!("git.project.not_repo").to_string()
    };
    let can_confirm = can_confirm_project(&path_value, probing, error.is_some());

    let mut body =
        v_flex()
            .w_full()
            .gap_2()
            .child(render_git_browser(
                &modal.picker,
                GitHost::Modal,
                14,
                true,
                p,
                app,
                cx,
            ))
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(rust_i18n::t!("form.icon").to_string()),
                            )
                            .child(render_appearance_icon_row(
                                "git-project",
                                icon_choice,
                                move |this: &mut AppShell, choice, cx| {
                                    if let Some(modal) = this.state.modals.git.project.as_mut() {
                                        modal.icon = choice;
                                    }
                                    cx.notify();
                                },
                                app,
                                cx,
                            )),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(rust_i18n::t!("form.color").to_string()),
                            )
                            .child(render_appearance_color_row(
                                "git-project",
                                color_choice,
                                move |this: &mut AppShell, choice, cx| {
                                    if let Some(modal) = this.state.modals.git.project.as_mut() {
                                        modal.color = choice;
                                    }
                                    cx.notify();
                                },
                                p,
                                app,
                                cx,
                            )),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .h(px(64.0))
                    .gap_6()
                    .items_start()
                    .child(div().flex_1().min_w_0().child(render_git_folder_field(
                        &folder,
                        GitHost::Modal,
                        &folders,
                        None,
                        p,
                        app,
                        cx,
                    )))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .pt(px(20.0))
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        Checkbox::new("git-project-init")
                                            .small()
                                            .checked(init_if_missing)
                                            .disabled(detected.is_some())
                                            .on_click({
                                                let app = app.clone();
                                                move |&checked, _, cx| {
                                                    app.update(cx, |this, cx| {
                                                        if let Some(modal) =
                                                            this.state.modals.git.project.as_mut()
                                                        {
                                                            modal.init_if_missing = checked;
                                                        }
                                                        cx.notify();
                                                    });
                                                }
                                            }),
                                    )
                                    .child(div().text_xs().child(
                                        rust_i18n::t!("git.project.init_label").to_string(),
                                    )),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .text_xs()
                                    .text_color(if detected.is_some() {
                                        p.success
                                    } else {
                                        p.muted_foreground
                                    })
                                    .line_clamp(2)
                                    .child(status_line),
                            ),
                    ),
            );
    body = body.when(error.is_some(), |el| {
        el.child(
            div()
                .text_xs()
                .text_color(p.danger)
                .child(error.unwrap_or_default()),
        )
    });

    let footer = h_flex()
        .w_full()
        .gap_2()
        .justify_end()
        .child(crate::widgets::dialog_cancel_button(
            "git-project-cancel",
            rust_i18n::t!("git.project.cancel").to_string(),
            crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                close_git_project_modal(this, cx);
            }),
        ))
        .child(
            crate::widgets::dialog_confirm_button(
                "git-project-confirm",
                rust_i18n::t!("git.name.confirm").to_string(),
                None,
                crate::widgets::DialogConfirmKind::Primary,
                crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                    commit_git_project_modal(this, cx);
                }),
            )
            .disabled(!can_confirm),
        );

    crate::widgets::dialog_body(
        "git-project-dialog",
        rust_i18n::t!("git.project.title").to_string(),
        None,
        body,
        Some(footer),
    )
}

/// 对当前输入路径发起仓库探测。
///
/// 只要路径一变就自动跑一次：底部按钮的文案（添加 / 初始化并添加）读的就是这个
/// 结果，因此省掉「检测」按钮，也避免用户点了添加才发现口径不对。
pub(crate) fn probe_git_project_dir(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let Some(modal) = shell.state.modals.git.project.as_mut() else {
        return;
    };
    let raw = modal.picker.path.read(cx).value().trim().to_string();
    if raw.is_empty() {
        return;
    }
    let path = expand_home(&raw);
    let request_id = GitRequestId::new();
    modal.request_id = request_id;
    modal.probing = true;
    modal.detected = None;
    modal.picker.error = None;
    shell.send_command(GitCommand::Probe { request_id, path });
    cx.notify();
}

/// 对普通目录执行 `git init`，成功后按同一份草稿加入列表。
fn init_git_project_dir(shell: &mut AppShell, draft: GitProjectDraft, cx: &mut Context<AppShell>) {
    let request_id = GitRequestId::new();
    let path = draft.path.clone();
    if let Some(modal) = shell.state.modals.git.project.as_mut() {
        modal.request_id = request_id;
        modal.probing = true;
        modal.picker.error = None;
    }
    shell.state.git.pending_init = Some((request_id, draft));
    shell.send_command(GitCommand::Init { request_id, path });
    cx.notify();
}

/// 处理探测与初始化事件。
pub(crate) fn apply_git_probe_event(
    shell: &mut AppShell,
    event: transom_core::GitAppEvent,
    cx: &mut Context<AppShell>,
) {
    use transom_core::GitAppEvent;
    match event {
        GitAppEvent::Probed {
            request_id,
            path,
            info,
            error,
        } => {
            let Some(modal) = shell.state.modals.git.project.as_mut() else {
                return;
            };
            if modal.request_id != request_id {
                return;
            }
            modal.probing = false;
            modal.picker.error = error.as_ref().map(crate::palette::localized_error);
            modal.detected = info.map(|info| *info);
            if modal.detected.is_none() && modal.picker.error.is_none() {
                // 探测成功但不是仓库：保持空 detected，界面显示「初始化并添加」。
                log::debug!(
                    "目录不是 Git 仓库，等待用户选择初始化，路径: {}",
                    path.display()
                );
            }
            cx.notify();
        }
        GitAppEvent::Inited {
            request_id,
            path,
            error,
        } => {
            let Some((pending_id, draft)) = shell.state.git.pending_init.take() else {
                return;
            };
            if pending_id != request_id {
                // 不是这一次弹窗初始化发起的回调：放回去，别把别的初始化
                // 结果错记到当前草稿上。
                shell.state.git.pending_init = Some((pending_id, draft));
                return;
            }
            if let Some(modal) = shell.state.modals.git.project.as_mut() {
                modal.probing = false;
                modal.request_id = request_id;
            }
            match error {
                Some(error) => {
                    let message = crate::palette::localized_error(&error);
                    log::error!(
                        "初始化 Git 仓库失败，路径: {}，原因: {message}",
                        path.display()
                    );
                    if let Some(modal) = shell.state.modals.git.project.as_mut() {
                        modal.picker.error = Some(message);
                    }
                }
                None => {
                    crate::git::actions::add_git_project(shell, draft, cx);
                    shell.state.modals.git.project = None;
                }
            }
            cx.notify();
        }
        _ => {}
    }
}

/// 确定按钮的可用口径：只拦空路径、探测中与探测报错。
///
/// 不是仓库不等于不能加入——未初始化项目会进入工作区并附带初始化入口，
/// 因此这里不再要求「已探测到仓库」或「勾选初始化」。
fn can_confirm_project(path_value: &str, probing: bool, has_error: bool) -> bool {
    !path_value.trim().is_empty() && !probing && !has_error
}

#[cfg(test)]
mod tests {
    use super::can_confirm_project;

    #[test]
    fn non_repo_directory_can_be_added_without_init() {
        assert!(can_confirm_project("/home/wangle/Downloads", false, false));
    }

    #[test]
    fn empty_probing_or_failed_path_stays_disabled() {
        assert!(!can_confirm_project("", false, false));
        assert!(!can_confirm_project("   ", false, false));
        assert!(!can_confirm_project("/home/wangle", true, false));
        assert!(!can_confirm_project("/home/wangle", false, true));
    }
}
