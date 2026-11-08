//! Podman Quadlet 单元列表模态弹窗。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::*;
use transom_core::ContainerQuadletAction;

impl AppShell {
    /// 渲染宿主 Quadlet 单元列表与 systemd 操作。
    pub(crate) fn render_container_quadlet_dialog_body(
        &self,
        app: &Entity<AppShell>,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some((tab_id, workspace)) =
            self.find_container_modal_tab(cx, |ws| ws.quadlets_dialog_open)
        else {
            return div().into_any_element();
        };
        let workspace = &workspace.read(cx).state;
        let loading = workspace.quadlets_loading;
        let error = workspace.quadlets_error.clone();

        let mut content = v_flex().gap_2().w_full();
        if loading {
            content = content.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Icon::new(IconName::Loader).xsmall())
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("container.quadlet.loading").to_string()),
                    ),
            );
        }
        if let Some(error) = error {
            content = content.child(div().text_sm().text_color(p.danger).child(error));
        }
        if !loading && workspace.quadlets.is_empty() && workspace.quadlets_error.is_none() {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.quadlet.empty").to_string()),
            );
        }
        for unit in &workspace.quadlets {
            let unit_name = unit.name.clone();
            let for_start = unit_name.clone();
            let for_stop = unit_name.clone();
            let for_restart = unit_name.clone();
            let active = unit.active == Some(true);
            let state_key = if active {
                "container.quadlet.active"
            } else {
                "container.quadlet.inactive"
            };
            let row = h_flex()
                .gap_2()
                .items_center()
                .p_2()
                .rounded(p.radius.md)
                .bg(p.muted.opacity(0.25))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().child(unit.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(format!(
                                    "{} · {} · {}",
                                    unit.kind,
                                    rust_i18n::t!(state_key),
                                    unit.path
                                )),
                        ),
                )
                .child(
                    Button::new(ElementId::Name(format!("quadlet-start-{unit_name}").into()))
                        .xsmall()
                        .ghost()
                        .label(rust_i18n::t!("container.action.start").to_string())
                        .disabled(active)
                        .on_click(crate::dialog_host::on_event(
                            app,
                            move |this, _: &ClickEvent, _, cx| {
                                this.container_quadlet_action(
                                    tab_id,
                                    for_start.clone(),
                                    ContainerQuadletAction::Start,
                                    cx,
                                );
                            },
                        )),
                )
                .child(
                    Button::new(ElementId::Name(format!("quadlet-stop-{unit_name}").into()))
                        .xsmall()
                        .ghost()
                        .label(rust_i18n::t!("container.action.stop").to_string())
                        .disabled(!active)
                        .on_click(crate::dialog_host::on_event(
                            app,
                            move |this, _: &ClickEvent, _, cx| {
                                this.container_quadlet_action(
                                    tab_id,
                                    for_stop.clone(),
                                    ContainerQuadletAction::Stop,
                                    cx,
                                );
                            },
                        )),
                )
                .child(
                    Button::new(ElementId::Name(
                        format!("quadlet-restart-{unit_name}").into(),
                    ))
                    .xsmall()
                    .ghost()
                    .label(rust_i18n::t!("container.action.restart").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        move |this, _: &ClickEvent, _, cx| {
                            this.container_quadlet_action(
                                tab_id,
                                for_restart.clone(),
                                ContainerQuadletAction::Restart,
                                cx,
                            );
                        },
                    )),
                );
            content = content.child(row);
        }

        let refresh = Button::new("container-quadlet-refresh")
            .ghost()
            .xsmall()
            .icon(IconName::RefreshCw)
            .label(rust_i18n::t!("common.refresh").to_string())
            .on_click(crate::dialog_host::on_event(
                app,
                move |this, _: &ClickEvent, _, cx| {
                    this.refresh_container_quadlets(tab_id, cx);
                    cx.notify();
                },
            ));
        let close = crate::widgets::dialog_cancel_button(
            "container-quadlet-close",
            rust_i18n::t!("common.close").to_string(),
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                this.close_container_quadlets_dialog(tab_id, cx);
            }),
        );
        let footer =
            crate::widgets::dialog_actions([refresh.into_any_element(), close.into_any_element()]);
        crate::widgets::dialog_body(
            "container-quadlet-modal",
            rust_i18n::t!("container.quadlet.title").to_string(),
            Some(rust_i18n::t!("container.quadlet.subtitle").to_string()),
            content,
            Some(footer),
        )
    }
}
