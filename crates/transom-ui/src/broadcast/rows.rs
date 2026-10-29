//! 广播目标下拉列表渲染与行视图装配。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{Connection, TabId};

/// 目标选择列表里的一行快照；下拉内容点开时才构建，闭包内不再借用 `AppShell`。
pub(super) struct BroadcastTargetRow {
    pub(super) tab_id: TabId,
    pub(super) title: String,
    pub(super) dot_color: Hsla,
    pub(super) icon: IconName,
    pub(super) selected: bool,
}

/// 目标下拉列表：复选框常驻，点选后不关闭，方便连续多选。
pub(super) fn render_target_rows(
    app: &Entity<AppShell>,
    rows: Vec<BroadcastTargetRow>,
    p: &Palette,
) -> AnyElement {
    let mut list = v_flex().min_w(px(240.0)).gap_1();
    if rows.is_empty() {
        list = list.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("broadcast.no_targets").to_string()),
        );
    }
    for row in rows {
        let tab_id = row.tab_id;
        let app = app.clone();
        list = list.child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .size(px(7.0))
                        .rounded_full()
                        .flex_none()
                        .bg(row.dot_color),
                )
                .child(Icon::new(row.icon).xsmall().text_color(p.muted_foreground))
                .child(
                    Checkbox::new(ElementId::Name(format!("broadcast-target-{tab_id}").into()))
                        .small()
                        .checked(row.selected)
                        .label(row.title)
                        .on_click(move |&checked, _window, cx| {
                            app.update(cx, |this, cx| {
                                this.set_broadcast_target(tab_id, checked, cx);
                            });
                        }),
                ),
        );
    }
    div()
        .max_h(px(320.0))
        .overflow_y_scrollbar()
        .child(list)
        .into_any_element()
}

pub(super) fn connection_icon(connection: &Connection) -> IconName {
    match connection {
        Connection::Local(_) => IconName::Terminal,
        Connection::Serial(_) => IconName::Cable,
        Connection::Telnet(_) => IconName::Network,
        Connection::Mosh(_) => IconName::Radio,
        _ => IconName::Server,
    }
}
