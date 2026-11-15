//! 三方合并冲突解决器模态框视图实体（3-Way Merge Resolver）。
//!
//! 设计原理（Why）：
//! - 当拉取、合并或变基产生冲突时，对齐 IntelliJ IDEA / openIDEA 标准提供直观的三方对比解决界面；
//! - 左栏展示本地版本（Ours），中栏展示当前工作区合并结果（Result），右栏展示传入版本（Theirs）；
//! - 实体化后文件切换、三方内容展示与解决状态均在本实体内部闭环，不再经由外壳转发。

mod panels;

use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::*;
use transom_core::{ConflictSides, GitProject, GitProjectId};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

/// 冲突解决弹窗发往外壳的意图事件。
pub(crate) enum GitConflictModalEvent {
    /// 请求后台读取指定冲突文件的三方内容。
    LoadSides { project: GitProject, path: String },
    /// 请求后台接受本地版本。
    ResolveOurs { project: GitProject, path: String },
    /// 请求后台接受传入版本。
    ResolveTheirs { project: GitProject, path: String },
    /// 请求后台标记为已解决。
    MarkResolved { project: GitProject, path: String },
    /// 请求关闭弹窗。
    Close,
}

/// 冲突解决模态框视图实体。
pub(crate) struct GitConflictModalView {
    pub project: GitProject,
    /// 冲突文件相对路径清单。
    pub files: Vec<String>,
    /// 当前选中的冲突文件索引。
    pub selected_index: usize,
    /// 当前选中文件的三方内容视图。
    pub sides: Option<ConflictSides>,
    /// 正在读取三方内容。
    pub loading: bool,
    /// 正在执行解决写入动作。
    pub resolving: bool,
    /// 错误提示。
    pub error: Option<String>,
}

impl EventEmitter<GitConflictModalEvent> for GitConflictModalView {}

impl Render for GitConflictModalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.render_body(cx, &p)
    }
}

impl GitConflictModalView {
    pub(crate) fn new(project: GitProject, files: Vec<String>) -> Self {
        Self {
            project,
            files,
            selected_index: 0,
            sides: None,
            loading: true,
            resolving: false,
            error: None,
        }
    }

    /// 获取当前选中的文件路径。
    pub(crate) fn active_file(&self) -> Option<&str> {
        self.files.get(self.selected_index).map(|s| s.as_str())
    }

    /// 后台三方内容返回时落盘到当前实体。
    pub(crate) fn apply_sides(
        &mut self,
        project_id: GitProjectId,
        path: &str,
        sides: ConflictSides,
        cx: &mut Context<Self>,
    ) {
        if self.project.id != project_id || self.active_file() != Some(path) {
            return;
        }
        self.sides = Some(sides);
        self.loading = false;
        cx.notify();
    }

    /// 处理一次冲突解决写操作完成事件；若全部文件解决则请求关闭。
    pub(crate) fn handle_resolve_done(
        &mut self,
        error_message: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.resolving = false;
        if let Some(err) = error_message {
            self.error = Some(err);
            cx.notify();
            return;
        }
        if self.selected_index < self.files.len() {
            self.files.remove(self.selected_index);
        }
        if self.files.is_empty() {
            cx.emit(GitConflictModalEvent::Close);
            return;
        }
        self.selected_index = self.selected_index.min(self.files.len() - 1);
        self.loading = true;
        self.sides = None;
        let next_file = self.files[self.selected_index].clone();
        cx.emit(GitConflictModalEvent::LoadSides {
            project: self.project.clone(),
            path: next_file,
        });
        cx.notify();
    }

    /// 切换当前选中的冲突文件并拉取其三方内容。
    pub(super) fn select_file(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.files.len() || self.selected_index == index {
            return;
        }
        self.selected_index = index;
        self.loading = true;
        self.error = None;
        let file = self.files[index].clone();
        cx.emit(GitConflictModalEvent::LoadSides {
            project: self.project.clone(),
            path: file,
        });
        cx.notify();
    }

    fn resolve_ours(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.active_file().map(|s| s.to_string()) else {
            return;
        };
        self.resolving = true;
        cx.emit(GitConflictModalEvent::ResolveOurs {
            project: self.project.clone(),
            path,
        });
        cx.notify();
    }

