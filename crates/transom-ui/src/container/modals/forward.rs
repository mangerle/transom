//! 容器内端口建立 SSH 本地端口转发模态弹窗主体渲染。
//!
//! 设计原理（Why）：通过直观的弹窗引导用户选择目标活跃 SSH 隧道、填写本地监听端口与目标主机，
//! 提供一键式的本地隧道映射体验，并在无可用会话时给出明确前置阻断提示。

use crate::container::types::ContainerForwardDialog;
use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::palette::Palette;
use crate::widgets::{
    DialogConfirmKind, dialog_actions, dialog_body, dialog_cancel_button, dialog_confirm_button,
};
use gpui_kit::base::{Disableable, h_flex, v_flex};
use gpui_kit::component::input::Input;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// 渲染容器端口建立 SSH 端口转发模态弹窗主体。
///
/// 设计原理（Why）：候选会话、输入控件与打开状态都归工作区实体，渲染只读
/// 状态快照；选择会话、取消与确认回调直接 `update` 实体，不再经外壳转发。
pub(crate) fn render_container_forward_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    p: &Palette,
) -> AnyElement {
    let Some(dialog) = state.modals.forward_dialog.as_ref() else {
        return div().into_any_element();
    };
    let has_sessions = !dialog.ssh_sessions.is_empty();
    let mut content = v_flex().gap_3().child(forward_info_card(dialog, p));
    if dialog.published_port.is_none() {
        content = content.child(unpublished_hint(p));
    }
    let content = content
        .child(forward_session_section(view, dialog, p))
        .child(forward_inputs(dialog, p));
    let footer = forward_footer(view, has_sessions);
    dialog_body(
        "container-forward-modal",
        rust_i18n::t!("container.forward.modal_title").to_string(),
        Some(rust_i18n::t!("container.forward.prompt").to_string()),
        content,
        Some(footer),
    )
}

/// 顶部目标信息卡片：容器名、远端端口与发布端口三格等宽。
fn forward_info_card(dialog: &ContainerForwardDialog, p: &Palette) -> AnyElement {
    let container_name = dialog.container_name.clone();
    let private_port = dialog.private_port.to_string();
    let published = dialog.published_port;
    let muted = p.muted_foreground;
    let primary = p.primary;
    h_flex()
        .gap_4()
        .p_2p5()
        .rounded_md()
        .bg(p.list_background)
        .border_1()
        .border_color(p.border)
        .child(info_cell(
            "container.forward.target_container",
            div()
                .text_sm()
                .font_family(p.mono_font_family.clone())
                .text_color(p.foreground)
                .child(container_name)
                .into_any_element(),
            muted,
        ))
        .child(info_cell(
            "container.forward.remote_port",
            div()
                .text_sm()
                .font_family(p.mono_font_family.clone())
                .text_color(primary)
                .child(private_port)
                .into_any_element(),
            muted,
        ))
        .child(info_cell(
            "container.forward.published_port",
            div()
                .text_sm()
                .font_family(p.mono_font_family.clone())
                .text_color(if published.is_some() { primary } else { muted })
                .child(
                    published
                        .map(|port| port.to_string())
                        .unwrap_or_else(|| "—".to_string()),
                )
                .into_any_element(),
            muted,
        ))
        .into_any_element()
}

fn info_cell(label_key: &'static str, value: AnyElement, muted: gpui_kit::Hsla) -> AnyElement {
    v_flex()
        .gap_1()
        .flex_1()
        .child(
            div()
                .text_xs()
                .text_color(muted)
                .child(rust_i18n::t!(label_key).to_string()),
        )
        .child(value)
        .into_any_element()
}

fn forward_session_section(
    view: &Entity<ContainerWorkspace>,
    dialog: &ContainerForwardDialog,
    p: &Palette,
) -> AnyElement {
    let header = div()
        .text_xs()
        .text_color(p.muted_foreground)
        .child(rust_i18n::t!("container.forward.ssh_session").to_string());
    let mut section = v_flex().gap_1p5().child(header);
    if dialog.ssh_sessions.is_empty() {
        section = section.child(no_ssh_sessions_hint(p));
    } else {
        section = section.child(forward_session_buttons(view, dialog, p));
    }
    section.into_any_element()
}

