//! 审计日志查看器模态弹窗渲染。

use super::types::format_file_size;
use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::*;

impl AppShell {
    /// 渲染审计日志查看器模态弹窗。
    pub(crate) fn render_audit_viewer_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(viewer) = self.state.modals.tools.audit_viewer.as_ref() else {
            return div().into_any_element();
        };

        let title = viewer.target_title.clone();
        let selected_file = viewer.entries.get(viewer.selected_index);
        let current_search = viewer.search_input.read(cx).value().trim().to_string();

        let toolbar = self.render_audit_toolbar(app, viewer);
        let file_list = self.render_audit_file_list(app, viewer, p);
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
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            this.state.modals.tools.audit_viewer = None;
                            cx.notify();
                        },
                    )),
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
    fn render_audit_toolbar(
        &self,
        app: &Entity<AppShell>,
        viewer: &super::types::AuditViewer,
    ) -> AnyElement {
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
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            this.toggle_audit_viewer_scope(cx);
                        },
                    )),
            );
        }

        let has_selected = selected_path.is_some();
        let selected_path_clone = selected_path.clone();

        if let Some(path) = selected_path_clone {
            toolbar = toolbar.child(
                Button::new("reveal-audit-file")
                    .outline()
                    .icon(IconName::FolderOpen)
                    .label(rust_i18n::t!("audit_viewer.reveal").to_string())
                    .on_click(move |_, _, _| {
                        crate::audit::reveal_in_file_manager(path.clone());
                    }),
            );
        } else {
            toolbar = toolbar.child(
                Button::new("reveal-audit-file")
                    .outline()
                    .disabled(true)
                    .icon(IconName::FolderOpen)
                    .label(rust_i18n::t!("audit_viewer.reveal").to_string()),
            );
        }

        toolbar = toolbar.child(
            Button::new("delete-current-audit-file")
                .outline()
                .disabled(!has_selected)
                .icon(IconName::Trash)
                .label(rust_i18n::t!("audit_viewer.delete_log").to_string())
                .on_click(crate::dialog_host::on_event(
                    app,
                    move |this, _: &ClickEvent, _, cx| {
                        let idx = this
                            .state
                            .modals
                            .tools
                            .audit_viewer
                            .as_ref()
                            .map(|v| v.selected_index)
                            .unwrap_or(0);
                        this.delete_audit_log(idx, cx);
                    },
                )),
        );

        toolbar = toolbar.child(
            Button::new("copy-audit-content")
                .outline()
                .disabled(!has_selected || viewer.content.is_empty())
                .icon(IconName::Copy)
                .label(rust_i18n::t!("audit_viewer.copy_content").to_string())
                .on_click(crate::dialog_host::on_event(
                    app,
                    |this, _: &ClickEvent, _, cx| {
                        this.copy_audit_viewer_content(cx);
                    },
                )),
        );
        toolbar.into_any_element()
    }

    /// 审计日志文件列表；空态提供打开设置入口。
    fn render_audit_file_list(
        &self,
        app: &Entity<AppShell>,
        viewer: &super::types::AuditViewer,
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
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, _: &ClickEvent, window, cx| {
                                    this.state.modals.tools.audit_viewer = None;
                                    this.open_settings_modal(None, window, cx);
                                },
                            )),
                    ),
            );
        } else {
            for (idx, entry) in viewer.entries.iter().enumerate() {
                file_list = file_list.child(audit_file_row(
                    idx,
                    entry,
                    idx == viewer.selected_index,
                    app,
                    p,
                ));
            }
        }

        file_list.into_any_element()
    }
}

/// 右侧日志内容与搜索框；过滤逻辑保持纯函数，便于后续单测。
fn render_audit_content(
    viewer: &super::types::AuditViewer,
    current_search: &str,
    p: &Palette,
) -> AnyElement {
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
                .bg(p.muted.opacity(0.3))
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
    idx: usize,
    entry: &super::types::AuditLogEntry,
    is_selected: bool,
    app: &Entity<AppShell>,
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
    let item_delete_idx = idx;
    h_flex()
        .id(ElementId::Name(format!("audit-log-item-{idx}").into()))
        .p_2()
        .rounded(p.radius.sm)
        .bg(bg_color)
        .border_1()
        .border_color(border_color)
        .cursor_pointer()
        .hover(|style| style.bg(p.muted.opacity(0.4)))
        .items_center()
        .justify_between()
        .gap_2()
        .on_click(crate::dialog_host::on_event(
            app,
            move |this, _: &ClickEvent, _, cx| {
                this.select_audit_log(idx, cx);
            },
        ))
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
            Button::new(ElementId::Name(format!("delete-audit-log-{idx}").into()))
                .ghost()
                .xsmall()
                .icon(IconName::Trash)
                .tooltip(rust_i18n::t!("common.delete").to_string())
                .on_click(crate::dialog_host::on_event(
                    app,
                    move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.delete_audit_log(item_delete_idx, cx);
                    },
                )),
        )
        .into_any_element()
}
