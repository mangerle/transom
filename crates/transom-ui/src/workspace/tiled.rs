//! 多标签并排渲染：按分组栈铺满网格，每条分隔线都可拖拽。
//!
//! 每个槽位仍调用 `render_group_content`，因此标签组内部的窗格分屏、最大化
//! 与拖拽比例在并排视图里原样生效；空槽显示“选择标签”占位，点开后可以把
//! 尚未并排的独立标签填进来。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::tiling::{TiledTabs, TilingId, TilingNode};
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_kit::*;
use transom_core::{Connection, GroupId, SplitAxis};

impl AppShell {
    /// 渲染并排网格；没有任何有效槽位时返回 `None`，由调用方退回单标签。
    pub(super) fn render_tiled_content(
        &mut self,
        tiled: &TiledTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> Option<AnyElement> {
        if tiled.occupied_count() == 0 {
            return None;
        }
        let body = self.render_tiling_node(&tiled.root, tiled, window, cx, p);
        Some(
            div()
                .flex_1()
                .size_full()
                .overflow_hidden()
                .child(body)
                .into_any_element(),
        )
    }

    /// 递归渲染网格：同一方向的 N 个格子交给一个可拖拽容器等分。
    fn render_tiling_node(
        &mut self,
        node: &TilingNode,
        tiled: &TiledTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        match node {
            TilingNode::Slot(slot) => match tiled.group_at(*slot) {
                Some(group) => {
                    let Some(index) = self
                        .workspace
                        .groups
                        .iter()
                        .position(|item| item.id == group)
                    else {
                        return div().into_any_element();
                    };
                    self.render_tiled_zone(index, group, *slot == tiled.focused, window, cx, p)
                }
                None => self.render_empty_tiling_slot(tiled.id, *slot, cx, p),
            },
            TilingNode::Stack { id, axis, children } => {
                let child_elements: Vec<AnyElement> = children
                    .iter()
                    .map(|child| self.render_tiling_node(child, tiled, window, cx, p))
                    .collect();
                // 同一方向的所有格子放进一个可拖拽容器：初始等分，拖动任一分隔线
                // 只影响相邻两格；状态与窗格分屏共用同一张表。
                let state = self.pane_split_state(*id, cx);
                let mut group = match axis {
                    SplitAxis::Horizontal => h_resizable(super::panes::split_element_id(*id)),
                    SplitAxis::Vertical => v_resizable(super::panes::split_element_id(*id)),
                }
                .with_state(&state);
                for child in child_elements {
                    group = group.child(resizable_panel().child(child));
                }
                div().flex_1().size_full().child(group).into_any_element()
            }
        }
    }

    /// 渲染并排中的一个标签格：细条标题（状态、标题、关闭）加标签组内容。
    fn render_tiled_zone(
        &mut self,
        group_index: usize,
        group_id: GroupId,
        focused: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let (pane_id, serial) = {
            let group = &self.workspace.groups[group_index];
            (group.active_pane, group.serial)
        };
        let (title, dot_color, tab_icon) =
            match self.workspace.tabs.iter().find(|tab| tab.id == pane_id) {
                Some(tab) => {
                    let (dot_color, _) = super::pane_status_indicator(&tab.status, p);
                    let mut title = tab.display_name();
                    if serial > 0 {
                        title = format!("{title} #{serial}");
                    }
                    let tab_icon = match &tab.session.connection {
                        Connection::Local(_) => IconName::Terminal,
                        Connection::Serial(_) => IconName::Cable,
                        Connection::Telnet(_) => IconName::Network,
                        _ => IconName::Server,
                    };
                    (title, dot_color, tab_icon)
                }
                None => (String::new(), p.muted_foreground, IconName::Terminal),
            };

        let header = super::pane_header::pane_header(
            super::pane_header::PaneHeaderProps {
                id: format!("tiled-zone-header-{group_id}"),
                close_id: format!("tiled-zone-close-{group_id}"),
                active: focused,
                dot_color,
                icon: tab_icon,
                title,
                ordinal: None,
                palette: p,
            },
            cx.listener(move |this, _, window, cx| {
                this.activate_group(group_id, window, cx);
            }),
            cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.close_group(group_id, window, cx);
            }),
        );

        let body = self.render_group_content(group_index, window, cx, p);
        v_flex()
            .size_full()
            .overflow_hidden()
            .child(header)
            .child(body)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.activate_group(group_id, window, cx);
                }),
            )
            .into_any_element()
    }

    /// 空槽占位：把尚未并排的独立标签填进这个格子。
    fn render_empty_tiling_slot(
        &mut self,
        tiling_id: TilingId,
        slot: usize,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let mut rows: Vec<(GroupId, String)> = Vec::new();
        for group_id in self.available_tiling_groups() {
            let Some(group) = self
                .workspace
                .groups
                .iter()
                .find(|group| group.id == group_id)
            else {
                continue;
            };
            let Some(tab) = self
                .workspace
                .tabs
                .iter()
                .find(|tab| tab.id == group.active_pane)
            else {
                continue;
            };
            let mut title = tab.display_name();
            if group.serial > 0 {
                title = format!("{title} #{}", group.serial);
            }
            rows.push((group_id, title));
        }
        let has_rows = !rows.is_empty();
        let text = if has_rows {
            rust_i18n::t!("tiling.empty_slot").to_string()
        } else {
            rust_i18n::t!("tiling.no_available").to_string()
        };
        let mut placeholder = v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(p.tab_bar)
            .border_1()
            .border_dashed()
            .border_color(p.border)
            .child(div().text_xs().text_color(p.muted_foreground).child(text));
        if has_rows {
            let app = cx.entity().clone();
            let trigger = Button::new(ElementId::Name(
                format!("fill-tiling-slot-{tiling_id}-{slot}").into(),
            ))
            .ghost()
            .small()
            .icon(IconName::Plus)
            .label(rust_i18n::t!("tiling.fill_slot").to_string())
            .dropdown_menu(move |menu, _window, _cx| {
                let mut menu = menu;
                for (group_id, title) in rows.iter() {
                    let group_id = *group_id;
                    let title = title.clone();
                    let app = app.clone();
                    menu = menu.item(PopupMenuItem::new(title).on_click(move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.fill_tiling_slot(tiling_id, slot, group_id, window, cx);
                        });
                    }));
                }
                menu
            });
            placeholder = placeholder.child(trigger);
        }
        placeholder.into_any_element()
    }
}
