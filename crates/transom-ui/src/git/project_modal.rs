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
//!   主要线索，事后再补一次编辑对话框不如一次填完（与会话表单同一口径）；
//! - 弹窗视图作为独立 GPUI 实体维护表单与目录选择状态，通过 `EventEmitter` 向外
//!   上抛探测、提交与关闭意图，避免与全局外壳状态耦合。

use super::dir_browser::{DirPicker, GitDirBrowserEvent, render_git_browser};
use super::folder_field::{GitFolderFieldProps, render_git_folder_field};
use super::form::GitFolderField;
use crate::keys::expand_home;
use crate::palette::Palette;
use crate::session_appearance::{render_appearance_color_row, render_appearance_icon_row};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{GitRequestId, RepoInfo, SessionColor, SessionIcon};

/// 「打开项目」弹窗对外发出的事件。
#[derive(Debug, Clone)]
pub(crate) enum GitProjectModalEvent {
    /// 路径变化，需要后台探测是否为 Git 仓库。
    Probe {
        request_id: GitRequestId,
        path: PathBuf,
    },
    /// 用户确认添加或初始化并添加项目。
    Commit {
        path: PathBuf,
        folder_name: String,
        icon: SessionIcon,
        color: SessionColor,
        already_repo: bool,
        init_if_missing: bool,
    },
    /// 取消或关闭弹窗。
    Close,
}

/// 「打开项目」独立弹窗实体。
pub(crate) struct GitProjectModalView {
    pub picker: Entity<DirPicker>,
    pub folder: GitFolderField,
    pub folders: Vec<String>,
    pub icon: SessionIcon,
    pub color: SessionColor,
    pub detected: Option<RepoInfo>,
    pub probing: bool,
    pub init_if_missing: bool,
    pub request_id: GitRequestId,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<GitProjectModalEvent> for GitProjectModalView {}

impl GitProjectModalView {
    /// 构建弹窗实体，绑定目录选择器事件并初始化根目录浏览。
    pub(crate) fn new(
        folders: Vec<String>,
        preset_folder: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = rust_i18n::t!("git.project.path_placeholder").to_string();
        let folder_placeholder = rust_i18n::t!("git.project.folder_placeholder").to_string();
        let picker = cx.new(|cx| DirPicker::new(window, cx, &placeholder));
        let mut folder = GitFolderField::new(window, cx, &folder_placeholder);
        if let Some(name) = preset_folder.filter(|name| !name.is_empty()) {
            folder.pick(name, window, cx);
        }

        let picker_sub = cx.subscribe(&picker, |this, _, event: &GitDirBrowserEvent, cx| {
            match event {
                GitDirBrowserEvent::PathSelected(_) => this.trigger_probe(cx),
                GitDirBrowserEvent::CloseTree => cx.notify(),
            }
        });
        let picker_obs = cx.observe(&picker, |_, _, cx| cx.notify());

        let root = picker.read(cx).browser.root.clone();
        picker.update(cx, |p, cx| p.browse_root(root, window, cx));

        Self {
            picker,
            folder,
            folders,
            icon: SessionIcon::default(),
            color: SessionColor::default(),
            detected: None,
            probing: false,
            init_if_missing: false,
            request_id: GitRequestId::new(),
            _subscriptions: vec![picker_sub, picker_obs],
        }
    }

    /// 供焦点管理获取路径输入框焦点句柄。
    pub(crate) fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.read(cx).path.read(cx).focus_handle(cx)
    }

    /// 触发一次目录仓库探测。
    pub(crate) fn trigger_probe(&mut self, cx: &mut Context<Self>) {
        let raw = self.picker.read(cx).path.read(cx).value().trim().to_string();
        if raw.is_empty() {
            return;
        }
        let path = expand_home(&raw);
        let request_id = GitRequestId::new();
        self.request_id = request_id;
        self.probing = true;
        self.detected = None;
        self.picker.update(cx, |p, _| p.error = None);
        cx.emit(GitProjectModalEvent::Probe { request_id, path });
        cx.notify();
    }

    /// 回填后台探测结果。
    pub(crate) fn apply_probe_result(
        &mut self,
        request_id: GitRequestId,
        info: Option<RepoInfo>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.request_id != request_id {
            return;
        }
        self.probing = false;
        self.picker.update(cx, |p, _| p.error = error);
        self.detected = info;
        cx.notify();
    }

