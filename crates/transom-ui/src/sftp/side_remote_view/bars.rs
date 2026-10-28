//! SFTP 挂载按钮及各列紧凑折叠条渲染组件。
//!
//! 设计原理（Why）：
//! - 实现初衷：将并列远端挂载弹窗菜单与列折叠竖条（本地/主远端/并列远端）从超长视图文件中抽离；
//! - 核心优势：
//!   1. 降低单文件代码行数与圈复杂度；
//!   2. 挂载弹窗逻辑独立，便于统一维护已连接会话的动态筛选规则；
//!   3. 折叠竖条保持 32px 统一视觉契约。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::sftp::extract_candidate_path;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Icon, Selectable, Sizable};
use gpui_kit::*;
use std::collections::HashSet;
use transom_core::{Connection, ConnectionId, TabId};

impl AppShell {
    /// 渲染挂载并列远端按钮及其 Popover 菜单。
    pub(crate) fn render_side_remote_mount_button(
        &self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (open, mounted_conn_ids) = self
            .workspace
            .tabs
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.sftp())
            .map(|p| {
                let mounted: Vec<ConnectionId> =
                    p.side_remotes.iter().map(|s| s.connection_id).collect();
                (p.mount_menu_open, mounted)
            })
            .unwrap_or((false, Vec::new()));

        let app_open = cx.entity().clone();
        let trigger = Button::new(format!("sftp-mount-side-remote-btn-{tab_id}"))
            .ghost()
            .xsmall()
            .icon(IconName::Plus)
            .selected(open)
            .tooltip("并列挂载其他已连接会话");

        // 收集当前标签页的连接与排除信息
        let current_tab = self.workspace.tabs.iter().find(|t| t.id == tab_id);
        let current_conn_id = current_tab.and_then(|t| t.connection_id);
        let current_session_id = current_tab.map(|t| t.session.id);
        let is_local_only = current_tab
            .and_then(|t| t.sftp())
            .map(|p| p.local.only)
            .unwrap_or(false);

        let mut seen_sessions = HashSet::new();
        // 如果当前标签页是远端会话（非纯本地模式），排除当前自身所属的会话
        if !is_local_only && let Some(sid) = current_session_id {
            seen_sessions.insert(sid);
        }

        // 收集已建立连接的其他远端标签页，并按会话去重与排除已挂载的会话
        let mut active_candidates: Vec<(ConnectionId, String, String)> = Vec::new();
        for t in &self.workspace.tabs {
            if t.id == tab_id {
                continue;
            }
            let Some(conn_id) = t.connection_id else {
                continue;
            };
            if Some(conn_id) == current_conn_id {
                continue;
            }
            if mounted_conn_ids.contains(&conn_id) {
                continue;
            }
            // 本地终端会话不提供 SFTP 远端服务
            if matches!(&t.session.connection, Connection::Local(_)) {
                continue;
            }
            // 针对相同会话 ID 进行去重，避免同一会话打开多个终端标签时在列表中重复出现
            if !seen_sessions.insert(t.session.id) {
                continue;
            }

            let display_name = t.display_name();
            let initial_path = t
                .sftp()
                .map(|p| p.remote.path.clone())
                .filter(|p| !p.is_empty() && p != ".")
                .or_else(|| extract_candidate_path(&t.title))
                .unwrap_or_else(|| ".".to_string());
            active_candidates.push((conn_id, display_name, initial_path));
        }

        let app_menu = cx.entity().clone();
        let palette = p.clone();

