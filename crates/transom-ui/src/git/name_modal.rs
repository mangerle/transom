//! Git 名称输入对话框：新建分支 / 新建标签 / 重命名分支 / 新建与重命名分组。
//!
//! 设计原理（Why）：
//! - 一个对话框服务多种目的（[`GitNamePurpose`]），差别只在标题、提示、默认值与
//!   提交后落到哪条命令上，避免几份几乎一样的输入框代码各自漂移；
//! - 名字合法性在引擎边界再校验一次（见 `transom-git::ops::check_ref_name`）：
//!   界面只做非空提示，真正的规则以 git 的口径为准，两处规则不会分叉；
//! - 分组名只是界面里的一个标签，不落给 git，所以它的目的分支不带项目上下文，
//!   `project` 因此是 `Option`；
//! - 本模块只维护实体自身输入态与 UI 渲染，提交或关闭通过 [`GitNameModalEvent`]
//!   上报外壳桥接层，避免弹窗视图直接依赖 `AppShell`。

use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitFolderId, GitProject};

/// 名称对话框的目的：决定标题、提示与提交后落到哪条命令。
#[derive(Clone)]
pub(crate) enum GitNamePurpose {
    /// 从某起点新建分支（`start` 为空表示当前头）。
    CreateBranch { start: Option<String> },
    /// 在某提交上新建标签。
    CreateTag { target: String },
    /// 重命名已有分支。
    RenameBranch { old: String },
    /// 新建侧栏分组。
    CreateFolder,
    /// 重命名已有分组。
    RenameFolder { id: GitFolderId },
}

/// 名称对话框向上层发送的事件。
pub(crate) enum GitNameModalEvent {
    Submit {
        project: Option<GitProject>,
        purpose: GitNamePurpose,
        name: String,
    },
    Close,
}

/// 名称对话框实体视图。
pub(crate) struct GitNameModalView {
    /// 上下文项目；分组相关目的不需要项目，故为 `Option`。
    pub project: Option<GitProject>,
    pub purpose: GitNamePurpose,
    pub input: Entity<InputState>,
    pub error: Option<String>,
}

impl EventEmitter<GitNameModalEvent> for GitNameModalView {}

impl Render for GitNameModalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.render_body(cx, &p)
    }
}

impl GitNameModalView {
    pub(crate) fn new(
        project: Option<GitProject>,
        purpose: GitNamePurpose,
        default: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(default));
        Self {
            project,
            purpose,
            input,
            error: None,
        }
    }

    /// 对话框标题文案键。
    fn title_key(&self) -> &'static str {
        match self.purpose {
            GitNamePurpose::CreateBranch { .. } => "git.name.branch",
            GitNamePurpose::CreateTag { .. } => "git.name.tag",
            GitNamePurpose::RenameBranch { .. } => "git.name.rename",
            GitNamePurpose::CreateFolder => "git.folder.new_title",
            GitNamePurpose::RenameFolder { .. } => "git.folder.rename_title",
        }
    }

    /// 输入框下方的提示文案键：分支/标签受 git 命名规则约束，分组名只是自己的标签，
    /// 两者混用一句提示会让人以为分组名也不能带某些字符。
    fn hint_key(&self) -> &'static str {
        match self.purpose {
            GitNamePurpose::CreateFolder | GitNamePurpose::RenameFolder { .. } => {
                "git.folder.name_hint"
            }
            _ => "git.name.hint",
        }
    }

    /// 提交：非空即发出 Submit 事件，具体名字合法性由引擎边界判定。
    pub(crate) fn submit(&mut self, cx: &mut Context<Self>) {
        let name = self.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.error = Some(rust_i18n::t!("git.name.need_name").to_string());
            cx.notify();
            return;
        }
        self.error = None;
        cx.emit(GitNameModalEvent::Submit {
            project: self.project.clone(),
            purpose: self.purpose.clone(),
            name,
        });
    }

    fn render_body(&mut self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let title = rust_i18n::t!(self.title_key()).to_string();
        let hint = rust_i18n::t!(self.hint_key()).to_string();
        let input_state = self.input.clone();
        let error = self.error.clone();
        let body = v_flex()
            .w_full()
            .gap_2()
            .child(crate::widgets::form_field(
                rust_i18n::t!("git.name.label").to_string(),
                input_state,
                p,
            ))
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
                "git-name-cancel",
                rust_i18n::t!("git.project.cancel").to_string(),
                cx.listener(|_, _: &ClickEvent, _, cx| {
                    cx.emit(GitNameModalEvent::Close);
                }),
            ))
            .child(crate::widgets::dialog_confirm_button(
                "git-name-confirm",
                rust_i18n::t!("git.name.confirm").to_string(),
                None,
                crate::widgets::DialogConfirmKind::Primary,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.submit(cx);
                }),
            ));
        crate::widgets::dialog_body("git-name-dialog", title, Some(hint), body, Some(footer))
    }
}
