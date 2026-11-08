//! 容器提交为新镜像居中模态弹窗。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::Input;
use gpui_kit::*;

impl AppShell {
    /// 渲染容器提交镜像模态弹窗主体。
    pub(crate) fn render_container_commit_dialog_body(
        &self,
        app: &Entity<AppShell>,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some((tab_id, workspace)) =
            self.find_container_modal_tab(cx, |ws| ws.commit_container.is_some())
        else {
            return div().into_any_element();
        };
        let Some(state) = workspace.read(cx).state.commit_container.clone() else {
            return div().into_any_element();
        };
        let repo_input = state.repo;
        let tag_input = state.tag;
        let comment_input = state.comment;

        let content = v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.commit.repo_label").to_string()),
                    )
                    .child(Input::new(&repo_input).w_full()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.commit.tag_label").to_string()),
                    )
                    .child(Input::new(&tag_input).w_full()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.commit.comment_label").to_string()),
                    )
                    .child(Input::new(&comment_input).w_full()),
            );

        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-container-commit",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                this.cancel_container_commit(tab_id, cx);
            }),
        );

        let confirm = crate::widgets::dialog_confirm_button(
            "confirm-container-commit",
            rust_i18n::t!("common.confirm").to_string(),
            Some(IconName::Layers),
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, window, cx| {
                this.submit_container_commit(tab_id, window, cx);
            }),
        );

        let footer =
            crate::widgets::dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

        crate::widgets::dialog_body(
            "container-commit-modal",
            rust_i18n::t!("container.commit.modal_title").to_string(),
            Some(rust_i18n::t!("container.commit.prompt").to_string()),
            content,
            Some(footer),
        )
    }
}
