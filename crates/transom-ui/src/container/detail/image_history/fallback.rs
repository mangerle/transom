use super::*;
use crate::container::ContainerWorkspace;
use gpui_kit::ClipboardItem;
use transom_ui_icons::UiIcon;

pub(super) fn render_layer_digests_fallback(
    view: &Entity<ContainerWorkspace>,
    layer_digests: &[String],
    p: &Palette,
) -> AnyElement {
    let mut layers_list = v_flex().gap_2().child(
        v_flex()
            .gap_1()
            .p_2()
            .rounded(p.radius.md)
            .bg(p.primary.opacity(0.1))
            .border_1()
            .border_color(p.primary.opacity(0.3))
            .child(
                div()
                    .text_xs()
                    .text_color(p.primary)
                    .child(rust_i18n::t!("container.detail.history_fallback_hint").to_string()),
            ),
    );

    for (index, digest) in layer_digests.iter().enumerate() {
        let copy_digest = copy_button(
            view,
            ElementId::Name(format!("copy-layer-digest-{index}").into()),
            digest.clone(),
            rust_i18n::t!("container.detail.copy_layer_digest").to_string(),
        );
        layers_list = layers_list.child(
            h_flex()
                .justify_between()
                .items_center()
                .p_2()
                .rounded(p.radius.md)
                .bg(p.surface.active)
                .border_1()
                .border_color(p.stroke.strong)
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .flex_1()
                        .min_w_0()
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
                                .flex_1()
                                .min_w_0()
                                .font_family(p.mono_font_family.clone())
                                .text_size(p.mono_font_size)
                                .text_color(p.foreground)
                                .child(digest.clone()),
                        ),
                )
                .child(copy_digest),
        );
    }
    scroll_body(layers_list, p).into_any_element()
}

/// 清洗并美化 Docker 构建历史指令，去除内部包装命令前缀。
pub(crate) fn clean_dockerfile_instruction(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(stripped) = trimmed.strip_prefix("/bin/sh -c #(nop) ") {
        stripped.trim().to_string()
    } else if let Some(stripped) = trimmed.strip_prefix("/bin/sh -c ") {
        format!("RUN {stripped}")
    } else if trimmed.is_empty() {
        "<empty layer>".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 通用一键复制按钮辅助组件。
///
/// 设计原理（Why）：复制只改剪贴板与工作区 `notice`，提示由外壳渲染帧统一转成
/// 通知；按钮因此只持有工作区实体句柄，不再需要 `Context<AppShell>`。
pub(crate) fn copy_button(
    view: &Entity<ContainerWorkspace>,
    id: ElementId,
    text: String,
    tooltip: String,
) -> AnyElement {
    let view = view.clone();
    crate::widgets::IconButton::new(id, UiIcon::COPY, tooltip)
        .size(crate::widgets::IconButtonSize::RowAction)
        .render()
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            let message = rust_i18n::t!("container.row.copied", value = text).to_string();
            view.update(cx, |workspace, cx| {
                workspace.state.notice = Some(message);
                cx.notify();
            });
        })
        .into_any_element()
}
