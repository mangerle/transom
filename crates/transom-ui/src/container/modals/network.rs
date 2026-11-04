//! 网络连接容器居中模态弹窗。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::Input;
use gpui_kit::*;

impl AppShell {
    /// 渲染网络连接容器模态弹窗主体。
    pub(crate) fn render_container_network_connect_dialog_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        _p: &Palette,
    ) -> AnyElement {
        let Some((tab_id, workspace)) =
            self.find_container_modal_tab(|ws| ws.connect_network.is_some())
        else {
            return div().into_any_element();
        };
        let Some((_, input)) = workspace.connect_network.clone() else {
            return div().into_any_element();
        };

        let content = v_flex().gap_3().child(Input::new(&input).w_full());

        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-network-connect",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                this.cancel_network_connect(tab_id, cx);
            }),
        );

        let confirm = crate::widgets::dialog_confirm_button(
            "confirm-network-connect",
            rust_i18n::t!("common.confirm").to_string(),
            Some(IconName::Check),
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, window, cx| {
                this.submit_network_connect(tab_id, window, cx);
            }),
        );

        let footer =
            crate::widgets::dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

        crate::widgets::dialog_body(
            "network-connect-modal",
            rust_i18n::t!("container.network_connect.modal_title").to_string(),
            Some(rust_i18n::t!("container.network_connect.prompt").to_string()),
            content,
            Some(footer),
        )
    }
}