    /// 标记正在执行 `git init`。
    pub(crate) fn begin_init(&mut self, request_id: GitRequestId, cx: &mut Context<Self>) {
        self.request_id = request_id;
        self.probing = true;
        self.picker.update(cx, |p, _| p.error = None);
        cx.notify();
    }

    /// 回填 `git init` 失败信息。
    pub(crate) fn apply_init_error(
        &mut self,
        request_id: GitRequestId,
        message: String,
        cx: &mut Context<Self>,
    ) {
        self.probing = false;
        self.request_id = request_id;
        self.picker.update(cx, |p, _| p.error = Some(message));
        cx.notify();
    }

    /// 校验并通过事件上报提交意图。
    pub(crate) fn commit(&mut self, cx: &mut Context<Self>) {
        let path_value = self.picker.read(cx).path.read(cx).value().trim().to_string();
        let has_error = self.picker.read(cx).error.is_some();
        if !can_confirm_project(&path_value, self.probing, has_error) {
            return;
        }
        let path = expand_home(&path_value);
        let folder_name = self.folder.name.read(cx).value().trim().to_string();
        cx.emit(GitProjectModalEvent::Commit {
            path,
            folder_name,
            icon: self.icon,
            color: self.color,
            already_repo: self.detected.is_some(),
            init_if_missing: self.init_if_missing,
        });
    }

    fn status_line(&self, path_value: &str) -> String {
        if let Some(info) = &self.detected {
            format!(
                "{} · {}",
                rust_i18n::t!("git.project.is_repo"),
                info.toplevel.display()
            )
        } else if self.probing {
            rust_i18n::t!("git.project.probing").to_string()
        } else if path_value.is_empty() {
            rust_i18n::t!("git.project.hint").to_string()
        } else {
            rust_i18n::t!("git.project.not_repo").to_string()
        }
    }
}

impl Render for GitProjectModalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let view = cx.entity().clone();
        let picker_state = self.picker.read(cx);
        let error = picker_state.error.clone();
        let path_value = picker_state.path.read(cx).value().trim().to_string();
        let status_line = self.status_line(&path_value);
        let can_confirm = can_confirm_project(&path_value, self.probing, error.is_some());

        let mut body = v_flex()
            .w_full()
            .gap_2()
            .child(render_git_browser(&self.picker, 14, true, &p, cx))
            .child(self.render_appearance_section(&view, &p, cx))
            .child(self.render_folder_and_init_row(&view, &p, status_line, cx));

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
                cx.listener(|_, _: &ClickEvent, _, cx| {
                    cx.emit(GitProjectModalEvent::Close);
                }),
            ))
            .child(
                crate::widgets::dialog_confirm_button(
                    "git-project-confirm",
                    rust_i18n::t!("git.name.confirm").to_string(),
                    None,
                    crate::widgets::DialogConfirmKind::Primary,
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.commit(cx);
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
}

impl GitProjectModalView {
    fn render_appearance_section(
        &self,
        view: &Entity<Self>,
        p: &Palette,
        cx: &App,
    ) -> AnyElement {
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
                        self.icon,
                        move |this: &mut Self, choice, cx| {
                            this.icon = choice;
                            cx.notify();
                        },
                        view,
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
                        self.color,
                        move |this: &mut Self, choice, cx| {
                            this.color = choice;
                            cx.notify();
                        },
                        p,
                        view,
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn render_folder_and_init_row(
        &self,
        view: &Entity<Self>,
        p: &Palette,
        status_line: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .h(px(64.0))
            .gap_6()
            .items_start()
            .child(div().flex_1().min_w_0().child(render_git_folder_field(
                &self.folder,
                GitFolderFieldProps {
                    options: &self.folders,
                    fixed_width: None,
                    palette: p,
                    entity: view,
                    access: |this| &mut this.folder,
                },
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
                                    .checked(self.init_if_missing)
                                    .disabled(self.detected.is_some())
                                    .on_click(cx.listener(|this, &checked, _, cx| {
                                        this.init_if_missing = checked;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .child(rust_i18n::t!("git.project.init_label").to_string()),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .text_xs()
                            .text_color(if self.detected.is_some() {
                                p.success
                            } else {
                                p.muted_foreground
                            })
                            .line_clamp(2)
                            .child(status_line),
                    ),
            )
            .into_any_element()
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
