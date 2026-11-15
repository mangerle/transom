//! 交互式变基模态框视图实体：把 `<基线>..HEAD` 的提交列成可编辑的待办清单。
//!
//! 设计原理（Why）：
//! - 清单一次取回后全部在界面里编辑（改动作、上下移动），最后一次性提交执行：
//!   逐步执行会让中途失败留下半完成的历史，而一次 `rebase -i` 要么完成、
//!   要么停在冲突处由「中止变基」回退；
//! - 动作按 pick → squash → fixup → drop → reword 循环，与 IDEA 里连点动作格一致；
//! - 实体化后列表重排、动作切换与执行状态完全在本实体内部闭环，不再经由外壳转发。

use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitAction, GitProject, GitProjectId, GitRebaseAction, GitRebaseStep};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

/// 交互式变基弹窗发往外壳的意图事件。
pub(crate) enum GitRebaseModalEvent {
    /// 提交执行变基清单。
    Run {
        project: GitProject,
        base: String,
        steps: Vec<GitRebaseStep>,
    },
    /// 请求关闭弹窗。
    Close,
}

/// 交互式变基模态框视图实体。
pub(crate) struct GitRebaseModalView {
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

impl EventEmitter<GitRebaseModalEvent> for GitRebaseModalView {}

impl Render for GitRebaseModalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.render_body(cx, &p)
    }
}

impl GitRebaseModalView {
    pub(crate) fn new(project: GitProject, base: String, base_subject: String) -> Self {
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

    /// 清单落地：只接受与当前打开的基线匹配的结果，迟到事件丢弃。
    pub(crate) fn apply_todo(
        &mut self,
        base: &str,
        steps: Vec<GitRebaseStep>,
        cx: &mut Context<Self>,
    ) {
        if self.base != base {
            return;
        }
        self.steps = steps;
        self.loading = false;
        cx.notify();
    }

    /// 变基失败时把原因写回对话框并解除执行中：对话框不关，用户要能改清单重试。
    pub(crate) fn set_failed(
        &mut self,
        project_id: GitProjectId,
        action: GitAction,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if action != GitAction::RebaseRun || self.project.id != project_id {
            return;
        }
        self.running = false;
        self.error = Some(message);
        cx.notify();
    }

    /// 提交执行：把编辑后的清单交给外壳转发引擎跑完。
    pub(crate) fn run(&mut self, cx: &mut Context<Self>) {
        if self.steps.is_empty() || self.running {
            return;
        }
        self.running = true;
        cx.emit(GitRebaseModalEvent::Run {
            project: self.project.clone(),
            base: self.base.clone(),
            steps: self.steps.clone(),
        });
        cx.notify();
    }

    /// 按提交 OID 循环切换动作，避免重排后索引指向另一条待办。
    fn cycle_by_oid(&mut self, oid: &str, cx: &mut Context<Self>) {
        if let Some(step) = self.steps.iter_mut().find(|step| step.oid == oid) {
            step.action = step.action.next();
            cx.notify();
        }
    }

    /// 按提交 OID 上移/下移一行，避免重排后索引指向另一条待办。
    fn move_by_oid(&mut self, oid: &str, up: bool, cx: &mut Context<Self>) {
        let Some(index) = self.steps.iter().position(|step| step.oid == oid) else {
            return;
        };
        let target = if up {
            index.checked_sub(1)
        } else {
            (index + 1 < self.steps.len()).then_some(index + 1)
        };
        if let Some(to) = target {
            self.steps.swap(index, to);
            cx.notify();
        }
    }

    fn render_body(&mut self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let subtitle = format!(
            "{} · {}",
            self.base.chars().take(7).collect::<String>(),
            self.base_subject
        );
        let loading = self.loading;
        let running = self.running;
        let error = self.error.clone();
        let total = self.steps.len();

        let rows: Vec<AnyElement> = if loading {
            vec![row_hint(rust_i18n::t!("git.rebase.loading").to_string(), p)]
        } else if self.steps.is_empty() {
            vec![row_hint(rust_i18n::t!("git.rebase.empty").to_string(), p)]
        } else {
            self.steps
                .iter()
                .enumerate()
                .map(|(index, step)| render_rebase_step_row(cx, index, total, step, running, p))
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
                cx.listener(|_, _: &ClickEvent, _, cx| {
                    cx.emit(GitRebaseModalEvent::Close);
                }),
            ))
            .child(
                crate::widgets::dialog_confirm_button(
                    "git-rebase-run",
                    rust_i18n::t!("git.rebase.run").to_string(),
                    None,
                    crate::widgets::DialogConfirmKind::Primary,
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.run(cx);
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
}

/// 一行待办：上移 / 下移 / 动作 / 短号 / 标题。
fn render_rebase_step_row(
    cx: &mut Context<GitRebaseModalView>,
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
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.move_by_oid(&up_oid, true, cx);
                })),
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
            .on_click(cx.listener(move |this, _, _, cx| {
                this.move_by_oid(&down_oid, false, cx);
            })),
        )
        .child(
            Button::new(ElementId::Name(format!("git-rebase-act-{step_oid}").into()))
                .ghost()
                .xsmall()
                .label(rust_i18n::t!(step.action.i18n_key()).to_string())
                .when(step.action != GitRebaseAction::Pick, |btn| btn.secondary())
                .disabled(running)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.cycle_by_oid(&cycle_oid, cx);
                })),
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