    fn resolve_theirs(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.active_file().map(|s| s.to_string()) else {
            return;
        };
        self.resolving = true;
        cx.emit(GitConflictModalEvent::ResolveTheirs {
            project: self.project.clone(),
            path,
        });
        cx.notify();
    }

    fn mark_resolved(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.active_file().map(|s| s.to_string()) else {
            return;
        };
        self.resolving = true;
        cx.emit(GitConflictModalEvent::MarkResolved {
            project: self.project.clone(),
            path,
        });
        cx.notify();
    }

    fn render_body(&mut self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let file_count = self.files.len();
        let selected_idx = self.selected_index;

        let header = render_conflict_header(self, file_count, selected_idx, p);
        let toolbar = render_conflict_toolbar(self, cx, p);
        let body = panels::render_conflict_body(self, cx, p);
        let footer = render_conflict_footer(cx, p);

        v_flex()
            .w_full()
            .h(px(520.0))
            .bg(p.background)
            .child(header)
            .child(toolbar)
            .child(body)
            .child(footer)
            .into_any_element()
    }
}

fn render_conflict_header(
    modal: &GitConflictModalView,
    file_count: usize,
    selected_idx: usize,
    p: &Palette,
) -> Div {
    h_flex()
        .w_full()
        .h(px(32.0))
        .flex_none()
        .px_3()
        .items_center()
        .justify_between()
        .bg(p.list_head)
        .border_b_1()
        .border_color(p.divider(p.border))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    AppIconElement::new(AppIcon::Ui(UiIcon::MERGE))
                        .small()
                        .text_color(p.warning),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(rust_i18n::t!("git.conflict.title").to_string()),
                )
                .child(div().text_xs().text_color(p.muted).child(format!(
                    "({}/{})",
                    selected_idx + 1,
                    file_count
                ))),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted)
                .child(modal.active_file().unwrap_or("").to_string()),
        )
}

fn render_conflict_toolbar(
    modal: &GitConflictModalView,
    cx: &mut Context<GitConflictModalView>,
    p: &Palette,
) -> Div {
    let is_busy = modal.resolving || modal.loading;

    h_flex()
        .w_full()
        .h(px(32.0))
        .flex_none()
        .px_3()
        .gap_2()
        .items_center()
        .justify_between()
        .bg(p.background)
        .border_b_1()
        .border_color(p.divider(p.border))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    Button::new("btn-resolve-ours")
                        .outline()
                        .xsmall()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWN)).small())
                        .label(rust_i18n::t!("git.conflict.accept_ours").to_string())
                        .disabled(is_busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.resolve_ours(cx);
                        })),
                )
                .child(
                    Button::new("btn-resolve-theirs")
                        .outline()
                        .xsmall()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWN)).small())
                        .label(rust_i18n::t!("git.conflict.accept_theirs").to_string())
                        .disabled(is_busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.resolve_theirs(cx);
                        })),
                )
                .child(
                    Button::new("btn-mark-resolved")
                        .ghost()
                        .xsmall()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHECK)).small())
                        .label(rust_i18n::t!("git.conflict.mark_resolved").to_string())
                        .disabled(is_busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.mark_resolved(cx);
                        })),
                ),
        )
        .child(if modal.resolving {
            div()
                .text_xs()
                .text_color(p.primary)
                .child(rust_i18n::t!("git.conflict.resolving").to_string())
        } else {
            div()
        })
}

fn render_conflict_footer(cx: &mut Context<GitConflictModalView>, p: &Palette) -> Div {
    h_flex()
        .w_full()
        .h(px(36.0))
        .flex_none()
        .px_3()
        .items_center()
        .justify_between()
        .bg(p.list_head)
        .border_t_1()
        .border_color(p.divider(p.border))
        .child(
            div()
                .text_xs()
                .text_color(p.muted)
                .child(rust_i18n::t!("git.conflict.footer_tip").to_string()),
        )
        .child(
            Button::new("btn-conflict-close")
                .outline()
                .small()
                .label(rust_i18n::t!("common.close").to_string())
                .on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(GitConflictModalEvent::Close);
                })),
        )
}
