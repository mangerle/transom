//! 标签栏渲染。

use crate::app::{AppShell, TabStatus};
use crate::palette::Palette;
use crate::theme::tokens::size;
use crate::tiling::TiledTabs;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{Connection, GroupId, SplitAxis, TabId};

use super::tab_drag::attach_tab_drag;

impl AppShell {
    pub(super) fn render_tab_strip(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut tab_strip = h_flex()
            .h(rems(size::TAB_BAR_HEIGHT))
            .items_center()
            .bg(p.tab_bar)
            .border_b_1()
            .border_color(p.border)
            .overflow_hidden();
        let app_entity = cx.entity().clone();
        // 并排菜单在每个标签上都显示：只有一个标签时置灰，避免用户以为功能
        // 不存在；右键活动标签时由 `tile_tab` 取标签栏里紧随其后的标签作搭档。
        let tiled_view = self.tiled_tabs.is_some();
        let can_tile = self.groups.len() >= 2;
        // 并排对在标签栏里只占一个条目：位置取对里最靠前的标签，内容取焦点标签。
        let entries = self.tab_strip_entries();

        for (group_index, composite) in entries {
            let group = &self.groups[group_index];
            let Some(tab) = self.tabs.iter().find(|tab| tab.id == group.active_pane) else {
                continue;
            };
            let group_id = group.id;
            let active_pane = group.active_pane;
            let session_id = tab.session_id;
            let title = tab.display_name();
            let pane_count = group.pane_count();
            let root_axis = group.root.root_axis();
            // 只有同会话的额外标签才带后缀，原始标签（serial 为 0）始终不带：
            // 新开标签只给自己贴号，绝不回改已经存在的标签。
            // 序号不拼进 tab.title —— 那个字段会被远端 OSC 标题序列覆盖，
            // 而且 SFTP 目录联动靠解析它。
            let serial = group.serial;
            let is_extra = serial > 0;
            let is_local = matches!(tab.session.connection, Connection::Local(_));
            let active = self.active_tab == Some(active_pane);
            // 并排对里的标签给一条弱下划线：配对仍在，即使当前单屏查看别的
            // 标签也能看到这对搭档；焦点标签再用强调色压住它。合并条目本身
            // 就代表整个并排对，因此始终按"配对"处理。
            let tiled_paired = composite
                || self
                    .tiled_tabs
                    .is_some_and(|tiled| tiled.contains(group_id));
            let dot_color = match &tab.status {
                TabStatus::Connecting => p.muted_foreground,
                TabStatus::Connected => p.success,
                TabStatus::Failed(_) | TabStatus::Disconnected => p.danger,
            };
            // 传输中的标签把状态点换成主题强调色：状态栏徽章只跟随活跃标签，
            // 用户切走后后台仍在跑的传输需要在这里保持可见。
            let dot_color = if tab.zmodem.is_some() {
                p.primary
            } else {
                dot_color
            };
            let tab_icon = connection_icon(&tab.session.connection);
            let mut tab_element = h_flex()
                .id(ElementId::Name(format!("tab-{group_id}").into()))
                .relative()
                .h_full()
                .px_2p5()
                .gap_1p5()
                .items_center()
                .max_w(rems(size::TAB_MAX_WIDTH))
                .min_w(rems(size::TAB_MIN_WIDTH))
                .overflow_hidden()
                .border_r_1()
                .border_color(p.border)
                .cursor_pointer();
            if active {
                tab_element = tab_element
                    .bg(p.tab_active)
                    .text_color(p.tab_active_foreground);
            } else {
                tab_element = tab_element.text_color(p.tab_foreground).hover(|style| {
                    style
                        .bg(p.sidebar_accent.opacity(0.6))
                        .text_color(p.foreground)
                });
            }
            let labeled = if is_extra {
                format!("{title} #{serial}")
            } else {
                title.clone()
            };
            let tooltip_title = if pane_count > 1 {
                rust_i18n::t!("tab.pane_count", title = labeled, count = pane_count).to_string()
            } else {
                labeled
            };
            // 标签主名固定用会话名，远端 OSC 上报的运行期标题（通常是 `用户@主机: 目录`
            // 或当前编辑器文件）只在 tooltip 第二行附带，且与主名重复时不显示。
            let runtime_title = tab.title.trim().to_string();
            let show_runtime = !runtime_title.is_empty() && runtime_title != tab.display_name();
            let tooltip_muted = p.muted_foreground;
            // 合并条目代表整个并排对，拖动会只带走其中一个标签，暂不提供换序。
            let tab_element = if composite {
                tab_element
            } else {
                attach_tab_drag(
                    tab_element,
                    group_id,
                    title.clone(),
                    tab_icon,
                    &app_entity,
                    p,
                )
            };
            let mut element = tab_element
                .tooltip(move |_window, cx| {
                    // 外层闭包会被反复调用，字符串必须每轮重新交出所有权。
                    let tooltip_title = tooltip_title.clone();
                    let runtime_title = runtime_title.clone();
                    Tooltip::element(move |_, _| {
                        let mut tip = v_flex()
                            .gap_0p5()
                            .child(div().text_sm().child(tooltip_title.clone()));
                        if show_runtime {
                            tip = tip.child(
                                div()
                                    .text_xs()
                                    .max_w(px(360.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .truncate()
                                    .text_color(tooltip_muted)
                                    .child(runtime_title.clone()),
                            );
                        }
                        tip
                    })
                    .build(_window, cx)
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.activate_tab(active_pane, window, cx);
                }))
                .context_menu({
                    let app_entity = app_entity.clone();
                    move |menu, _window, _cx| {
                        let app = app_entity.clone();
                        let mut menu = menu
                            .item(
                                PopupMenuItem::new(rust_i18n::t!("menu.reconnect").to_string())
                                    .icon(IconName::RefreshCw)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.reconnect_tab(active_pane, window, cx);
                                            });
                                        }
                                    }),
                            )
                            .item(
                                PopupMenuItem::new(rust_i18n::t!("menu.duplicate").to_string())
                                    .icon(IconName::Plus)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.activate_tab(active_pane, window, cx);
                                                if is_local {
                                                    this.open_local_terminal(window, cx);
                                                } else {
                                                    this.connect_session(session_id, window, cx);
                                                }
                                            });
                                        }
                                    }),
                            )
                            .item(PopupMenuItem::separator())
                            .item(
                                PopupMenuItem::new(rust_i18n::t!("menu.split_right").to_string())
                                    .icon(IconName::Columns2)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.activate_tab(active_pane, window, cx);
                                                this.split_active_pane(
                                                    SplitAxis::Horizontal,
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }
                                    }),
                            )
                            .item(
                                PopupMenuItem::new(rust_i18n::t!("menu.split_down").to_string())
                                    .icon(IconName::Rows2)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.activate_tab(active_pane, window, cx);
                                                this.split_active_pane(
                                                    SplitAxis::Vertical,
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }
                                    }),
                            );
                        if pane_count > 1 {
                            menu = menu
                                .item(
                                    PopupMenuItem::new(
                                        rust_i18n::t!("menu.toggle_split_axis").to_string(),
                                    )
                                    .icon(IconName::ArrowLeftRight)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.activate_tab(active_pane, window, cx);
                                                this.toggle_split_axis(cx);
                                            });
                                        }
                                    }),
                                )
                                .item(
                                    PopupMenuItem::new(
                                        rust_i18n::t!("menu.detach_pane").to_string(),
                                    )
                                    .icon(IconName::ArrowUpRight)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.activate_tab(active_pane, window, cx);
                                                this.detach_pane_to_tab(active_pane, window, cx);
                                            });
                                        }
                                    }),
                                )
                                .item(
                                    PopupMenuItem::new(
                                        rust_i18n::t!("menu.close_pane").to_string(),
                                    )
                                    .icon(IconName::X)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.close_pane(active_pane, window, cx);
                                            });
                                        }
                                    }),
                                );
                        }
                        menu = menu
                            .item(PopupMenuItem::separator())
                            .item(
                                PopupMenuItem::new(rust_i18n::t!("menu.tile_right").to_string())
                                    .icon(IconName::Columns2)
                                    .disabled(!can_tile)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.tile_tab(
                                                    group_id,
                                                    SplitAxis::Horizontal,
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }
                                    }),
                            )
                            .item(
                                PopupMenuItem::new(rust_i18n::t!("menu.tile_down").to_string())
                                    .icon(IconName::Rows2)
                                    .disabled(!can_tile)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.tile_tab(
                                                    group_id,
                                                    SplitAxis::Vertical,
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }
                                    }),
                            )
                            .item(
                                PopupMenuItem::new(rust_i18n::t!("menu.untile").to_string())
                                    .icon(IconName::Minimize2)
                                    .disabled(!tiled_view)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, _window, cx| {
                                            app.update(cx, |this, cx| this.untile_tabs(cx));
                                        }
                                    }),
                            );
                        menu.item(PopupMenuItem::separator())
                            .item(
                                PopupMenuItem::new(rust_i18n::t!("menu.close_tab").to_string())
                                    .icon(IconName::X)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.close_group(group_id, window, cx);
                                            });
                                        }
                                    }),
                            )
                            .item(
                                PopupMenuItem::new(
                                    rust_i18n::t!("menu.close_other_tabs").to_string(),
                                )
                                .icon(IconName::Trash)
                                .on_click({
                                    let app = app.clone();
                                    move |_, window, cx| {
                                        app.update(cx, |this, cx| {
                                            this.activate_tab(active_pane, window, cx);
                                            this.close_other_tabs(active_pane, window, cx);
                                        });
                                    }
                                }),
                            )
                    }
                })
                .child(div().size(px(8.0)).rounded_full().flex_none().bg(dot_color))
                .child(Icon::new(tab_icon).flex_none().text_color(if active {
                    p.tab_active_foreground
                } else {
                    p.muted_foreground
                }))
                .child(
                    div()
                        .text_sm()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .truncate()
                        .child(title.clone()),
                );
            if is_extra {
                element = element.child(
                    div()
                        .text_xs()
                        .flex_none()
                        .text_color(p.muted_foreground)
                        .child(format!("#{serial}")),
                );
            }
            if pane_count > 1 && !composite {
                // 徽标即窗格选择器：放大态下其他窗格不在界面上，这里是唯一的鼠标出口。
                let rows: Vec<PaneRow> = group
                    .pane_ids()
                    .into_iter()
                    .enumerate()
                    .filter_map(|(position, pane_id)| {
                        let tab = self.tabs.iter().find(|tab| tab.id == pane_id)?;
                        let (dot_color, icon) = super::pane_status_indicator(&tab.status, p);
                        Some(PaneRow {
                            tab_id: pane_id,
                            ordinal: position + 1,
                            title: tab.display_name(),
                            dot_color,
                            icon,
                            active: pane_id == active_pane,
                        })
                    })
                    .collect();
                let zoomed = group.zoomed.is_some();
                let muted = p.muted_foreground;
                let foreground = p.foreground;
                let badge_icon = if root_axis == SplitAxis::Horizontal {
                    IconName::Columns2
                } else {
                    IconName::Rows2
                };
                element = element.child(
                    Button::new(ElementId::Name(format!("pane-list-{group_id}").into()))
                        .ghost()
                        .xsmall()
                        .icon(badge_icon)
                        .label(pane_count.to_string())
                        .dropdown_menu({
                            let app_entity = app_entity.clone();
                            move |menu, _window, _cx| {
                                let mut menu = menu;
                                for row in rows.iter() {
                                    let tab_id = row.tab_id;
                                    let title = row.title.clone();
                                    let (dot_color, icon, active, ordinal) =
                                        (row.dot_color, row.icon, row.active, row.ordinal);
                                    menu = menu.item(
                                        PopupMenuItem::element(move |_window, _cx| {
                                            let mut row = h_flex()
                                                .gap_2()
                                                .items_center()
                                                .child(
                                                    div()
                                                        .size(px(7.0))
                                                        .rounded_full()
                                                        .flex_none()
                                                        .bg(dot_color),
                                                )
                                                .child(Icon::new(icon).xsmall().text_color(muted))
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .flex_none()
                                                        .text_color(muted)
                                                        .child(ordinal.to_string()),
                                                )
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .flex_1()
                                                        .overflow_hidden()
                                                        .whitespace_nowrap()
                                                        .truncate()
                                                        .child(title.clone()),
                                                );
                                            if active {
                                                row = row.child(
                                                    Icon::new(IconName::Check)
                                                        .xsmall()
                                                        .text_color(foreground),
                                                );
                                            }
                                            row
                                        })
                                        .on_click({
                                            let app = app_entity.clone();
                                            move |_, window, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.select_pane(tab_id, window, cx);
                                                });
                                            }
                                        }),
                                    );
                                }
                                if zoomed {
                                    menu = menu.item(PopupMenuItem::separator()).item(
                                        PopupMenuItem::new(
                                            rust_i18n::t!("menu.restore_panes").to_string(),
                                        )
                                        .icon(IconName::Minimize2)
                                        .on_click({
                                            let app = app_entity.clone();
                                            move |_, _window, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.toggle_zoom_pane(active_pane, cx)
                                                });
                                            }
                                        }),
                                    );
                                }
                                menu
                            }
                        }),
                );
            }
            if composite && let Some(tiled) = self.tiled_tabs {
                element = element.child(self.tiled_group_badge(tiled, &app_entity, p));
            }
            if self.settings.window.show_tab_close_button {
                element = element.child(
                    div()
                        .id(ElementId::Name(format!("close-{group_id}").into()))
                        .size(px(18.0))
                        .rounded(p.radius.sm)
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .text_color(if active {
                            p.tab_active_foreground
                        } else {
                            p.muted_foreground
                        })
                        .hover(|s| {
                            s.bg(p.muted_foreground.opacity(0.2))
                                .text_color(p.foreground)
                        })
                        .active(|s| s.bg(p.muted_foreground.opacity(0.3)))
                        .tooltip(move |_window, cx| {
                            Tooltip::new(rust_i18n::t!("app.close").to_string()).build(_window, cx)
                        })
                        .on_mouse_down(MouseButton::Left, |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_group(group_id, window, cx);
                        }))
                        .child(Icon::new(IconName::WindowClose).xsmall()),
                );
            }
            if active || tiled_paired {
                // 标签栏背景与激活色非常接近，仅靠背景几乎看不出焦点；用底边
                // 强调色补齐这个信号。绝对定位不参与横向布局，不挤压标签内容。
                let (height, color) = if active {
                    (px(2.0), p.primary)
                } else {
                    (px(1.0), p.muted_foreground.opacity(0.6))
                };
                element = element.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(height)
                        .bg(color),
                );
            }
            tab_strip = tab_strip.child(element);
        }

        tab_strip
            .child(
                div()
                    .h_full()
                    .px_1p5()
                    .flex()
                    .items_center()
                    .child(Self::tab_strip_button(
                        "tab-strip-quick-new",
                        IconName::Plus,
                        rust_i18n::t!("app.new_local").to_string(),
                        true,
                        p,
                        cx.listener(|this, _, window, cx| {
                            this.open_local_terminal(window, cx);
                        }),
                    )),
            )
            .into_any_element()
    }

    /// 标签栏要渲染的条目：并排对合并成一个条目，位置取对里最靠前的标签，
    /// 内容取当前焦点标签。返回 `(标签组下标, 是否为合并条目)`。
    ///
    /// 配对失效（任一组缺失、两组相同）时不合并，按普通标签逐个渲染，
    /// 与 `prune_tiling` 的兜底口径一致。
    fn tab_strip_entries(&self) -> Vec<(usize, bool)> {
        let singles = || {
            (0..self.groups.len())
                .map(|index| (index, false))
                .collect::<Vec<_>>()
        };
        let Some(tiled) = self.tiled_tabs else {
            return singles();
        };
        let first = self.groups.iter().position(|group| group.id == tiled.first);
        let second = self
            .groups
            .iter()
            .position(|group| group.id == tiled.second);
        let (Some(first), Some(second)) = (first, second) else {
            return singles();
        };
        if first == second {
            return singles();
        }
        let Some(focused) = self
            .groups
            .iter()
            .position(|group| group.id == tiled.group(tiled.focused))
        else {
            return singles();
        };
        let merge_position = first.min(second);
        let mut entries = Vec::with_capacity(self.groups.len().saturating_sub(1));
        for index in 0..self.groups.len() {
            if index == merge_position {
                entries.push((focused, true));
            } else if index == first || index == second {
                continue;
            } else {
                entries.push((index, false));
            }
        }
        entries
    }

    /// 合并条目的分组选择器：下拉列出并排对里的标签，可切换焦点或退出并排。
    ///
    /// 复用标签栏已有的状态点与连接图标；条目本身只占一个标签宽度，因此
    /// 组内多个标签不再各自占一格。
    fn tiled_group_badge(
        &self,
        tiled: TiledTabs,
        app_entity: &Entity<AppShell>,
        p: &Palette,
    ) -> AnyElement {
        let mut rows = Vec::with_capacity(2);
        for (position, group_id) in [tiled.first, tiled.second].into_iter().enumerate() {
            let Some(group) = self.groups.iter().find(|group| group.id == group_id) else {
                continue;
            };
            let Some(tab) = self.tabs.iter().find(|tab| tab.id == group.active_pane) else {
                continue;
            };
            let (dot_color, _) = super::pane_status_indicator(&tab.status, p);
            let mut title = tab.display_name();
            if group.serial > 0 {
                title = format!("{title} #{}", group.serial);
            }
            rows.push(TiledRow {
                group_id,
                ordinal: position + 1,
                title,
                dot_color,
                icon: connection_icon(&tab.session.connection),
                focused: tiled.group(tiled.focused) == group_id,
            });
        }
        let axis_icon = match tiled.axis {
            SplitAxis::Horizontal => IconName::Columns2,
            SplitAxis::Vertical => IconName::Rows2,
        };
        let count = rows.len();
        let muted = p.muted_foreground;
        let foreground = p.foreground;
        Button::new(ElementId::Name(
            format!("tiled-list-{}", tiled.first).into(),
        ))
        .ghost()
        .xsmall()
        .icon(axis_icon)
        .label(count.to_string())
        .dropdown_menu({
            let app_entity = app_entity.clone();
            move |menu, _window, _cx| {
                let mut menu = menu;
                for row in rows.iter() {
                    let group_id = row.group_id;
                    let title = row.title.clone();
                    let (dot_color, icon, focused, ordinal) =
                        (row.dot_color, row.icon, row.focused, row.ordinal);
                    menu = menu.item(
                        PopupMenuItem::element(move |_window, _cx| {
                            let mut row = h_flex()
                                .gap_2()
                                .items_center()
                                .child(div().size(px(7.0)).rounded_full().flex_none().bg(dot_color))
                                .child(Icon::new(icon).xsmall().text_color(muted))
                                .child(
                                    div()
                                        .text_xs()
                                        .flex_none()
                                        .text_color(muted)
                                        .child(ordinal.to_string()),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .flex_1()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .truncate()
                                        .child(title.clone()),
                                );
                            if focused {
                                row = row.child(
                                    Icon::new(IconName::Check).xsmall().text_color(foreground),
                                );
                            }
                            row
                        })
                        .on_click({
                            let app = app_entity.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.activate_group(group_id, window, cx);
                                });
                            }
                        }),
                    );
                }
                menu.item(PopupMenuItem::separator()).item(
                    PopupMenuItem::new(rust_i18n::t!("menu.untile").to_string())
                        .icon(IconName::Minimize2)
                        .on_click({
                            let app = app_entity.clone();
                            move |_, _window, cx| {
                                app.update(cx, |this, cx| this.untile_tabs(cx));
                            }
                        }),
                )
            }
        })
        .into_any_element()
    }

    fn tab_strip_button(
        id: &'static str,
        icon: IconName,
        tooltip: String,
        enabled: bool,
        p: &Palette,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let mut button = div()
            .id(id)
            .size(px(22.0))
            .rounded(p.radius.sm)
            .flex()
            .items_center()
            .justify_center()
            .tooltip(move |_window, cx| Tooltip::new(tooltip.clone()).build(_window, cx));
        if enabled {
            button = button
                .cursor_pointer()
                .text_color(p.muted_foreground)
                .hover(|s| s.bg(p.sidebar_accent).text_color(p.foreground))
                .active(|s| s.bg(p.border))
                .on_click(on_click);
        } else {
            button = button
                .cursor_not_allowed()
                .text_color(p.muted_foreground.opacity(0.4));
        }
        button.child(Icon::new(icon).xsmall()).into_any_element()
    }
}

/// 窗格列表里的一行预渲染数据。
///
/// 下拉内容是点开时才构建的，因此这里先把标题、状态标识等取成拥有值，
/// 闭包内不再借用 `AppShell`。
struct PaneRow {
    tab_id: TabId,
    ordinal: usize,
    title: String,
    dot_color: Hsla,
    icon: IconName,
    active: bool,
}

/// 合并条目下拉里的一行：一个并排标签组的预渲染数据。
struct TiledRow {
    group_id: GroupId,
    ordinal: usize,
    title: String,
    dot_color: Hsla,
    icon: IconName,
    focused: bool,
}

/// 标签栏与并排分组下拉共用的连接类型图标。
fn connection_icon(connection: &Connection) -> IconName {
    match connection {
        Connection::Local(_) => IconName::Terminal,
        Connection::Serial(_) => IconName::Cable,
        Connection::Telnet(_) => IconName::Network,
        _ => IconName::Server,
    }
}
