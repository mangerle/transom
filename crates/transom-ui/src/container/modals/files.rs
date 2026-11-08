//! 容器文件管理居中模态弹窗（重命名与新建目录）。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::Input;
use gpui_kit::*;

impl AppShell {
    /// 渲染容器文件重命名模态弹窗主体。
    ///
    /// 弹窗本体仍由外壳按统一模态栈组装；输入框与提交/取消动作属于工作区实体，
    /// 回调直接 `update` 找到的实体，不再回交外壳方法。
    pub(crate) fn render_container_file_rename_dialog_body(
        &self,
        _app: &Entity<AppShell>,
        cx: &App,
        _p: &Palette,
    ) -> AnyElement {
        let Some((_, workspace)) =
            self.find_container_modal_tab(cx, |ws| ws.files.prompt_input.is_some())
        else {
            return div().into_any_element();
        };
        let Some(input) = workspace.read(cx).state.files.prompt_input.clone() else {
            return div().into_any_element();
        };
        let Some(prompt) = workspace.read(cx).state.files.prompt.clone() else {
            return div().into_any_element();
        };

        let content = v_flex().gap_3().child(Input::new(&input).w_full());

        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-container-file-rename",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(&workspace, move |workspace, _: &ClickEvent, _, cx| {
                workspace.cancel_container_file_rename(cx);
            }),
        );

        let confirm = crate::widgets::dialog_confirm_button(
            "confirm-container-file-rename",
            rust_i18n::t!("common.confirm").to_string(),
            Some(IconName::Check),
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(&workspace, move |workspace, _: &ClickEvent, _, cx| {
                workspace.submit_container_file_prompt(cx);
            }),
        );

        let footer =
            crate::widgets::dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

        let title = rust_i18n::t!(prompt.title_key()).to_string();
        let prompt_text = rust_i18n::t!(prompt.prompt_key()).to_string();
        crate::widgets::dialog_body(
            "container-file-rename-modal",
            title,
            Some(prompt_text),
            content,
            Some(footer),
        )
    }

    /// 渲染容器新建目录模态弹窗主体。
    pub(crate) fn render_container_file_new_dir_dialog_body(
        &self,
        _app: &Entity<AppShell>,
        cx: &App,
        _p: &Palette,
    ) -> AnyElement {
        let Some((_, workspace)) = self.find_container_modal_tab(cx, |ws| {
            ws.files.new_dir_open && ws.files.new_dir_input.is_some()
        }) else {
            return div().into_any_element();
        };
        let Some(input) = workspace.read(cx).state.files.new_dir_input.clone() else {
            return div().into_any_element();
        };

        let content = v_flex().gap_3().child(Input::new(&input).w_full());

        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-container-file-mkdir",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(&workspace, move |workspace, _: &ClickEvent, _, cx| {
                workspace.cancel_container_file_new_dir(cx);
            }),
        );

        let confirm = crate::widgets::dialog_confirm_button(
            "confirm-container-file-mkdir",
            rust_i18n::t!("container.create").to_string(),
            Some(IconName::Plus),
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(&workspace, move |workspace, _: &ClickEvent, _, cx| {
                workspace.submit_container_mkdir(cx);
            }),
        );

        let footer =
            crate::widgets::dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

        crate::widgets::dialog_body(
            "container-file-mkdir-modal",
            rust_i18n::t!("container.files.mkdir_modal_title").to_string(),
            Some(rust_i18n::t!("container.files.mkdir_prompt").to_string()),
            content,
            Some(footer),
        )
    }
}
