//! 镜像添加标签居中模态弹窗。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::Input;
use gpui_kit::*;

impl AppShell {
    /// 渲染镜像打标签模态弹窗主体。
    pub(crate) fn render_container_tag_dialog_body(
        &self,
        app: &Entity<AppShell>,
        cx: &App,
        _p: &Palette,
    ) -> AnyElement {
        let Some((tab_id, workspace)) =
            self.find_container_modal_tab(cx, |ws| ws.tag_image.is_some())
        else {
            return div().into_any_element();
        };
        let Some((_, input)) = workspace.read(cx).state.tag_image.clone() else {
            return div().into_any_element();
        };

        let content = v_flex().gap_3().child(Input::new(&input).w_full());

        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-image-tag",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                this.cancel_image_tag(tab_id, cx);
            }),
        );

        let confirm = crate::widgets::dialog_confirm_button(
            "confirm-image-tag",
            rust_i18n::t!("common.confirm").to_string(),
            Some(IconName::Check),
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, window, cx| {
                this.submit_image_tag(tab_id, window, cx);
            }),
        );

        let footer =
            crate::widgets::dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

        crate::widgets::dialog_body(
            "image-tag-modal",
            rust_i18n::t!("container.tag.modal_title").to_string(),
            Some(rust_i18n::t!("container.tag.prompt").to_string()),
            content,
            Some(footer),
        )
    }
}
