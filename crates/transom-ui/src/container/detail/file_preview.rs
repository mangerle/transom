//! 容器与数据卷内文本文件的预览与就地编辑器。

use super::*;
use crate::container::ContainerWorkspace;
use gpui_kit::component::Disableable;
use gpui_kit::component::input::{Textarea, TextareaState};

/// 文件预览渲染参数：预览内容与编辑状态属于同一组视图数据。
pub(super) struct FilePreviewView<'a> {
    pub(super) preview: &'a (String, String, bool),
    pub(super) preview_editor: Option<&'a Entity<TextareaState>>,
    pub(super) preview_saving: bool,
    pub(super) binary: bool,
    pub(super) readonly: bool,
}

/// 渲染文件预览卡片或就地编辑框。
pub(super) fn render_file_preview(
    view: &Entity<ContainerWorkspace>,
    params: FilePreviewView<'_>,
    p: &Palette,
) -> AnyElement {
    let FilePreviewView {
        preview,
        preview_editor,
        preview_saving,
        binary,
        readonly,
    } = params;
    let (path, content, truncated) = preview;
    let mut header = h_flex().gap_2().items_center().justify_between().child(
        h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(path.clone()),
            )
            .children((*truncated).then(|| {
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.files.truncated").to_string())
            }))
            .children(binary.then(|| {
                div()
                    .text_xs()
                    .text_color(p.warning)
                    .child(rust_i18n::t!("container.files.binary").to_string())
            })),
    );

    let mut actions = h_flex().gap_1p5().items_center();
    if let Some(_editor) = preview_editor {
        let save_view = view.clone();
        actions = actions
            .child(
                Button::new("container-file-save")
                    .primary()
                    .xsmall()
                    .icon(IconName::Save)
                    .label(rust_i18n::t!("common.save").to_string())
                    .disabled(preview_saving)
                    .on_click(move |_, _, cx| {
                        save_view.update(cx, |workspace, cx| {
                            workspace.save_edit_container_file(cx);
                        });
                    }),
            )
            .child(
                Button::new("container-file-cancel-edit")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .disabled(preview_saving)
                    .on_click({
                        let cancel_view = view.clone();
                        move |_, _, cx| {
                            cancel_view.update(cx, |workspace, cx| {
                                workspace.cancel_edit_container_file(cx);
                            });
                        }
                    }),
            );
    } else {
        if !readonly && !*truncated && !binary {
            let edit_view = view.clone();
            actions = actions.child(
                Button::new("container-file-edit")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Pen)
                    .label(rust_i18n::t!("container.files.edit").to_string())
                    .on_click(move |_, window, cx| {
                        edit_view.update(cx, |workspace, cx| {
                            workspace.begin_edit_container_file(window, cx);
                        });
                    }),
            );
        }
        let close_view = view.clone();
        actions = actions.child(
            Button::new("container-file-close-preview")
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .on_click(move |_, _, cx| {
                    close_view.update(cx, |workspace, cx| {
                        workspace.close_preview_container_file(cx);
                    });
                }),
        );
    }
    header = header.child(actions);

    let body_content: AnyElement = if binary {
        super::centered_hint(rust_i18n::t!("container.files.binary_hint").to_string(), p)
    } else if let Some(editor) = preview_editor {
        div()
            .flex_1()
            .min_h_0()
            .p_1()
            .rounded(p.radius.md)
            .bg(p.muted.opacity(0.15))
            .child(Textarea::new(editor).size_full())
            .into_any_element()
    } else {
        div()
            .id("container-preview-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .rounded(p.radius.md)
            .bg(p.muted.opacity(0.3))
            .font_family(p.mono_font_family.clone())
            .text_size(p.mono_font_size)
            .child(content.clone())
            .into_any_element()
    };

    v_flex()
        .flex_1()
        .min_h_0()
        .p_2()
        .gap_1()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Top, p.divider_width)
        .child(header)
        .child(body_content)
        .into_any_element()
}
