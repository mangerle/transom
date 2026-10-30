//! SFTP 悬停预览浮层渲染。
//!
//! 设计原理（Why）：预览卡片挂在窗口根层（`render_overlays`）而不是文件行内，
//! 这样不会被虚拟列表的裁剪边界切掉，鼠标位置可以直接当作窗口坐标使用；
//! 卡片只读、不接收交互，离开文件行即由悬停状态机收起。

use super::preview::{
    PREVIEW_CARD_WIDTH, PREVIEW_IMAGE_HEIGHT, PreviewContent, PreviewKind, SftpPreviewState,
};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Icon;
use gpui_kit::*;
use std::sync::Arc;

impl AppShell {
    /// 渲染当前活动面板的悬停预览卡片；无预览或仍在延迟期时返回 `None`。
    pub(crate) fn render_sftp_hover_preview(
        &self,
        window: &Window,
        p: &Palette,
    ) -> Option<AnyElement> {
        let tab_id = self.workspace.active_tab?;
        let preview = self.workspace.tab(tab_id)?.sftp()?.hover_preview.as_ref()?;
        if matches!(preview.content, PreviewContent::Pending) {
            return None;
        }
        Some(Self::render_sftp_preview_card(preview, window, p))
    }

    fn render_sftp_preview_card(
        preview: &SftpPreviewState,
        window: &Window,
        p: &Palette,
    ) -> AnyElement {
        let (left, top) = preview_card_position(preview, window);
        let header = preview_card_header(preview, p);
        let body = preview_card_body(preview, p);

        v_flex()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(PREVIEW_CARD_WIDTH))
            .gap_2()
            .p_3()
            .rounded(p.radius.none)
            .border_1()
            .border_color(p.border)
            .bg(p.popover)
            .text_color(p.popover_foreground)
            .shadow_2xl()
            .overflow_hidden()
            .child(header)
            .child(body)
            .into_any_element()
    }
}

/// 计算卡片左上角坐标：默认落在鼠标右下方，靠近窗口边缘时翻转到另一侧。
fn preview_card_position(preview: &SftpPreviewState, window: &Window) -> (f32, f32) {
    const MARGIN: f32 = 12.0;
    const CURSOR_GAP: f32 = 18.0;

    let viewport = window.viewport_size();
    let viewport_width: f32 = viewport.width.into();
    let viewport_height: f32 = viewport.height.into();
    let anchor_x: f32 = preview.anchor.x.into();
    let anchor_y: f32 = preview.anchor.y.into();
    let estimated_height = match &preview.content {
        PreviewContent::Image { .. } => PREVIEW_IMAGE_HEIGHT + 76.0,
        PreviewContent::Text { lines, truncated } => {
            76.0 + lines.len() as f32 * 15.0 + if *truncated { 18.0 } else { 0.0 }
        }
        _ => 110.0,
    };

    let left = if anchor_x + CURSOR_GAP + PREVIEW_CARD_WIDTH + MARGIN <= viewport_width {
        anchor_x + CURSOR_GAP
    } else {
        (anchor_x - PREVIEW_CARD_WIDTH - CURSOR_GAP).max(MARGIN)
    };
    let top = if anchor_y + CURSOR_GAP + estimated_height + MARGIN <= viewport_height {
        anchor_y + CURSOR_GAP
    } else {
        (anchor_y - estimated_height - CURSOR_GAP).max(MARGIN)
    };
    (left, top)
}

fn preview_card_header(preview: &SftpPreviewState, p: &Palette) -> AnyElement {
    let (icon, label) = match preview.kind {
        PreviewKind::Image => (
            IconName::FileImage,
            rust_i18n::t!("sftp.preview.image_label").to_string(),
        ),
        PreviewKind::Text => (
            IconName::FileText,
            rust_i18n::t!("sftp.preview.text_label").to_string(),
        ),
    };
    h_flex()
        .gap_2()
        .items_center()
        .child(
            Icon::new(icon)
                .text_color(p.muted_foreground)
                .flex_shrink_0(),
        )
        .child(
            div()
                .flex_1()
                .truncate()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(preview.name.clone()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(label),
        )
        .into_any_element()
}

fn preview_card_body(preview: &SftpPreviewState, p: &Palette) -> AnyElement {
    match &preview.content {
        PreviewContent::Pending => div().into_any_element(),
        PreviewContent::Loading => {
            let message = match preview.kind {
                PreviewKind::Image => rust_i18n::t!("sftp.preview.loading_image").to_string(),
                PreviewKind::Text => rust_i18n::t!("sftp.preview.loading_text").to_string(),
            };
            h_flex()
                .h(px(64.0))
                .w_full()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(message)
                .into_any_element()
        }
        PreviewContent::Error(message) => div()
            .w_full()
            .text_xs()
            .text_color(p.danger)
            .child(message.clone())
            .into_any_element(),
        PreviewContent::Text { lines, truncated } => render_text_preview(lines, *truncated, p),
        PreviewContent::Image { thumbnail } => render_image_preview(thumbnail, p),
    }
}

fn render_text_preview(lines: &[String], truncated: bool, p: &Palette) -> AnyElement {
    // 字体与字号走主题等宽槽位，密度档位变化时预览文字随全局等宽字号缩放；
    // 行高按原 11/15 比例由字号派生，保持视觉密度不变。
    let mut column = v_flex()
        .gap(px(1.0))
        .font_family(p.mono_font_family.clone())
        .text_size(p.mono_font_size)
        .line_height(p.mono_font_size * 1.36);
    if lines.is_empty() {
        column = column.child(
            div()
                .text_size(px(12.0))
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("sftp.preview.empty_file").to_string()),
        );
    } else {
        for (index, line) in lines.iter().enumerate() {
            column = column.child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(
                        div()
                            .w(px(24.0))
                            .flex_shrink_0()
                            .text_right()
                            .text_color(p.muted_foreground)
                            .child(format!("{}", index.saturating_add(1))),
                    )
                    .child(div().flex_1().truncate().child(if line.is_empty() {
                        " ".to_string()
                    } else {
                        line.clone()
                    })),
            );
        }
    }

    let mut body = v_flex().gap_1().child(column);
    if truncated {
        body = body.child(
            div()
                .text_size(px(11.0))
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("sftp.preview.head_only").to_string()),
        );
    }
    body.into_any_element()
}

fn render_image_preview(thumbnail: &Arc<RenderImage>, p: &Palette) -> AnyElement {
    v_flex()
        .child(
            div()
                .w_full()
                .h(px(PREVIEW_IMAGE_HEIGHT))
                .rounded(p.radius.md)
                .overflow_hidden()
                .bg(p.muted)
                .child(
                    img(ImageSource::Render(thumbnail.clone()))
                        .w_full()
                        .h_full()
                        .object_fit(ObjectFit::Contain),
                ),
        )
        .into_any_element()
}
