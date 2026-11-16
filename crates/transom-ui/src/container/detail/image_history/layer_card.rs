use super::*;

pub(super) fn render_history_layer_card(
    view: &Entity<ContainerWorkspace>,
    index: usize,
    item: &transom_core::ContainerImageLayer,
    layer_advices: &[ImageSlimmingAdvice],
    p: &Palette,
) -> AnyElement {
    let size_str = if item.size == 0 {
        "0 B".to_string()
    } else {
        format!("+ {}", crate::format::format_size(item.size))
    };
    let cmd_raw = if item.created_by.is_empty() {
        item.comment.clone()
    } else {
        item.created_by.clone()
    };
    let cmd_cleaned = clean_dockerfile_instruction(&cmd_raw);
    let tags_str = if !item.tags.is_empty() {
        format!("Tag: {}", item.tags.join(", "))
    } else {
        String::new()
    };
    let copy_cmd = copy_button(
        view,
        ElementId::Name(format!("copy-layer-cmd-{index}").into()),
        cmd_raw,
        rust_i18n::t!("container.detail.copy_instruction").to_string(),
    );

    let mut card = v_flex()
        .gap_1p5()
        .p_2()
        .rounded(p.radius.md)
        .bg(p.surface.active)
        .border_1()
        .border_color(p.stroke.strong)
        .child(render_layer_card_header(
            index,
            &size_str,
            item.size > 0,
            item.created,
            &tags_str,
            copy_cmd,
            p,
        ))
        .child(
            div()
                .p_1p5()
                .rounded(p.radius.sm)
                .bg(p.background)
                .font_family(p.mono_font_family.clone())
                .text_size(p.mono_font_size)
                .text_color(p.foreground)
                .child(cmd_cleaned),
        );

    for adv in layer_advices {
        card = card.child(render_layer_advice_badge(adv, p));
    }
    card.into_any_element()
}

fn render_layer_card_header(
    index: usize,
    size_str: &str,
    has_size: bool,
    created: i64,
    tags_str: &str,
    copy_cmd: AnyElement,
    p: &Palette,
) -> AnyElement {
    h_flex()
        .justify_between()
        .items_center()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(p.radius.sm)
                        .bg(p.primary.opacity(0.15))
                        .text_xs()
                        .text_color(p.primary)
                        .child(format!("#{}", index + 1)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if has_size {
                            p.primary
                        } else {
                            p.muted_foreground
                        })
                        .child(size_str.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(format_unix(created)),
                )
                .children((!tags_str.is_empty()).then(|| {
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(p.radius.sm)
                        .bg(p.muted)
                        .text_xs()
                        .text_color(p.foreground)
                        .child(tags_str.to_string())
                })),
        )
        .child(copy_cmd)
        .into_any_element()
}

fn render_layer_advice_badge(adv: &ImageSlimmingAdvice, p: &Palette) -> AnyElement {
    let badge_color = if adv.is_warning { p.danger } else { p.warning };
    h_flex()
        .gap_2()
        .p_1p5()
        .rounded(p.radius.sm)
        .bg(badge_color.opacity(0.12))
        .items_start()
        .child(
            AppIconElement::new(if adv.is_warning {
                AppIcon::Ui(UiIcon::WARNING)
            } else {
                AppIcon::Custom(CustomIcon::INFO)
            })
            .with_size(px(14.0))
            .text_color(badge_color),
        )
        .child(
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(badge_color)
                        .child(adv.title.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(adv.suggestion.clone()),
                ),
        )
        .into_any_element()
}
