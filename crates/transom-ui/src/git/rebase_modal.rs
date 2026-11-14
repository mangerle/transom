//! 交互式变基模态框：把 `<基线>..HEAD` 的提交列成可编辑的待办清单。
//!
//! 设计原理（Why）：
//! - 清单一次取回后全部在界面里编辑（改动作、上下移动），最后一次性提交执行：
//!   逐步执行会让中途失败留下半完成的历史，而一次 `rebase -i` 要么完成、
//!   要么停在冲突处由「中止变基」回退；
//! - 不做 `edit` 动作，也不做拖拽排序：前者需要变基中间态的继续/停止控制，
//!   后者在 gpui 里要引入拖放协议；用「上移/下移」两个按钮达到同样的重排能力，
//!   代价是多一次点击；
//! - 动作按 pick → squash → fixup → drop → reword 循环，与 IDEA 里连点动作格一致。
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitCommand, GitProject, GitRebaseAction, GitRebaseStep, GitRequestId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 交互式变基模态框状态。
pub(crate) struct GitRebaseModal {
    pub project: GitProject,
    /// 基线提交：清单是它之上的提交，基线本身不动。
    pub base: String,
    /// 基线标题，用于对话框副标题，让用户确认「从哪条之后开始改」。
    pub base_subject: String,
    pub steps: Vec<GitRebaseStep>,
    /// 清单在途：取回前不给编辑，避免把空清单当真清单提交。
    pub loading: bool,
    /// 已在执行：执行中禁用按钮，防重复提交同一次变基。
    pub running: bool,
    pub error: Option<String>,
}

impl GitRebaseModal {
    fn new(project: GitProject, base: String, base_subject: String) -> Self {
        Self {
            project,
            base,
            base_subject,
            steps: Vec::new(),
            loading: true,
            running: false,
            error: None,
        }
    }
}

impl AppShell {
    /// 打开「从这里交互式变基」：先取清单，再让用户编辑。
    pub(crate) fn git_open_rebase_modal(
        &mut self,
        project: &GitProject,
        base: &str,
        cx: &mut Context<Self>,
    ) {
        let subject = self
            .state
            .git_workspace_of_project(project.id, cx)
            .and_then(|workspace| {
                let state = &workspace.read(cx).state;
                state
                    .log
                    .entries
                    .iter()
                    .find(|entry| entry.oid == base)
                    .map(|entry| entry.subject.clone())
            })
            .unwrap_or_default();
        let request_id = GitRequestId::new();
        if let Some(workspace) = self.state.git_workspace_of_project(project.id, cx) {
            workspace.update(cx, |workspace, _| {
                workspace.state.pending.push(request_id);
            });
        }
        self.state.modals.git.rebase = Some(GitRebaseModal::new(
            project.clone(),
            base.to_string(),
            subject,
        ));
        self.send_command(GitCommand::RebaseTodo {
            project: project.clone(),
            request_id,
            base: base.to_string(),
        });
        cx.notify();
    }

    /// 清单落地：只接受与当前打开的基线匹配的结果，迟到事件丢弃。
    pub(crate) fn git_apply_rebase_todo(&mut self, base: &str, steps: Vec<GitRebaseStep>) {
        let Some(modal) = self.state.modals.git.rebase.as_mut() else {
            return;
        };
        if modal.base != base {
            return;
        }
        modal.steps = steps;
        modal.loading = false;
    }

    /// 循环切换某一行的动作。
    pub(crate) fn git_rebase_cycle(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(modal) = self.state.modals.git.rebase.as_mut()
            && let Some(step) = modal.steps.get_mut(index)
        {
            step.action = step.action.next();
        }
        cx.notify();
    }

    /// 按提交 OID 循环切换动作，避免重排后索引指向另一条待办。
    pub(crate) fn git_rebase_cycle_by_oid(&mut self, oid: &str, cx: &mut Context<Self>) {
        let index = self
            .state
            .modals
            .git
            .rebase
            .as_ref()
            .and_then(|modal| modal.steps.iter().position(|step| step.oid == oid));
        if let Some(index) = index {
            self.git_rebase_cycle(index, cx);
        }
    }

    /// 按提交 OID 上移/下移一行，避免重排后索引指向另一条待办。
    pub(crate) fn git_rebase_move_by_oid(&mut self, oid: &str, up: bool, cx: &mut Context<Self>) {
        let index = self
            .state
            .modals
            .git
            .rebase
            .as_ref()
            .and_then(|modal| modal.steps.iter().position(|step| step.oid == oid));
        if let Some(index) = index {
            self.git_rebase_move(index, up, cx);
        }
    }

    /// 上移/下移一行：重排待办清单就是重排历史顺序。
    pub(crate) fn git_rebase_move(&mut self, index: usize, up: bool, cx: &mut Context<Self>) {
        let Some(modal) = self.state.modals.git.rebase.as_mut() else {
            return;
        };
        let target = if up {
            index.checked_sub(1)
        } else {
            (index + 1 < modal.steps.len()).then_some(index + 1)
        };
        if let Some(to) = target {
            modal.steps.swap(index, to);
        }
        cx.notify();
    }

    /// 提交执行：把编辑后的清单交给引擎按 `rebase -i` 跑完。
    pub(crate) fn git_rebase_run(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.state.modals.git.rebase.as_mut() else {
            return;
        };
        if modal.steps.is_empty() || modal.running {
            return;
        }
        modal.running = true;
        let project = modal.project.clone();
        let base = modal.base.clone();
        let steps = modal.steps.clone();
        self.send_command(GitCommand::RebaseRun {
            project,
            base,
            steps,
        });
        cx.notify();
    }

