//! 容器基础信息总览的分组卡片：运行身份、网络与端口、存储挂载、环境变量与健康时间线。
//!
//! 设计原理（Why）：
//! - 将原本单调的一长串键值对改造成清晰的模块化分组卡片，提升信息获取效率；
//! - 字段行统一 label 96px、11px 弱色，ID/镜像/命令等关键参数等宽排版并支持悬停一键复制；
//! - 端口映射提升为可交互 Chip（整块可点击直接在浏览器打开 URL，内置一键 SSH 端口转发）；
//! - 存储挂载采用两行式（宿主来源 → 容器目标 + 读写/挂载类型徽标）；
//! - 环境变量支持整组一键复制为 `.env` 文本；
//! - 健康检查诊断由小方块重构为垂直时间线，大幅提升排障信息密度。
//!
//! 按关注点拆分为字段行、端口、存储、环境变量与健康时间线子模块；共用的卡片
//! 容器与复制行留在本文件，避免各子模块重复实现同一套底色、标题与复制提示。

use super::*;
use crate::container::ContainerWorkspace;
use gpui_kit::ClipboardItem;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};

mod environment;
mod health;
mod mounts;
mod ports;

pub(super) use environment::render_env_card;
pub(super) use health::render_health_timeline;
pub(super) use mounts::render_mounts_card;
pub(super) use ports::render_port_chips;

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
///
/// 设计原理（Why）：复制按钮只把文案写进工作区实体的 `notice`，由外壳渲染帧
/// 统一转成通知；渲染函数因此不持有 `Context<AppShell>`，也不会在渲染期更新实体。
pub(super) fn copyable_field(
    view: &Entity<ContainerWorkspace>,
    label: impl Into<SharedString>,
    value: impl Into<String>,
    is_mono: bool,
    p: &Palette,
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
        .accessibility_label(rust_i18n::t!("common.copy").to_string())
        .tooltip(rust_i18n::t!("common.copy").to_string())
        .on_click({
            let view = view.clone();
            move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text_to_copy.clone()));
                let message = rust_i18n::t!("container.row.copied", value = text_to_copy.as_str())
                    .to_string();
                view.update(cx, |workspace, cx| {
                    workspace.state.notice = Some(message);
                    cx.notify();
                });
            }
        });
        row = row.child(copy_btn);
    }

    row.into_any_element()
}
