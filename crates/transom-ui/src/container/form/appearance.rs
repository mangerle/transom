//! 容器引擎连接表单的图标与环境色选择：紧凑方形按钮加弹层面板。

use super::*;
use crate::session_appearance::container_engine_icon;
use gpui_kit::component::popover::Popover;
use transom_ui_icons::{AppIconElement, UiIcon};

/// 外观色板色块尺寸；与会话表单保持一致。
const APPEARANCE_SWATCH_SIZE: f32 = 18.0;

impl ContainerFormView {
    /// 紧凑方形“图标”按钮：显示当前图标与环境色，点击弹出图标与颜色选择面板。
    pub(super) fn render_icon_picker_button(
        &self,
        form: &ContainerFormState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let icon = form.icon;
        let color = form.color;
        let display_icon = session_icon(icon).unwrap_or_else(|| container_engine_icon(form.kind));
        let tint = session_color(color).unwrap_or(p.primary);
        let entity = cx.entity();
        let content_palette = p.clone();

        let popover = Popover::new("container-form-icon-popover")
            .anchor(Anchor::TopLeft)
            .offset(px(4.0))
            .open(form.icon_open)
            .on_open_change(cx.listener(|this, open: &bool, _, cx| {
                this.set_icon_open(*open, cx);
            }))
            .trigger(
                Button::new("container-form-icon-trigger")
                    .outline()
                    .child(AppIconElement::new(display_icon).small().text_color(tint))
                    .accessibility_label(rust_i18n::t!("form.icon").to_string()),
            )
            .content(move |_, _, _cx| {
                Self::render_appearance_panel(&entity, icon, color, &content_palette)
            });

        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.icon").to_string()),
            )
            .child(popover)
            .into_any_element()
    }

    /// 弹层内的图标与环境色面板。
    ///
    /// 弹层内容闭包拿不到 `Context<Self>`，点击回调捕获实体句柄后经 `update`
    /// 回到实体；选择后弹层保持展开，便于连续挑选图标与颜色，点击外部收起。
    fn render_appearance_panel(
        entity: &Entity<ContainerFormView>,
        icon: SessionIcon,
        color: SessionColor,
        p: &Palette,
    ) -> AnyElement {
        let auto_entity = entity.clone();
        let icon_row = h_flex()
            .gap_1()
            .flex_wrap()
            .child(
                Button::new("container-form-icon-auto")
                    .small()
                    .child(AppIconElement::new(UiIcon::REFRESH_AUTO).small())
                    .accessibility_label(rust_i18n::t!("form.icon_auto").to_string())
                    .selected(icon == SessionIcon::Auto)
                    .tooltip(rust_i18n::t!("form.icon_auto").to_string())
                    .on_click(move |_: &ClickEvent, _, cx| {
                        auto_entity.update(cx, |this, cx| this.set_icon(SessionIcon::Auto, cx));
                    }),
            )
            .children(SESSION_ICON_CHOICES.into_iter().map(|choice| {
                let Some(name) = session_icon(choice) else {
                    return div().into_any_element();
                };
                let pick_entity = entity.clone();
                Button::new(ElementId::Name(
                    format!("container-form-icon-{choice:?}").into(),
                ))
                .small()
                .child(AppIconElement::new(name).small())
                .accessibility_label(format!("{choice:?}"))
                .tooltip(format!("{choice:?}"))
                .selected(icon == choice)
                .on_click(move |_: &ClickEvent, _, cx| {
                    pick_entity.update(cx, |this, cx| this.set_icon(choice, cx));
                })
                .into_any_element()
            }));

        let none_entity = entity.clone();
        let color_row = h_flex()
            .gap_2()
            .child(
                div()
                    .id("container-form-color-none")
                    .size(px(APPEARANCE_SWATCH_SIZE))
                    .rounded_full()
                    .border_1()
                    .border_color(if color == SessionColor::None {
                        p.primary
                    } else {
                        p.border
                    })
                    .bg(p.background)
                    .cursor_pointer()
                    .tooltip(move |_window, cx| {
                        Tooltip::new(rust_i18n::t!("form.color_none").to_string())
                            .build(_window, cx)
                    })
                    .on_click(move |_: &ClickEvent, _, cx| {
                        none_entity.update(cx, |this, cx| this.set_color(SessionColor::None, cx));
                    }),
            )
            .children(SESSION_COLOR_CHOICES.into_iter().map(|choice| {
                let Some(value) = session_color(choice) else {
                    return div().into_any_element();
                };
                let pick_entity = entity.clone();
                div()
                    .id(ElementId::Name(
                        format!("container-form-color-{choice:?}").into(),
                    ))
                    .size(px(APPEARANCE_SWATCH_SIZE))
                    .rounded_full()
                    .bg(value)
                    .border_1()
                    .border_color(if color == choice {
                        p.foreground
                    } else {
                        p.border
                    })
                    .cursor_pointer()
                    .on_click(move |_: &ClickEvent, _, cx| {
                        pick_entity.update(cx, |this, cx| this.set_color(choice, cx));
                    })
                    .into_any_element()
            }));

        v_flex()
            .w(rems(16.5))
            .p_2()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.icon").to_string()),
            )
            .child(icon_row)
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.color").to_string()),
            )
            .child(color_row)
            .into_any_element()
    }
}
