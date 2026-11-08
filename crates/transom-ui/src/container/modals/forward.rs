//! 容器内端口建立 SSH 本地端口转发模态弹窗主体渲染。
//!
//! 设计原理（Why）：通过直观的弹窗引导用户选择目标活跃 SSH 隧道、填写本地监听端口与目标主机，
//! 提供一键式的本地隧道映射体验，并在无可用会话时给出明确前置阻断提示。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable, h_flex, v_flex};
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    /// 渲染容器端口建立 SSH 端口转发模态弹窗主体。
    pub(crate) fn render_container_forward_dialog_body(
        &self,
        tab_id: TabId,
        app: &Entity<AppShell>,
        _window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some((_, workspace)) =
            self.find_container_modal_tab(cx, |ws| ws.forward_dialog.is_some())
        else {
            return div().into_any_element();
        };
        let Some(dialog) = workspace.read(cx).state.forward_dialog.as_ref() else {
            return div().into_any_element();
        };

        let container_name = dialog.container_name.clone();
        let private_port = dialog.private_port;
        let published_port = dialog.published_port;
        let has_sessions = !dialog.ssh_sessions.is_empty();
        let selected_index = dialog.selected_ssh_index;

        // 顶部目标信息卡片
        let info_card =
            h_flex()
                .gap_4()
                .p_2p5()
                .rounded_md()
                .bg(p.list_background)
                .border_1()
                .border_color(p.border)
                .child(
                    v_flex()
                        .gap_1()
                        .flex_1()
                        .child(
                            div().text_xs().text_color(p.muted_foreground).child(
                                rust_i18n::t!("container.forward.target_container").to_string(),
                            ),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_family(p.mono_font_family.clone())
                                .text_color(p.foreground)
                                .child(container_name),
                        ),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .flex_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("container.forward.remote_port").to_string()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_family(p.mono_font_family.clone())
                                .text_color(p.primary)
                                .child(private_port.to_string()),
                        ),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .flex_1()
                        .child(
                            div().text_xs().text_color(p.muted_foreground).child(
                                rust_i18n::t!("container.forward.published_port").to_string(),
                            ),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_family(p.mono_font_family.clone())
                                .text_color(if published_port.is_some() {
                                    p.primary
                                } else {
                                    p.muted_foreground
                                })
                                .child(
                                    published_port
                                        .map(|port| port.to_string())
                                        .unwrap_or_else(|| "—".to_string()),
                                ),
                        ),
                );

        // SSH 会话选择区
        let mut session_section = v_flex().gap_1p5().child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.forward.ssh_session").to_string()),
        );

        if !has_sessions {
            session_section = session_section.child(
                h_flex()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .bg(p.warning.opacity(0.1))
                    .border_1()
                    .border_color(p.warning)
                    .items_center()
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .size(px(14.0))
                            .text_color(p.warning),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.warning)
                            .child(rust_i18n::t!("container.forward.no_ssh_sessions").to_string()),
                    ),
            );
        } else {
            let session_buttons =
                dialog
                    .ssh_sessions
                    .iter()
                    .enumerate()
                    .map(|(idx, (name, _, _))| {
                        let is_selected = idx == selected_index;
                        let bg_color = if is_selected {
                            p.primary.opacity(0.12)
                        } else {
                            p.list_background
                        };
                        let border_color = if is_selected { p.primary } else { p.border };
                        let text_color = if is_selected { p.primary } else { p.foreground };
                        let app_entity = app.clone();
                        let display_name = name.clone();

                        h_flex()
                            .id(format!("forward-session-item-{idx}"))
                            .cursor_pointer()
                            .gap_2()
                            .px_2p5()
                            .py_1p5()
                            .rounded_md()
                            .bg(bg_color)
                            .border_1()
                            .border_color(border_color)
                            .items_center()
                            .on_click(move |_, _window, cx| {
                                app_entity.update(cx, |this, cx| {
                                    this.select_forward_ssh_session(tab_id, idx, cx);
                                });
                            })
                            .child(
                                Icon::new(IconName::Terminal)
                                    .size(px(13.0))
                                    .text_color(text_color),
                            )
                            .child(div().text_xs().text_color(text_color).child(display_name))
                    });
            session_section =
                session_section.child(h_flex().gap_2().flex_wrap().children(session_buttons));
        }

        // 本地端口与目标主机输入
        let inputs_section = h_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .flex_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.forward.local_port").to_string()),
                    )
                    .child(Input::new(&dialog.local_port_input).w_full()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .flex_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.forward.target_host").to_string()),
                    )
                    .child(Input::new(&dialog.target_host_input).w_full()),
            );

        let mut content = v_flex().gap_3().child(info_card);
        if published_port.is_none() {
            content =
                content.child(
                    h_flex()
                        .gap_2()
                        .p_2()
                        .rounded_md()
                        .bg(p.warning.opacity(0.1))
                        .border_1()
                        .border_color(p.warning)
                        .items_center()
                        .child(
                            Icon::new(IconName::TriangleAlert)
                                .size(px(14.0))
                                .text_color(p.warning),
                        )
                        .child(div().text_xs().text_color(p.warning).child(
                            rust_i18n::t!("container.forward.unpublished_hint").to_string(),
                        )),
                );
        }
        let content = content.child(session_section).child(inputs_section);

        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-container-forward",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                this.cancel_container_port_forward(tab_id, cx);
            }),
        );

        let confirm = if has_sessions {
            crate::widgets::dialog_confirm_button(
                "confirm-container-forward",
                rust_i18n::t!("container.forward.submit").to_string(),
                Some(IconName::ArrowLeftRight),
                crate::widgets::DialogConfirmKind::Primary,
                crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                    this.submit_container_port_forward(tab_id, cx);
                }),
            )
            .into_any_element()
        } else {
            Button::new("confirm-container-forward-disabled")
                .primary()
                .disabled(true)
                .icon(IconName::ArrowLeftRight)
                .label(rust_i18n::t!("container.forward.submit").to_string())
                .into_any_element()
        };

        let footer = crate::widgets::dialog_actions([cancel.into_any_element(), confirm]);

        crate::widgets::dialog_body(
            "container-forward-modal",
            rust_i18n::t!("container.forward.modal_title").to_string(),
            Some(rust_i18n::t!("container.forward.prompt").to_string()),
            content,
            Some(footer),
        )
    }
}
