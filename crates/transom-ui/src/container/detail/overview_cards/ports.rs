//! 端口映射 Chips：整块可打开宿主机 URL，并内置一键 SSH 端口转发。

use super::*;
use crate::container::{ContainerWorkspace, ContainerWorkspaceEvent};
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use transom_core::ContainerPort;

/// 端口映射 Chips 组。
///
/// 设计原理（Why）：端口转发需要窗口打开弹窗，渲染函数拿不到 `Window`，因此
/// 按钮只把容器名、私有端口与已发布端口打包成事件交给外壳，由渲染帧统一
/// 调用既有外壳方法；外壳处理事件时不回读本实体。
pub(crate) fn render_port_chips(
    view: &Entity<ContainerWorkspace>,
    ports: &[ContainerPort],
    browser_host: &str,
    container_name: &str,
    p: &Palette,
) -> AnyElement {
    if ports.is_empty() {
        return div()
            .text_size(rems(0.75))
            .text_color(p.muted_foreground)
            .child("—")
            .into_any_element();
    }

    let host_for_ports = browser_host.to_string();
    let container_name = container_name.trim_start_matches('/').to_string();

    let chips = ports.iter().map(|port| {
        let private_port = port.private_port;
        let published_port = port.public_port;
        let protocol = if port.protocol.is_empty() {
            "tcp"
        } else {
            &port.protocol
        };
        let forward_btn = Button::new(ElementId::Name(format!("chip-fwd-{}", private_port).into()))
            .ghost()
            .xsmall()
            .icon(IconName::ArrowLeftRight)
            .accessibility_label(rust_i18n::t!("container.forward.button_tooltip").to_string())
            .tooltip(rust_i18n::t!("container.forward.button_tooltip").to_string())
            .on_click({
                let view = view.clone();
                let container_name = container_name.clone();
                move |_, _, cx| {
                    view.update(cx, |_workspace, cx| {
                        cx.emit(ContainerWorkspaceEvent::OpenPortForward {
                            container: container_name.clone(),
                            private: private_port,
                            public: published_port,
                        });
                    });
                }
            });

        match port.public_port {
            Some(public) => {
                let url = format!("http://{host_for_ports}:{public}");
                let label = format!("{public} → {private_port}/{protocol}");
                let open_url = url.clone();
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .px_2()
                    .py_1()
                    .rounded(p.radius.sm)
                    .bg(p.muted.opacity(0.12))
                    .border_1()
                    .border_color(p.border.opacity(0.4))
                    .child(
                        Button::new(ElementId::Name(format!("chip-open-{public}").into()))
                            .ghost()
                            .xsmall()
                            .icon(IconName::ExternalLink)
                            .label(label)
                            .tooltip(url)
                            .on_click(move |_, _, _| {
                                if let Err(error) =
                                    transom_platform::opener::open_with_default_app(&open_url)
                                {
                                    log::warn!(
                                        "打开容器概览链接失败，地址: {open_url}，原因: {error}"
                                    );
                                }
                            }),
                    )
                    .child(forward_btn)
                    .into_any_element()
            }
            None => {
                let label = format!("{private_port}/{protocol}");
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .px_2()
                    .py_1()
                    .rounded(p.radius.sm)
                    .bg(p.muted.opacity(0.08))
                    .border_1()
                    .border_color(p.border.opacity(0.3))
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .font_family(p.mono_font_family.clone())
                            .text_color(p.muted_foreground)
                            .child(label),
                    )
                    .child(forward_btn)
                    .into_any_element()
            }
        }
    });

    h_flex()
        .gap_2()
        .flex_wrap()
        .children(chips)
        .into_any_element()
}
