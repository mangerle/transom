//! 容器重命名居中模态弹窗。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::Input;
use gpui_kit::*;

impl AppShell {
    /// 渲染容器重命名模态弹窗主体。
    pub(crate) fn render_container_rename_dialog_body(
        &self,
        _app: &Entity<AppShell>,
        cx: &App,
        _p: &Palette,
    ) -> AnyElement {
        let Some((_tab_id, workspace)) =
            self.find_container_modal_tab(cx, |ws| ws.rename_container.is_some())
        else {
            return div().into_any_element();
        };
        let Some((_, input)) = workspace.read(cx).state.rename_container.clone() else {
            return div().into_any_element();
        };

        let content = v_flex().gap_3().child(Input::new(&input).w_full());

        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-container-rename",
            rust_i18n::t!("common.cancel").to_string(),
            {
                let view = workspace.clone();
                move |_, _, cx| {
                    view.update(cx, |workspace, cx| workspace.cancel_container_rename(cx));
                }
            },
        );

        let confirm = crate::widgets::dialog_confirm_button(
            "confirm-container-rename",
            rust_i18n::t!("common.confirm").to_string(),
            Some(IconName::Check),
            crate::widgets::DialogConfirmKind::Primary,
            move |_, _, cx| {
                workspace.update(cx, |workspace, cx| workspace.submit_container_rename(cx));
            },
        );

        let footer =
            crate::widgets::dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

        crate::widgets::dialog_body(
            "container-rename-modal",
            rust_i18n::t!("container.rename.modal_title").to_string(),
            Some(rust_i18n::t!("container.rename.prompt").to_string()),
            content,
            Some(footer),
        )
    }
}
