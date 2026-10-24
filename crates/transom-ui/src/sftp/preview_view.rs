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

impl AppShell {
    /// 渲染当前活动面板的悬停预览卡片；无预览或仍在延迟期时返回 `None`。
    pub(crate) fn render_sftp_hover_preview(
        &self,
        window: &Window,
        p: &Palette,
    ) -> Option<AnyElement> {
        let tab_id = self.active_tab?;
        let preview = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)?
            .sftp
            .as_ref()?
            .hover_preview
            .as_ref()?;
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
            .rounded_lg()
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
        PreviewKind::Image => (IconName::FileImage, "图片预览"),
        PreviewKind::Text => (IconName::FileText, "文本预览"),
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
        PreviewContent::Loading => h_flex()
            .h(px(64.0))
            .w_full()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(p.muted_foreground)
            .child("正在读取预览…")
            .into_any_element(),
        PreviewContent::Error(message) => div()
            .w_full()
            .text_xs()
            .text_color(p.danger)
            .child(message.clone())
            .into_any_element(),
        PreviewContent::Text { lines, truncated } => render_text_preview(lines, *truncated, p),
        PreviewContent::Image { path } => render_image_preview(path, p),
    }
}

fn render_text_preview(lines: &[String], truncated: bool, p: &Palette) -> AnyElement {
    let mut column = v_flex()
        .gap(px(1.0))
        .font_family("monospace")
        .text_size(px(11.0))
        .line_height(px(15.0));
    if lines.is_empty() {
        column = column.child(
            div()
                .text_size(px(12.0))
                .text_color(p.muted_foreground)
                .child("（空文件）"),
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
                .child("仅显示文件开头部分"),
        );
    }
    body.into_any_element()
}

fn render_image_preview(path: &std::path::Path, p: &Palette) -> AnyElement {
    let muted = p.muted;
    let muted_foreground = p.muted_foreground;
    v_flex()
        .child(
            div()
                .w_full()
                .h(px(PREVIEW_IMAGE_HEIGHT))
                .rounded_md()
                .overflow_hidden()
                .bg(muted)
                .child(
                    img(path.to_path_buf())
                        .w_full()
                        .h_full()
                        .object_fit(ObjectFit::Contain)
                        .with_loading(move || image_placeholder("正在加载图片…", muted_foreground))
                        .with_fallback(move || {
                            image_placeholder("无法解析该图片", muted_foreground)
                        }),
                ),
        )
        .into_any_element()
}

fn image_placeholder(message: &'static str, color: Hsla) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .text_color(color)
        .child(message)
        .into_any_element()
}