fn no_ssh_sessions_hint(p: &Palette) -> AnyElement {
    h_flex()
        .gap_2()
        .p_2()
        .rounded_md()
        .bg(p.warning.opacity(0.1))
        .border_1()
        .border_color(p.warning)
        .items_center()
        .child(
            AppIconElement::new(AppIcon::Ui(UiIcon::WARNING))
                .with_rems(rems(crate::theme::tokens::control::COMPACT_ICON))
                .text_color(p.warning),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.warning)
                .child(rust_i18n::t!("container.forward.no_ssh_sessions").to_string()),
        )
        .into_any_element()
}

fn forward_session_buttons(
    view: &Entity<ContainerWorkspace>,
    dialog: &ContainerForwardDialog,
    p: &Palette,
) -> AnyElement {
    let selected_index = dialog.selected_ssh_index;
    let buttons = dialog
        .ssh_sessions
        .iter()
        .enumerate()
        .map(|(idx, (name, _, _))| {
            let is_selected = idx == selected_index;
            let bg_color = if is_selected {
                p.tone.primary_subtle
            } else {
                p.list_background
            };
            let border_color = if is_selected { p.primary } else { p.border };
            let text_color = if is_selected { p.primary } else { p.foreground };
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
                .on_click({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |workspace, cx| {
                            workspace.select_forward_ssh_session(idx, cx);
                        });
                    }
                })
                .child(
                    AppIconElement::new(AppIcon::Domain(DomainIcon::TERMINAL))
                        .with_rems(rems(crate::theme::tokens::control::COMPACT_ICON))
                        .text_color(text_color),
                )
                .child(div().text_xs().text_color(text_color).child(display_name))
        });
    h_flex()
        .gap_2()
        .flex_wrap()
        .children(buttons)
        .into_any_element()
}

fn forward_inputs(dialog: &ContainerForwardDialog, p: &Palette) -> AnyElement {
    h_flex()
        .gap_3()
        .child(input_field(
            "container.forward.local_port",
            Input::new(&dialog.local_port_input).w_full(),
            p,
        ))
        .child(input_field(
            "container.forward.target_host",
            Input::new(&dialog.target_host_input).w_full(),
            p,
        ))
        .into_any_element()
}

fn input_field(label_key: &'static str, input: impl IntoElement, p: &Palette) -> AnyElement {
    v_flex()
        .gap_1()
        .flex_1()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!(label_key).to_string()),
        )
        .child(input)
        .into_any_element()
}

fn unpublished_hint(p: &Palette) -> AnyElement {
    h_flex()
        .gap_2()
        .p_2()
        .rounded_md()
        .bg(p.warning.opacity(0.1))
        .border_1()
        .border_color(p.warning)
        .items_center()
        .child(
            AppIconElement::new(AppIcon::Ui(UiIcon::WARNING))
                .with_rems(rems(crate::theme::tokens::control::COMPACT_ICON))
                .text_color(p.warning),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.warning)
                .child(rust_i18n::t!("container.forward.unpublished_hint").to_string()),
        )
        .into_any_element()
}

fn forward_footer(view: &Entity<ContainerWorkspace>, has_sessions: bool) -> AnyElement {
    let cancel_view = view.clone();
    let cancel = dialog_cancel_button(
        "cancel-container-forward",
        rust_i18n::t!("common.cancel").to_string(),
        move |_, _, cx| {
            cancel_view.update(cx, |workspace, cx| {
                workspace.cancel_container_port_forward(cx)
            });
        },
    );
    let confirm = if has_sessions {
        dialog_confirm_button(
            "confirm-container-forward",
            rust_i18n::t!("container.forward.submit").to_string(),
            Some(AppIcon::Domain(DomainIcon::FORWARD)),
            DialogConfirmKind::Primary,
            {
                let view = view.clone();
                move |_, _, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.submit_container_port_forward(cx)
                    });
                }
            },
        )
        .into_any_element()
    } else {
        dialog_confirm_button(
            "confirm-container-forward-disabled",
            rust_i18n::t!("container.forward.submit").to_string(),
            Some(AppIcon::Domain(DomainIcon::FORWARD)),
            DialogConfirmKind::Primary,
            |_, _, _| {},
        )
        .disabled(true)
        .into_any_element()
    };
    dialog_actions([cancel.into_any_element(), confirm])
}