    /// 变基失败时把原因写回对话框并解除执行中：对话框不关，用户要能改清单重试。
    pub(crate) fn git_rebase_failed(
        &mut self,
        project_id: transom_core::GitProjectId,
        action: transom_core::GitAction,
        message: String,
    ) {
        if action != transom_core::GitAction::RebaseRun {
            return;
        }
        let matches = self
            .state
            .modals
            .git
            .rebase
            .as_ref()
            .is_some_and(|modal| modal.project.id == project_id);
        if !matches {
            return;
        }
        if let Some(modal) = self.state.modals.git.rebase.as_mut() {
            modal.running = false;
            modal.error = Some(message);
        }
    }

    /// 关闭对话框（取消不改历史）。
    pub(crate) fn git_close_rebase_modal(&mut self, cx: &mut Context<Self>) {
        self.state.modals.git.rebase = None;
        cx.notify();
    }

    /// 渲染交互式变基对话框主体（接入 dialog_host）。
    pub(crate) fn render_git_rebase_dialog_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.state.modals.git.rebase.as_ref() else {
            return div().into_any_element();
        };
        let subtitle = format!(
            "{} · {}",
            modal.base.chars().take(7).collect::<String>(),
            modal.base_subject
        );
        let loading = modal.loading;
        let running = modal.running;
        let error = modal.error.clone();
        let total = modal.steps.len();

        let rows: Vec<AnyElement> = if loading {
            vec![row_hint(rust_i18n::t!("git.rebase.loading").to_string(), p)]
        } else if modal.steps.is_empty() {
            vec![row_hint(rust_i18n::t!("git.rebase.empty").to_string(), p)]
        } else {
            modal
                .steps
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    self.render_rebase_step_row(app, index, total, step, running, p)
                })
                .collect()
        };

        let body = v_flex()
            .w_full()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(subtitle),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("git.rebase.hint")),
            )
            .child(
                v_flex()
                    .max_h(px(360.0))
                    .min_h_0()
                    .gap_0p5()
                    .overflow_y_scrollbar()
                    .children(rows),
            )
            .when(error.is_some(), |el| {
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
                "git-rebase-cancel",
                rust_i18n::t!("git.project.cancel").to_string(),
                crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                    this.git_close_rebase_modal(cx);
                }),
            ))
            .child(
                crate::widgets::dialog_confirm_button(
                    "git-rebase-run",
                    rust_i18n::t!("git.rebase.run").to_string(),
                    None,
                    crate::widgets::DialogConfirmKind::Primary,
                    crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                        this.git_rebase_run(cx);
                    }),
                )
                .disabled(loading || running || total == 0),
            );
        crate::widgets::dialog_body(
            "git-rebase-dialog",
            rust_i18n::t!("git.rebase.title").to_string(),
            None,
            body,
            Some(footer),
        )
    }

    /// 一行待办：上移 / 下移 / 动作 / 短号 / 标题。
    fn render_rebase_step_row(
        &self,
        app: &Entity<AppShell>,
        index: usize,
        total: usize,
        step: &GitRebaseStep,
        running: bool,
        p: &Palette,
    ) -> AnyElement {
        let step_oid = step.oid.clone();
        let up_oid = step.oid.clone();
        let down_oid = step.oid.clone();
        let cycle_oid = step.oid.clone();
        h_flex()
            .id(ElementId::Name(format!("git-rebase-{step_oid}").into()))
            .px_1()
            .py_0p5()
            .gap_1()
            .items_center()
            .rounded(px(4.0))
            .hover(|style| style.bg(p.list_hover))
            .child(
                Button::new(ElementId::Name(format!("git-rebase-up-{step_oid}").into()))
                    .ghost()
                    .xsmall()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::UP)).small())
                    .accessibility_label("Move step up")
                    .disabled(running || index == 0)
                    .on_click({
                        let app = app.clone();
                        move |_, _, cx| {
                            app.update(cx, |this, cx| {
                                this.git_rebase_move_by_oid(&up_oid, true, cx)
                            });
                        }
                    }),
            )
            .child(
                Button::new(ElementId::Name(
                    format!("git-rebase-down-{step_oid}").into(),
                ))
                .ghost()
                .xsmall()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWN)).small())
                .accessibility_label("Move step down")
                .disabled(running || index + 1 >= total)
                .on_click({
                    let app = app.clone();
                    move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            this.git_rebase_move_by_oid(&down_oid, false, cx)
                        });
                    }
                }),
            )
            .child(
                Button::new(ElementId::Name(format!("git-rebase-act-{step_oid}").into()))
                    .ghost()
                    .xsmall()
                    .label(rust_i18n::t!(step.action.i18n_key()).to_string())
                    .when(step.action != GitRebaseAction::Pick, |btn| btn.secondary())
                    .disabled(running)
                    .on_click({
                        let app = app.clone();
                        move |_, _, cx| {
                            app.update(cx, |this, cx| this.git_rebase_cycle_by_oid(&cycle_oid, cx));
                        }
                    }),
            )
            .child(
                div()
                    .w(px(72.0))
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(step.oid.chars().take(7).collect::<String>()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .truncate()
                    .text_sm()
                    .text_color(p.foreground)
                    .child(step.subject.clone()),
            )
            .into_any_element()
    }
}

/// 清单区的一行提示。
fn row_hint(text: String, p: &Palette) -> AnyElement {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .text_color(p.muted_foreground)
        .child(text)
        .into_any_element()
}
