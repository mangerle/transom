//! 存储挂载卡片：宿主来源 → 容器目标 + 读写/挂载类型徽标。

use super::*;
use crate::container::ContainerWorkspace;
use gpui_kit::base::{h_flex, v_flex};
use transom_core::ContainerMount;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;

/// 存储挂载两行式列表。
///
/// `view` 与其余卡片保持统一的“实体句柄 + Palette”参数形态；当前挂载行没有
/// 交互动作，因此只借用不读取。
pub(crate) fn render_mounts_card(
    _view: &Entity<ContainerWorkspace>,
    mounts: &[ContainerMount],
    p: &Palette,
) -> AnyElement {
    if mounts.is_empty() {
        return div()
            .text_size(rems(0.75))
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
            .border_color(p.stroke.muted)
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(
                        AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER))
                            .xsmall()
                            .text_color(p.muted_foreground),
                    )
                    .child(
                        div()
                            .text_size(rems(0.6875))
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
                            .text_size(rems(0.6875))
                            .text_color(p.muted_foreground)
                            .child("→"),
                    )
                    .child(
                        div()
                            .text_size(rems(0.6875))
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