        Popover::new(ElementId::Name(
            format!("sftp-mount-popover-{tab_id}").into(),
        ))
        .anchor(Anchor::TopRight)
        .open(open)
        .on_open_change(move |is_open, _window, cx| {
            let is_open = *is_open;
            app_open.update(cx, |this, cx| {
                if let Some(panel) = this.sftp_panel_mut_tab(tab_id)
                    && panel.mount_menu_open != is_open
                {
                    panel.mount_menu_open = is_open;
                    cx.notify();
                }
            });
        })
        .trigger(trigger)
        .content(move |_window, _cx, _area| {
            let mut menu = v_flex()
                .min_w(px(240.0))
                .max_h(px(320.0))
                .overflow_hidden()
                .p_1()
                .gap_1()
                .bg(palette.popover)
                .text_color(palette.popover_foreground)
                .rounded(palette.radius.md)
                .border_1()
                .border_color(palette.border);

            if active_candidates.is_empty() {
                return menu
                    .p_3()
                    .items_center()
                    .text_xs()
                    .text_color(palette.muted_foreground)
                    .child("暂无其他已连接的远端会话")
                    .into_any_element();
            }

            menu = menu.child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.muted_foreground)
                    .child("已连接的会话 (点击挂载)"),
            );
            for (conn_id, display_name, initial_path) in active_candidates.clone() {
                let app = app_menu.clone();
                let title_str = display_name.clone();
                let path_str = initial_path.clone();
                menu = menu.child(
                    h_flex()
                        .id(ElementId::Name(
                            format!("mount-candidate-{}", conn_id).into(),
                        ))
                        .px_2()
                        .py_1p5()
                        .gap_2()
                        .items_center()
                        .rounded(palette.radius.sm)
                        .cursor_pointer()
                        .hover(|style| {
                            style
                                .bg(palette.sidebar_accent)
                                .text_color(palette.sidebar_accent_foreground)
                        })
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.sftp_mount_side_remote(
                                    tab_id,
                                    conn_id,
                                    title_str.clone(),
                                    path_str.clone(),
                                    window,
                                    cx,
                                );
                            });
                        })
                        .child(Icon::new(IconName::Server).small())
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .truncate()
                                .child(display_name),
                        ),
                );
            }
            menu.into_any_element()
        })
        .into_any_element()
    }

    /// 渲染本地文件列折叠后的 32px 竖条。
    pub(crate) fn render_sftp_local_collapsed_bar(
        &self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expand_btn = Button::new("expand-sftp-local-btn")
            .ghost()
            .xsmall()
            .icon(IconName::PanelLeftOpen)
            .tooltip("展开本地文件列")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_toggle_local_collapse(tab_id, cx);
            }));

        v_flex()
            .w(px(32.0))
            .h_full()
            .items_center()
            .py_2()
            .gap_2()
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.sidebar_border)
            .child(expand_btn)
            .child(
                Icon::new(IconName::Laptop)
                    .small()
                    .text_color(p.muted_foreground),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.muted_foreground)
                    .child("本地"),
            )
            .into_any_element()
    }

    /// 渲染主远端列折叠后的 32px 紧凑竖条。
    pub(crate) fn render_sftp_remote_collapsed_bar(
        &self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expand_btn = Button::new("expand-sftp-remote-btn")
            .ghost()
            .xsmall()
            .icon(IconName::PanelLeftOpen)
            .tooltip("展开主远端列")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_toggle_remote_collapse(tab_id, cx);
            }));

        v_flex()
            .w(px(32.0))
            .h_full()
            .items_center()
            .py_2()
            .gap_2()
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.sidebar_border)
            .child(expand_btn)
            .child(
                Icon::new(IconName::Server)
                    .small()
                    .text_color(p.muted_foreground),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.muted_foreground)
                    .child("远端"),
            )
            .into_any_element()
    }

    /// 渲染单个并列远端列折叠后的 32px 紧凑竖条。
    pub(crate) fn render_sftp_side_remote_collapsed_bar(
        &self,
        tab_id: TabId,
        side_index: usize,
        title: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expand_btn = Button::new(format!("expand-side-{side_index}"))
            .ghost()
            .xsmall()
            .icon(IconName::PanelRightOpen)
            .tooltip("展开远端列")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_toggle_side_remote_collapse(tab_id, side_index, cx);
            }));

        let close_btn = Button::new(format!("close-side-{side_index}"))
            .ghost()
            .xsmall()
            .icon(IconName::X)
            .tooltip("关闭并列远端")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_close_side_remote(tab_id, side_index, cx);
            }));

        v_flex()
            .w(px(32.0))
            .h_full()
            .items_center()
            .py_2()
            .gap_2()
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.sidebar_border)
            .child(expand_btn)
            .child(close_btn)
            .child(
                Icon::new(IconName::Server)
                    .small()
                    .text_color(p.muted_foreground),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.muted_foreground)
                    .child(title.chars().take(2).collect::<String>()),
            )
            .into_any_element()
    }
}
