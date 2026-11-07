//! 三方合并冲突解决器模态框（3-Way Merge Resolver）。
//!
//! 设计原理（Why）：
//! - 当拉取、合并或变基产生冲突时，对齐 IntelliJ IDEA / openIDEA 标准提供直观的三方对比解决界面；
//! - 左栏展示本地版本（Ours），中栏展示当前工作区合并结果（Result），右栏展示传入版本（Theirs）；
//! - 用户可针对选中文件一键「接受本地版本 (Ours)」、「接受传入版本 (Theirs)」或在外部编辑后「标记为已解决」。

mod panels;

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::*;
use transom_core::{ConflictSides, GitCommand, GitProject, GitRequestId};

/// 冲突解决模态框状态。
pub(crate) struct GitConflictModal {
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

impl GitConflictModal {
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
}

impl AppShell {
    /// 打开冲突解决模态框。
    pub(crate) fn open_git_conflict_modal(&mut self, project: &GitProject, cx: &mut Context<Self>) {
        let conflict_files: Vec<String> = self
            .git_state_of(project.id)
            .and_then(|state| state.status.as_ref())
            .map(|status| {
                status
                    .conflicts()
                    .into_iter()
                    .map(|e| e.path.clone())
                    .collect()
            })
            .unwrap_or_default();

        if conflict_files.is_empty() {
            return;
        }

        let first_file = conflict_files[0].clone();
        self.state.modals.git.conflict =
            Some(GitConflictModal::new(project.clone(), conflict_files));

        let request_id = GitRequestId::new();
        self.send_command(GitCommand::LoadConflictSides {
            project: project.clone(),
            request_id,
            path: first_file,
        });
        cx.notify();
    }

    /// 关闭冲突解决模态框。
    pub(crate) fn close_git_conflict_modal(&mut self, cx: &mut Context<Self>) {
        self.state.modals.git.conflict = None;
        cx.notify();
    }

    /// 切换当前选中的冲突文件并拉取其三方内容。
    pub(crate) fn git_conflict_select_file(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(modal) = self.state.modals.git.conflict.as_mut() else {
            return;
        };
        if index >= modal.files.len() || modal.selected_index == index {
            return;
        }
        modal.selected_index = index;
        modal.loading = true;
        modal.error = None;
        let file = modal.files[index].clone();
        let project = modal.project.clone();
        let request_id = GitRequestId::new();
        self.send_command(GitCommand::LoadConflictSides {
            project,
            request_id,
            path: file,
        });
        cx.notify();
    }

    /// 解决冲突：接受本地/当前版本。
    pub(crate) fn git_conflict_resolve_ours(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.state.modals.git.conflict.as_mut() else {
            return;
        };
        let Some(path) = modal.active_file().map(|s| s.to_string()) else {
            return;
        };
        modal.resolving = true;
        let project = modal.project.clone();
        self.send_command(GitCommand::ResolveConflictOurs {
            project,
            paths: vec![path],
        });
        cx.notify();
    }

    /// 解决冲突：接受传入版本。
    pub(crate) fn git_conflict_resolve_theirs(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.state.modals.git.conflict.as_mut() else {
            return;
        };
        let Some(path) = modal.active_file().map(|s| s.to_string()) else {
            return;
        };
        modal.resolving = true;
        let project = modal.project.clone();
        self.send_command(GitCommand::ResolveConflictTheirs {
            project,
            paths: vec![path],
        });
        cx.notify();
    }

    /// 解决冲突：标记为已手动解决。
    pub(crate) fn git_conflict_mark_resolved(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.state.modals.git.conflict.as_mut() else {
            return;
        };
        let Some(path) = modal.active_file().map(|s| s.to_string()) else {
            return;
        };
        modal.resolving = true;
        let project = modal.project.clone();
        self.send_command(GitCommand::ResolveConflictMark {
            project,
            paths: vec![path],
        });
        cx.notify();
    }

    /// 渲染冲突解决弹窗主体。
    pub(crate) fn render_git_conflict_dialog_body(
        &self,
        app: &Entity<Self>,
        _window: &mut Window,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.state.modals.git.conflict.as_ref() else {
            return div().into_any_element();
        };

        let file_count = modal.files.len();
        let selected_idx = modal.selected_index;

        let header = self.render_conflict_header(modal, file_count, selected_idx, p);
        let toolbar = self.render_conflict_toolbar(modal, app, p);
        let body = self.render_conflict_body(modal, app, p);
        let footer = self.render_conflict_footer(app, p);

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

    fn render_conflict_header(
        &self,
        modal: &GitConflictModal,
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
                    .child(Icon::new(IconName::GitMerge).small().text_color(p.warning))
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
        &self,
        modal: &GitConflictModal,
        app: &Entity<Self>,
        p: &Palette,
    ) -> Div {
        let app_ours = app.clone();
        let app_theirs = app.clone();
        let app_mark = app.clone();
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
                            .icon(IconName::ArrowDownLeft)
                            .label(rust_i18n::t!("git.conflict.accept_ours").to_string())
                            .disabled(is_busy)
                            .on_click(move |_, _, cx| {
                                app_ours.update(cx, |this, cx| this.git_conflict_resolve_ours(cx));
                            }),
                    )
                    .child(
                        Button::new("btn-resolve-theirs")
                            .outline()
                            .xsmall()
                            .icon(IconName::ArrowDownRight)
                            .label(rust_i18n::t!("git.conflict.accept_theirs").to_string())
                            .disabled(is_busy)
                            .on_click(move |_, _, cx| {
                                app_theirs
                                    .update(cx, |this, cx| this.git_conflict_resolve_theirs(cx));
                            }),
                    )
                    .child(
                        Button::new("btn-mark-resolved")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Check)
                            .label(rust_i18n::t!("git.conflict.mark_resolved").to_string())
                            .disabled(is_busy)
                            .on_click(move |_, _, cx| {
                                app_mark.update(cx, |this, cx| this.git_conflict_mark_resolved(cx));
                            }),
                    ),
            )
            .child(
                modal
                    .resolving
                    .then(|| {
                        div()
                            .text_xs()
                            .text_color(p.primary)
                            .child(rust_i18n::t!("git.conflict.resolving").to_string())
                    })
                    .unwrap_or_else(div),
            )
    }

    fn render_conflict_footer(&self, app: &Entity<Self>, p: &Palette) -> Div {
        let app_close = app.clone();

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
                    .on_click(move |_, _, cx| {
                        app_close.update(cx, |this, cx| this.close_git_conflict_modal(cx));
                    }),
            )
    }
}
