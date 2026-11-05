//! 容器基础信息总览的分组卡片：运行身份、网络与端口、存储挂载、环境变量与健康时间线。
//!
//! 设计原理（Why）：
//! - 将原本单调的一长串键值对改造成清晰的模块化分组卡片，提升信息获取效率；
//! - 字段行统一 label 96px、11px 弱色，ID/镜像/命令等关键参数等宽排版并支持悬停一键复制；
//! - 端口映射提升为可交互 Chip（整块可点击直接在浏览器打开 URL，内置一键 SSH 端口转发）；
//! - 存储挂载采用两行式（宿主来源 → 容器目标 + 读写/挂载类型徽标）；
//! - 环境变量支持整组一键复制为 `.env` 文本；
//! - 健康检查诊断由小方块重构为垂直时间线，大幅提升排障信息密度。

use super::*;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use transom_core::{ContainerMount, ContainerPort, TabId};

/// 分组卡片容器：4% 弱底色、圆角、无冗余粗边框、11px 弱色标题。
pub(super) fn overview_card(
    title: impl Into<SharedString>,
    icon: IconName,
    p: &Palette,
    extra: Option<AnyElement>,
    content: impl IntoElement,
) -> AnyElement {
    let header = h_flex()
        .justify_between()
        .items_center()
        .pb_1p5()
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(Icon::new(icon).xsmall().text_color(p.muted_foreground))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.muted_foreground)
                        .child(title.into()),
                ),
        )
        .children(extra);

    v_flex()
        .gap_1p5()
        .p_3()
        .rounded(p.radius.md)
        .bg(p.muted.opacity(0.04))
        .border_1()
        .border_color(p.border.opacity(0.3))
        .child(header)
        .child(content)
        .into_any_element()
}

/// 支持等宽与悬停复制的属性行。
pub(super) fn copyable_field(
    label: impl Into<SharedString>,
    value: impl Into<String>,
    is_mono: bool,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let val_str = value.into();
    let display_str = if val_str.trim().is_empty() {
        "—".to_string()
    } else {
        val_str.clone()
    };
    let is_empty = val_str.trim().is_empty();

    let mut val_el = div()
        .flex_1()
        .min_w_0()
        .text_size(px(12.0))
        .truncate()
        .child(display_str);

    if is_mono {
        val_el = val_el.font_family(p.mono_font_family.clone());
    }

    let mut row = h_flex()
        .gap_2()
        .items_center()
        .child(
            div()
                .w(px(96.0))
                .flex_none()
                .text_size(px(11.0))
                .text_color(p.muted_foreground)
                .child(label.into()),
        )
        .child(val_el);

    if !is_empty {
        let text_to_copy = val_str.clone();
        let copy_btn = Button::new(ElementId::Name(
            format!("copy-field-{}", val_str.len()).into(),
        ))
        .ghost()
        .xsmall()
        .icon(IconName::Copy)
        .tooltip(rust_i18n::t!("common.copy").to_string())
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text_to_copy.clone()));
            this.show_toast(
                rust_i18n::t!("container.row.copied", value = text_to_copy.as_str()).to_string(),
                true,
                window,
                cx,
            );
        }));
        row = row.child(copy_btn);
    }

    row.into_any_element()
}

/// 端口映射 Chips 组。
pub(super) fn render_port_chips(
    tab_id: TabId,
    ports: &[ContainerPort],
    browser_host: &str,
    container_name: &str,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    if ports.is_empty() {
        return div()
            .text_size(px(12.0))
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
            .tooltip(rust_i18n::t!("container.forward.button_tooltip").to_string())
            .on_click(cx.listener({
                let container_name = container_name.clone();
                move |this, _, window, cx| {
                    this.open_container_port_forward(
                        tab_id,
                        &container_name,
                        private_port,
                        published_port,
                        window,
                        cx,
                    );
                }
            }));

        let chip_body = match port.public_port {
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
                                let _ = open::that_detached(&open_url);
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
                            .text_size(px(11.0))
                            .font_family(p.mono_font_family.clone())
                            .text_color(p.muted_foreground)
                            .child(label),
                    )
                    .child(forward_btn)
                    .into_any_element()
            }
        };

        chip_body
    });

    h_flex()
        .gap_2()
        .flex_wrap()
        .children(chips)
        .into_any_element()
}

/// 存储挂载两行式列表。
pub(super) fn render_mounts_card(mounts: &[ContainerMount], p: &Palette) -> AnyElement {
    if mounts.is_empty() {
        return div()
            .text_size(px(12.0))
            .text_color(p.muted_foreground)
            .child("—")
            .into_any_element();
    }

    let rows = mounts.iter().map(|mount| {
        let rw_badge = if mount.rw {
            crate::widgets::status_badge("rw", p.success, p)
        } else {
            crate::widgets::status_badge("ro", p.warning, p)
        };
        let kind_badge = if !mount.kind.is_empty() {
            Some(crate::widgets::status_badge(
                mount.kind.clone(),
                p.primary,
                p,
            ))
        } else {
            None
        };

        v_flex()
            .gap_0p5()
            .p_2()
            .rounded(p.radius.sm)
            .bg(p.background)
            .border_1()
            .border_color(p.border.opacity(0.3))
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(
                        Icon::new(IconName::Folder)
                            .xsmall()
                            .text_color(p.muted_foreground),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_family(p.mono_font_family.clone())
                            .truncate()
                            .child(
                                mount
                                    .volume_name()
                                    .unwrap_or(mount.source.as_str())
                                    .to_string(),
                            ),
                    )
                    .children(kind_badge),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(p.muted_foreground)
                            .child("→"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_family(p.mono_font_family.clone())
                            .text_color(p.foreground.opacity(0.9))
                            .truncate()
                            .child(mount.destination.clone()),
                    )
                    .child(rw_badge),
            )
            .into_any_element()
    });

    v_flex().gap_1p5().children(rows).into_any_element()
}

