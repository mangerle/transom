//! Podman Pod 列表模态弹窗。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::*;
use transom_core::ContainerPodAction;

impl AppShell {
    /// 渲染 Pod 列表弹窗：列表、加载/错误状态与启停按钮。
    pub(crate) fn render_container_pods_dialog_body(
        &self,
        app: &Entity<AppShell>,
        p: &Palette,
    ) -> AnyElement {
        let Some((tab_id, workspace)) = self.find_container_modal_tab(|ws| ws.pods_dialog_open)
        else {
            return div().into_any_element();
        };
        let loading = workspace.pods_loading;
        let error = workspace.pods_error.clone();

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
                            .child(rust_i18n::t!("container.pods.loading").to_string()),
                    ),
            );
        }
        if let Some(error) = error {
            content = content.child(div().text_sm().text_color(p.danger).child(error));
        }
        if !loading && workspace.pods.is_empty() && workspace.pods_error.is_none() {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.pods.empty").to_string()),
            );
        }
        for pod in &workspace.pods {
            let pod_id = pod.id.clone();
            let pod_for_start = pod_id.clone();
            let pod_for_stop = pod_id.clone();
            let pod_for_restart = pod_id.clone();
            let status_running = pod.status.to_ascii_lowercase().contains("running");
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
                        .child(div().text_sm().child(pod.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(format!(
                                    "{} · {} {}",
                                    pod.status,
                                    pod.containers,
                                    rust_i18n::t!("container.pods.containers")
                                )),
                        ),
                )
                .child(
                    Button::new(ElementId::Name(format!("pod-start-{pod_id}").into()))
                        .xsmall()
                        .ghost()
                        .label(rust_i18n::t!("container.action.start").to_string())
                        .disabled(status_running)
                        .on_click(crate::dialog_host::on_event(
                            app,
                            move |this, _: &ClickEvent, _, cx| {
                                this.container_pod_action(
                                    tab_id,
                                    pod_for_start.clone(),
                                    ContainerPodAction::Start,
                                    cx,
                                );
                            },
                        )),
                )
                .child(
                    Button::new(ElementId::Name(format!("pod-stop-{pod_id}").into()))
                        .xsmall()
                        .ghost()
                        .label(rust_i18n::t!("container.action.stop").to_string())
                        .disabled(!status_running)
                        .on_click(crate::dialog_host::on_event(
                            app,
                            move |this, _: &ClickEvent, _, cx| {
                                this.container_pod_action(
                                    tab_id,
                                    pod_for_stop.clone(),
                                    ContainerPodAction::Stop,
                                    cx,
                                );
                            },
                        )),
                )
                .child(
                    Button::new(ElementId::Name(format!("pod-restart-{pod_id}").into()))
                        .xsmall()
                        .ghost()
                        .label(rust_i18n::t!("container.action.restart").to_string())
                        .on_click(crate::dialog_host::on_event(
                            app,
                            move |this, _: &ClickEvent, _, cx| {
                                this.container_pod_action(
                                    tab_id,
                                    pod_for_restart.clone(),
                                    ContainerPodAction::Restart,
                                    cx,
                                );
                            },
                        )),
                );
            content = content.child(row);
        }

        let refresh = Button::new("container-pods-refresh")
            .ghost()
            .xsmall()
            .icon(IconName::RefreshCw)
            .label(rust_i18n::t!("common.refresh").to_string())
            .on_click(crate::dialog_host::on_event(
                app,
                move |this, _: &ClickEvent, _, cx| {
                    this.refresh_container_pods(tab_id);
                    cx.notify();
                },
            ));
        let close = crate::widgets::dialog_cancel_button(
            "container-pods-close",
            rust_i18n::t!("common.close").to_string(),
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                this.close_container_pods_dialog(tab_id, cx);
            }),
        );
        let footer =
            crate::widgets::dialog_actions([refresh.into_any_element(), close.into_any_element()]);
        crate::widgets::dialog_body(
            "container-pods-modal",
            rust_i18n::t!("container.pods.title").to_string(),
            Some(rust_i18n::t!("container.pods.subtitle").to_string()),
            content,
            Some(footer),
        )
    }
}
