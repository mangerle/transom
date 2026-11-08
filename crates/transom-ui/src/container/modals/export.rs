//! 容器配置反向导出查看居中模态弹窗。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::ClipboardItem;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

impl AppShell {
    /// 渲染容器配置反向导出模态弹窗主体。
    pub(crate) fn render_container_export_config_dialog_body(
        &self,
        app: &Entity<AppShell>,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some((tab_id, workspace)) =
            self.find_container_modal_tab(cx, |ws| ws.export_config.is_some())
        else {
            return div().into_any_element();
        };
        let Some((name, run_cmd, compose_yaml, is_compose)) =
            workspace.read(cx).state.export_config.clone()
        else {
            return div().into_any_element();
        };

        let title = rust_i18n::t!("container.export.title", name = name).to_string();
        let display_content = if is_compose {
            compose_yaml.clone()
        } else {
            run_cmd.clone()
        };

        let mode_tabs = h_flex()
            .gap_1()
            .items_center()
            .child(
                Button::new("container-export-tab-run")
                    .xsmall()
                    .ghost()
                    .selected(!is_compose)
                    .label(rust_i18n::t!("container.export.run_tab").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        move |this, _: &ClickEvent, _, cx| {
                            this.toggle_container_export_mode(tab_id, false, cx);
                        },
                    )),
            )
            .child(
                Button::new("container-export-tab-compose")
                    .xsmall()
                    .ghost()
                    .selected(is_compose)
                    .label(rust_i18n::t!("container.export.compose_tab").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        move |this, _: &ClickEvent, _, cx| {
                            this.toggle_container_export_mode(tab_id, true, cx);
                        },
                    )),
            );

        let code_box = div()
            .id("container-export-modal-scroll")
            .w_full()
            .max_h(px(320.0))
            .overflow_y_scroll()
            .p_2p5()
            .rounded(p.radius.md)
            .bg(p.muted.opacity(0.3))
            .border_1()
            .border_color(p.border)
            .text_xs()
            .font_family(".SystemUIFont")
            .text_color(p.foreground)
            .child(display_content.clone());

        let content = v_flex().gap_3().child(mode_tabs).child(code_box);

        let copy_btn = crate::widgets::dialog_confirm_button(
            "copy-container-export",
            rust_i18n::t!("common.copy").to_string(),
            Some(IconName::Copy),
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(app, {
                let copy_text = display_content.clone();
                move |this, _: &ClickEvent, window, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                    this.show_toast(
                        rust_i18n::t!("container.export.copied").to_string(),
                        true,
                        window,
                        cx,
                    );
                }
            }),
        );

        let close_btn = crate::widgets::dialog_cancel_button(
            "close-container-export",
            rust_i18n::t!("common.close").to_string(),
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                this.cancel_container_export(tab_id, cx);
            }),
        );

        let footer = crate::widgets::dialog_actions([
            close_btn.into_any_element(),
            copy_btn.into_any_element(),
        ]);

        crate::widgets::dialog_body(
            "container-export-modal",
            title,
            None::<String>,
            content,
            Some(footer),
        )
    }
}
