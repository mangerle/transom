//! 克隆后台任务管理、事件更新与右下角进度卡渲染。

use super::calc::clone_percent;
use crate::app::AppShell;
use crate::git::form::GitProjectDraft;
use crate::palette::{Palette, localized_error};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitAppEvent, GitCommand, GitRequestId, SessionColor, SessionIcon};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 在途克隆任务：弹窗关闭后由右下角进度卡接管。
pub(crate) struct GitCloneTask {
    pub request_id: GitRequestId,
    /// 仓库名（进度卡标题用）。
    pub repo_name: String,
    /// 克隆成功后登记用；分组名在成功时才落库。
    pub folder_name: String,
    pub icon: SessionIcon,
    pub color: SessionColor,
    /// 最新一条非空输出，进度卡副标题。
    pub last_line: String,
    /// 最新百分比；`None` 时进度条走不确定动画。
    pub percent: Option<f32>,
    pub running: bool,
    pub error: Option<String>,
    /// 成功后的目标目录：非空表示仓库已登记，渲染时延后开标签。
    pub done: Option<std::path::PathBuf>,
}

impl AppShell {
    /// 落地克隆进度与完成事件。
    pub(crate) fn apply_git_clone_event(&mut self, event: GitAppEvent, cx: &mut Context<Self>) {
        match event {
            GitAppEvent::CloneProgress { request_id, text } => {
                let Some(task) = self.state.git.clone_task.as_mut() else {
                    return;
                };
                if task.request_id != request_id {
                    return;
                }
                if let Some(line) = text.lines().rev().find(|line| !line.trim().is_empty()) {
                    task.last_line = line.trim().to_string();
                }
                if let Some(percent) = clone_percent(&text) {
                    task.percent = Some(percent);
                }
                cx.notify();
            }
            GitAppEvent::CloneDone {
                request_id,
                dest,
                error,
            } => {
                let (folder_name, icon, color, success) = {
                    let Some(task) = self.state.git.clone_task.as_mut() else {
                        return;
                    };
                    if task.request_id != request_id {
                        return;
                    }
                    task.running = false;
                    match error {
                        Some(error) => {
                            task.error = Some(localized_error(&error));
                            (task.folder_name.clone(), task.icon, task.color, false)
                        }
                        None => {
                            task.error = None;
                            task.done = Some(dest.clone());
                            (task.folder_name.clone(), task.icon, task.color, true)
                        }
                    }
                };
                // 结束对 task 的借用后再走登记：add_git_project 会写项目表与落盘。
                if success {
                    let mut draft = GitProjectDraft::new(dest);
                    draft.icon = icon;
                    draft.color = color;
                    draft.folder_id = self.git_folder_id_for_name(&folder_name, cx);
                    self.add_git_project(draft, cx);
                }
                cx.notify();
            }
            _ => {}
        }
    }

    /// 取消在途克隆：杀掉子进程并收起进度卡。
    pub(crate) fn git_cancel_clone_task(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.state.git.clone_task.take() else {
            return;
        };
        self.send_command(GitCommand::Cancel {
            request_id: task.request_id,
        });
        cx.notify();
    }

    /// 右下角克隆进度卡：非模态，成功后延后开仓库并消失。
    pub(crate) fn render_git_clone_task(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> Option<AnyElement> {
        let task = self.state.git.clone_task.as_ref()?;
        if let Some(path) = task.done.clone() {
            let app = cx.entity().clone();
            window.defer(cx, move |window, cx| {
                app.update(cx, |this, cx| {
                    if let Some(project) = this
                        .state
                        .git
                        .projects
                        .iter()
                        .find(|item| item.path == path)
                        .cloned()
                    {
                        this.open_git_tab(project, window, cx);
                    }
                    this.state.git.clone_task = None;
                    cx.notify();
                });
            });
            return None;
        }
        let running = task.running;
        let error = task.error.clone();
        let last_line = task.last_line.clone();
        let percent = task.percent;
        let title = if error.is_some() {
            rust_i18n::t!("git.clone.task_failed").to_string()
        } else {
            rust_i18n::t!("git.clone.task_title", repo = task.repo_name.as_str()).to_string()
        };
        let detail = error.clone().unwrap_or_else(|| {
            if last_line.is_empty() {
                rust_i18n::t!("git.clone.running").to_string()
            } else {
                last_line
            }
        });
        let detail_color = if error.is_some() {
            p.danger
        } else {
            p.muted_foreground
        };
        let mut bar = Progress::new("git-clone-task-progress").w_full();
        if let Some(value) = percent {
            bar = bar.value(value);
        } else {
            bar = bar.loading(running);
        }
        let cancel_app = cx.entity().clone();
        let close_app = cx.entity().clone();
        let header = h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .child(
                AppIconElement::new(if error.is_some() {
                    AppIcon::Ui(UiIcon::ERROR)
                } else {
                    AppIcon::Ui(UiIcon::DOWNLOAD)
                })
                .xsmall()
                .text_color(if error.is_some() { p.danger } else { p.primary }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(title),
            )
            .when(running, |row| {
                row.child(
                    Button::new("git-clone-task-cancel")
                        .ghost()
                        .xsmall()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).small())
                        .accessibility_label(rust_i18n::t!("git.clone.cancel").to_string())
                        .tooltip(rust_i18n::t!("git.clone.cancel").to_string())
                        .on_click(move |_, _, cx| {
                            cancel_app.update(cx, |this, cx| this.git_cancel_clone_task(cx));
                        }),
                )
            })
            .when(!running && error.is_some(), |row| {
                row.child(
                    Button::new("git-clone-task-close")
                        .ghost()
                        .xsmall()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).small())
                        .accessibility_label(rust_i18n::t!("git.clone.close").to_string())
                        .tooltip(rust_i18n::t!("git.clone.close").to_string())
                        .on_click(move |_, _, cx| {
                            close_app.update(cx, |this, cx| {
                                this.state.git.clone_task = None;
                                cx.notify();
                            });
                        }),
                )
            });
        Some(
            v_flex()
                .absolute()
                .right(px(16.0))
                .bottom(px(40.0))
                .w(px(360.0))
                .id("git-clone-task")
                .occlude()
                .gap_2()
                .p_3()
                .rounded(p.radius.md)
                .bg(p.popover)
                .border_1()
                .border_color(p.border)
                .shadow_2xl()
                .child(header)
                .when(error.is_none(), |el| el.child(bar))
                .child(
                    div()
                        .w_full()
                        .line_clamp(2)
                        .text_xs()
                        .text_color(detail_color)
                        .child(detail),
                )
                .into_any_element(),
        )
    }
}
