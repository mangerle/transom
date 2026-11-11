//! 审计日志查看器主体渲染：按钮回调经 `cx.listener` 回到实体。
//!
//! 渲染期实体已被租借，这里只读 `state`；所有动作在监听器里执行，避免出现
//! “渲染里再 `update` 自己”的双重租借。空态“打开设置”先关闭查看器，再通过
//! [`AuditViewerEvent::OpenSettings`] 交回外壳处理窗口级动作。

use super::entity::{AuditViewerEvent, AuditViewerView};
use super::types::{AuditLogEntry, AuditViewer, format_file_size};
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use std::path::PathBuf;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

impl AuditViewerView {
    /// 渲染审计日志查看器模态弹窗。
    pub(crate) fn render_body(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let Some(viewer) = self.viewer() else {
            return div().into_any_element();
        };

        let title = viewer.target_title.clone();
        let selected_file = viewer.entries.get(viewer.selected_index);
        let current_search = viewer.search_input.read(cx).value().trim().to_string();

        let toolbar = self.render_toolbar(viewer, cx);
        let file_list = self.render_file_list(viewer, cx, p);
        let right_pane = render_audit_content(viewer, &current_search, p);

        let left_pane = v_flex()
            .w(px(260.0))
            .h_full()
            .min_h_0()
            .flex_none()
            .overflow_hidden()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Right, p.divider_width)
            .pr_2()
            .child(file_list);

        let body = h_flex()
            .gap_3()
            .h(px(440.0))
            .flex_none()
            .child(left_pane)
            .child(right_pane);

        let footer = h_flex()
            .w_full()
            .flex_1()
            .justify_between()
            .items_center()
            .child(toolbar)
            .child(
                Button::new("close-audit-viewer")
                    .ghost()
                    .label(rust_i18n::t!("audit_viewer.close").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx))),
            );

        crate::widgets::dialog_body(
            "audit-viewer-modal",
            title,
            selected_file.map(|f| f.file_name.clone()),
            body,
            Some(footer),
        )
    }

    /// 审计查看器顶栏操作：范围切换、定位文件、删除、复制。
    fn render_toolbar(&self, viewer: &AuditViewer, cx: &mut Context<Self>) -> AnyElement {
        let selected_path = viewer
            .entries
            .get(viewer.selected_index)
            .map(|entry| entry.path.clone());
        let scope_button_text = if viewer.show_all {
            rust_i18n::t!("audit_viewer.current_session").to_string()
        } else {
            rust_i18n::t!("audit_viewer.all_logs").to_string()
        };
        // 顶栏操作区
        let mut toolbar = h_flex().gap_2().items_center();

        if viewer.session_name.is_some() {
            toolbar = toolbar.child(
                Button::new("toggle-audit-scope")
                    .outline()
                    .label(scope_button_text)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_scope(cx))),
            );
        }

        toolbar = toolbar.children(audit_file_action_buttons(
            cx,
            selected_path,
            viewer.content.is_empty(),
        ));
        toolbar.into_any_element()
    }

    /// 审计日志文件列表；空态提供打开设置入口。
    fn render_file_list(
        &self,
        viewer: &AuditViewer,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        // 左侧文件列表
        let mut file_list = v_flex()
            .id("audit-file-list")
            .size_full()
            .min_h_0()
            .gap_1()
            .overflow_y_scrollbar();
        if viewer.entries.is_empty() {
            file_list = file_list.child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .items_center()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("audit_viewer.no_logs").to_string())
                    .child(
                        div()
                            .text_center()
                            .child(rust_i18n::t!("audit_viewer.no_logs_hint").to_string()),
                    )
                    .child(
                        Button::new("open-settings-from-audit")
                            .outline()
                            .label(rust_i18n::t!("audit_viewer.open_settings").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.close(cx);
                                cx.emit(AuditViewerEvent::OpenSettings);
                            })),
                    ),
            );
        } else {
            for (index, entry) in viewer.entries.iter().enumerate() {
                file_list =
                    file_list.child(audit_file_row(entry, index == viewer.selected_index, cx, p));
            }
        }

        file_list.into_any_element()
    }
}

