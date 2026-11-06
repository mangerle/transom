//! Git 名称输入对话框：新建分支 / 新建标签 / 重命名分支 / 新建与重命名分组。
//!
//! 设计原理（Why）：
//! - 之前这几类动作由界面自动生成名字（`branch-<短号>`、`tag-<短号>`），用户点完
//!   只得到一个没法读的分支名。IDEA 的对应菜单项带省略号，就是要点出输入框的；
//! - 一个对话框服务多种目的（[`GitNamePurpose`]），差别只在标题、提示、默认值与
//!   提交后落到哪条命令上，避免几份几乎一样的输入框代码各自漂移；
//! - 名字合法性在引擎边界再校验一次（见 `transom-git::ops::check_ref_name`）：
//!   界面只做非空提示，真正的规则以 git 的口径为准，两处规则不会分叉；
//! - 分组名只是界面里的一个标签，不落给 git，所以它的目的分支不带项目上下文，
//!   `project` 因此是 `Option`。
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitCommand, GitFolderId, GitProject};

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

/// 名称对话框状态。
pub(crate) struct GitNameModal {
    /// 上下文项目；分组相关目的不需要项目，故为 `Option`。
    pub project: Option<GitProject>,
    pub purpose: GitNamePurpose,
    pub input: Entity<InputState>,
    pub error: Option<String>,
}

impl GitNameModal {
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
}

impl AppShell {
    /// 打开名称对话框；`default` 是预填的建议名（用户可以改）。
    pub(crate) fn open_git_name_modal(
        &mut self,
        project: &GitProject,
        purpose: GitNamePurpose,
        default: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_name_modal_with(Some(project.clone()), purpose, default, window, cx);
    }

    /// 打开「新建分组」对话框。
    pub(crate) fn open_git_folder_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_name_modal_with(
            None,
            GitNamePurpose::CreateFolder,
            String::new(),
            window,
            cx,
        );
    }

    /// 打开「重命名分组」对话框，预填当前组名。
    pub(crate) fn open_git_rename_folder_modal(
        &mut self,
        id: GitFolderId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self
            .state
            .git
            .folders
            .iter()
            .find(|folder| folder.id == id)
            .map(|folder| folder.name.clone())
        else {
            return;
        };
        self.open_name_modal_with(
            None,
            GitNamePurpose::RenameFolder { id },
            current,
            window,
            cx,
        );
    }

    /// 三个入口的共同实现。
    fn open_name_modal_with(
        &mut self,
        project: Option<GitProject>,
        purpose: GitNamePurpose,
        default: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(&default));
        self.state.modals.git.name = Some(GitNameModal {
            project,
            purpose,
            input,
            error: None,
        });
        cx.notify();
    }

    /// 提交：非空即发命令，具体名字合法性由引擎边界判定并把原因回传界面。
    pub(crate) fn submit_git_name(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.state.modals.git.name.as_ref() else {
            return;
        };
        let name = modal.input.read(cx).value().trim().to_string();
        let project = modal.project.clone();
        let purpose = modal.purpose.clone();
        if name.is_empty() {
            if let Some(modal) = self.state.modals.git.name.as_mut() {
                modal.error = Some(rust_i18n::t!("git.name.need_name").to_string());
            }
            cx.notify();
            return;
        }
        self.state.modals.git.name = None;
        match purpose {
            GitNamePurpose::CreateBranch { start } => {
                let Some(project) = project else {
                    return;
                };
                self.send_command(GitCommand::CreateBranch {
                    project,
                    name,
                    start,
                    switch_to: true,
                });
            }
            GitNamePurpose::CreateTag { target } => {
                let Some(project) = project else {
                    return;
                };
                self.send_command(GitCommand::CreateTag {
                    project,
                    name,
                    target,
                    message: None,
                });
            }
            GitNamePurpose::RenameBranch { old } => {
                let Some(project) = project else {
                    return;
                };
                self.send_command(GitCommand::RenameBranch {
                    project,
                    old,
                    new: name,
                });
            }
            GitNamePurpose::CreateFolder => self.add_git_folder(name, cx),
            GitNamePurpose::RenameFolder { id } => self.rename_git_folder(id, name, cx),
        }
        cx.notify();
    }

    /// 关闭名称录入对话框。
    pub(crate) fn close_git_name_modal(&mut self, cx: &mut Context<Self>) {
        self.state.modals.git.name = None;
        cx.notify();
    }

    /// 渲染名称对话框主体（接入 dialog_host）。
    pub(crate) fn render_git_name_dialog_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.state.modals.git.name.as_ref() else {
            return div().into_any_element();
        };
        let title = rust_i18n::t!(modal.title_key()).to_string();
        let hint = rust_i18n::t!(modal.hint_key()).to_string();
        let input_state = modal.input.clone();
        let error = modal.error.clone();
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
                crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                    this.close_git_name_modal(cx);
                }),
            ))
            .child(crate::widgets::dialog_confirm_button(
                "git-name-confirm",
                rust_i18n::t!("git.name.confirm").to_string(),
                None,
                crate::widgets::DialogConfirmKind::Primary,
                crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                    this.submit_git_name(cx);
                }),
            ));
        crate::widgets::dialog_body("git-name-dialog", title, Some(hint), body, Some(footer))
    }
}
