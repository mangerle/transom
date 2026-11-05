//! 容器文件管理居中模态弹窗（重命名与新建目录）。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::Input;
use gpui_kit::*;

impl AppShell {
    /// 渲染容器文件重命名模态弹窗主体。
    pub(crate) fn render_container_file_rename_dialog_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        _p: &Palette,
    ) -> AnyElement {
        let Some((tab_id, workspace)) =
            self.find_container_modal_tab(|ws| ws.files.rename_input.is_some())
        else {
            return div().into_any_element();
        };
        let Some(input) = workspace.files.rename_input.clone() else {
            return div().into_any_element();
        };

        let content = v_flex().gap_3().child(Input::new(&input).w_full());

        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-container-file-rename",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                this.cancel_container_file_rename(tab_id, cx);
            }),
        );

        let confirm = crate::widgets::dialog_confirm_button(
            "confirm-container-file-rename",
            rust_i18n::t!("common.confirm").to_string(),
            Some(IconName::Check),
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, window, cx| {
                this.submit_container_file_rename(tab_id, window, cx);
            }),
        );

        let footer =
            crate::widgets::dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

        crate::widgets::dialog_body(
            "container-file-rename-modal",
            rust_i18n::t!("container.files.rename_modal_title").to_string(),
            Some(rust_i18n::t!("container.files.rename_prompt").to_string()),
            content,
            Some(footer),
        )
    }

    /// 渲染容器新建目录模态弹窗主体。
    pub(crate) fn render_container_file_new_dir_dialog_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        _p: &Palette,
    ) -> AnyElement {
        let Some((tab_id, workspace)) = self.find_container_modal_tab(|ws| {
            ws.files.new_dir_open && ws.files.new_dir_input.is_some()
        }) else {
            return div().into_any_element();
        };
        let Some(input) = workspace.files.new_dir_input.clone() else {
            return div().into_any_element();
        };

        let content = v_flex().gap_3().child(Input::new(&input).w_full());

        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-container-file-mkdir",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                this.cancel_container_file_new_dir(tab_id, cx);
            }),
        );

        let confirm = crate::widgets::dialog_confirm_button(
            "confirm-container-file-mkdir",
            rust_i18n::t!("container.create").to_string(),
            Some(IconName::Plus),
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, window, cx| {
                this.submit_container_mkdir(tab_id, window, cx);
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