/// 环境变量列表（支持一键复制全部为 .env）。
pub(super) fn render_env_card(
    env: &[String],
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    if env.is_empty() {
        return div()
            .text_size(px(12.0))
            .text_color(p.muted_foreground)
            .child("—")
            .into_any_element();
    }

    let all_env_text = env.join("\n");
    let copy_env_text = all_env_text.clone();
    let copy_all_btn = Button::new("btn-copy-all-env")
        .ghost()
        .xsmall()
        .icon(IconName::Copy)
        .label(rust_i18n::t!("container.detail.copy_env").to_string())
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(copy_env_text.clone()));
            this.show_toast(
                rust_i18n::t!("container.detail.env_copied").to_string(),
                true,
                window,
                cx,
            );
        }));

    let items = env.iter().take(12).map(|entry| {
        let (k, v) = entry.split_once('=').unwrap_or((entry.as_str(), ""));
        h_flex()
            .gap_1()
            .items_center()
            .text_size(px(11.0))
            .font_family(p.mono_font_family.clone())
            .truncate()
            .child(div().text_color(p.muted_foreground).child(format!("{k}=")))
            .child(div().text_color(p.foreground).child(v.to_string()))
            .into_any_element()
    });

    let count_hint = if env.len() > 12 {
        let remaining = env.len() - 12;
        Some(
            div()
                .text_size(px(11.0))
                .text_color(p.muted_foreground)
                .child(
                    rust_i18n::t!("container.detail.env_more_hint", count = remaining).to_string(),
                )
                .into_any_element(),
        )
    } else {
        None
    };

    let content = v_flex()
        .gap_1()
        .p_2()
        .rounded(p.radius.sm)
        .bg(p.background)
        .border_1()
        .border_color(p.border.opacity(0.3))
        .children(items)
        .children(count_hint);

    overview_card(
        rust_i18n::t!("container.field.env").to_string(),
        IconName::FileCode,
        p,
        Some(copy_all_btn.into_any_element()),
        content,
    )
}

/// 健康检查时间线。
pub(super) fn render_health_timeline(
    health: &transom_core::ContainerHealthDetails,
    p: &Palette,
) -> AnyElement {
    let status_color = if health.status == "healthy" {
        p.success
    } else if health.status == "unhealthy" {
        p.danger
    } else {
        p.warning
    };

    let header_extra = h_flex()
        .gap_2()
        .items_center()
        .child(crate::widgets::status_badge(
            health.status.clone(),
            status_color,
            p,
        ))
        .child(
            div()
                .text_size(px(11.0))
                .text_color(p.muted_foreground)
                .child(format!(
                    "{}: {}",
                    rust_i18n::t!("container.health.failing_streak"),
                    health.failing_streak
                )),
        );

    let mut timeline_items = Vec::new();
    for item in health.log.iter().rev().take(5) {
        let ok = item.exit_code == 0;
        let dot_color = if ok { p.success } else { p.danger };
        let exit_badge =
            crate::widgets::status_badge(format!("exit {}", item.exit_code), dot_color, p);

        let row = h_flex()
            .gap_2()
            .items_start()
            .child(
                v_flex()
                    .items_center()
                    .pt_1()
                    .child(crate::widgets::status_dot_bare(dot_color))
                    .child(div().w(px(1.0)).h(px(16.0)).bg(p.border.opacity(0.4))),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_family(p.mono_font_family.clone())
                                    .text_color(p.muted_foreground)
                                    .child(item.start.clone()),
                            )
                            .child(exit_badge),
                    )
                    .when(!item.output.trim().is_empty(), |el| {
                        el.child(
                            div()
                                .p_1p5()
                                .rounded(p.radius.sm)
                                .bg(p.background)
                                .border_1()
                                .border_color(p.border.opacity(0.2))
                                .text_size(px(11.0))
                                .font_family(p.mono_font_family.clone())
                                .text_color(p.foreground.opacity(0.85))
                                .child(item.output.trim().to_string()),
                        )
                    }),
            )
            .into_any_element();
        timeline_items.push(row);
    }

    overview_card(
        format!(
            "{} · {}",
            rust_i18n::t!("container.field.health"),
            health.status
        ),
        IconName::Activity,
        p,
        Some(header_extra.into_any_element()),
        v_flex().gap_1().children(timeline_items),
    )
}