/// 右侧日志内容与搜索框；过滤逻辑保持纯函数，便于后续单测。
fn render_audit_content(viewer: &AuditViewer, current_search: &str, p: &Palette) -> AnyElement {
    // 右侧内容查看区
    let mut filtered_lines = Vec::new();
    for line in viewer.content.lines() {
        if current_search.is_empty()
            || line
                .to_ascii_lowercase()
                .contains(current_search.to_ascii_lowercase().as_str())
        {
            filtered_lines.push(line);
        }
    }

    let content_text = if filtered_lines.is_empty() && !viewer.content.is_empty() {
        rust_i18n::t!("audit_viewer.no_search_matches").to_string()
    } else {
        filtered_lines.join("\n")
    };

    let right_pane = v_flex()
        .flex_1()
        .gap_2()
        .size_full()
        .overflow_hidden()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().flex_1().child(viewer.search_input.clone()))
                .child(if viewer.is_truncated {
                    div()
                        .text_xs()
                        .text_color(p.warning)
                        .child(rust_i18n::t!("audit_viewer.truncated_hint").to_string())
                } else {
                    div()
                }),
        )
        .child(
            div()
                .id("audit-content-scroll")
                .flex_1()
                .size_full()
                .p_3()
                .rounded(p.radius.md)
                .bg(p.surface.hover)
                .border_1()
                .border_color(p.border)
                .overflow_y_scroll()
                .font_family(p.mono_font_family.clone())
                .text_size(p.mono_font_size)
                .line_height(relative(1.4))
                .text_color(p.foreground)
                .child(content_text),
        );

    right_pane.into_any_element()
}

/// 单条审计日志行：文件名、时间/大小与删除入口。
fn audit_file_row(
    entry: &AuditLogEntry,
    is_selected: bool,
    cx: &mut Context<AuditViewerView>,
    p: &Palette,
) -> AnyElement {
    let bg_color = if is_selected {
        p.primary.opacity(0.15)
    } else {
        gpui::transparent_black()
    };
    let border_color = if is_selected {
        p.primary
    } else {
        gpui::transparent_black()
    };
    let entry_path = entry.path.to_string_lossy().to_string();
    let item_path = entry_path.clone();
    let delete_path = entry_path.clone();
    h_flex()
        .id(ElementId::Name(
            format!("audit-log-item-{entry_path}").into(),
        ))
        .p_2()
        .rounded(p.radius.sm)
        .bg(bg_color)
        .border_1()
        .border_color(border_color)
        .cursor_pointer()
        .hover(|style| style.bg(p.surface.pressed))
        .items_center()
        .justify_between()
        .gap_2()
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.select_path(&item_path, cx);
        }))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.foreground)
                        .truncate()
                        .child(entry.file_name.clone()),
                )
                .child(
                    h_flex()
                        .justify_between()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(entry.modified_display.clone())
                        .child(format_file_size(entry.size_bytes)),
                ),
        )
        .child(
            crate::widgets::IconButton::new(
                ElementId::Name(format!("delete-audit-log-{delete_path}").into()),
                AppIcon::Ui(UiIcon::DELETE),
                rust_i18n::t!("common.delete").to_string(),
            )
            .size(crate::widgets::IconButtonSize::Compact)
            .tooltip(rust_i18n::t!("audit_viewer.delete_log").to_string())
            .render()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.delete_path(&delete_path, cx);
            })),
        )
        .into_any_element()
}

/// 选中文件动作：定位、删除与复制内容；未选中时定位/删除禁用。
fn audit_file_action_buttons(
    cx: &mut Context<AuditViewerView>,
    selected_path: Option<PathBuf>,
    content_empty: bool,
) -> Vec<Button> {
    let has_selected = selected_path.is_some();
    let delete_path = selected_path
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    let mut buttons = Vec::with_capacity(3);
    if let Some(path) = selected_path {
        buttons.push(
            Button::new("reveal-audit-file")
                .outline()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER)).small())
                .label(rust_i18n::t!("audit_viewer.reveal").to_string())
                .on_click(move |_, _, _| {
                    crate::audit::reveal_in_file_manager(path.clone());
                }),
        );
    } else {
        buttons.push(
            Button::new("reveal-audit-file")
                .outline()
                .disabled(true)
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER)).small())
                .label(rust_i18n::t!("audit_viewer.reveal").to_string()),
        );
    }

    buttons.push(
        Button::new("delete-current-audit-file")
            .outline()
            .disabled(!has_selected)
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small())
            .label(rust_i18n::t!("audit_viewer.delete_log").to_string())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some(path) = delete_path.as_deref() {
                    this.delete_path(path, cx);
                }
            })),
    );

    buttons.push(
        Button::new("copy-audit-content")
            .outline()
            .disabled(!has_selected || content_empty)
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::COPY)).small())
            .label(rust_i18n::t!("audit_viewer.copy_content").to_string())
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.copy_content(cx))),
    );
    buttons
}
