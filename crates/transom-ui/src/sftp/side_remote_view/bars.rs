//! SFTP 挂载按钮及各列紧凑折叠条渲染组件。
//!
//! 设计原理（Why）：
//! - 实现初衷：将并列远端挂载弹窗菜单与列折叠竖条（本地/主远端/并列远端）从超长视图文件中抽离；
//! - 核心优势：
//!   1. 降低单文件代码行数与圈复杂度；
//!   2. 挂载弹窗逻辑独立，便于统一维护已连接会话的动态筛选规则；
//!   3. 折叠竖条保持 32px 统一视觉契约。

use crate::palette::Palette;
use crate::sftp::entity::SftpPanelView;
use crate::sftp::normalize_remote_cwd;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Icon, Selectable, Sizable};
use gpui_kit::*;
use transom_core::ConnectionId;

impl SftpPanelView {
    /// 渲染挂载并列远端按钮及其 Popover 菜单。
    pub(crate) fn render_side_remote_mount_button(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = self.tab_id;
        let open = self.state.mount_menu_open;
        let mounted_conn_ids: Vec<ConnectionId> = self
            .state
            .side_remotes
            .iter()
            .filter_map(|side| side.connection_id)
            .collect();

        let app_open = cx.entity().clone();
        let trigger = Button::new(format!("sftp-mount-side-remote-btn-{tab_id}"))
            .ghost()
            .xsmall()
            .icon(IconName::Plus)
            .selected(open)
            .tooltip(rust_i18n::t!("sftp.sidebar.mount_tooltip").to_string());

        // 候选由外壳在标签变化时推送：渲染期不读工作区。
        let candidates = self.state.mount_candidates.clone();
        let mut active_candidates: Vec<(ConnectionId, String, String)> = Vec::new();
        for candidate in candidates {
            if mounted_conn_ids.contains(&candidate.connection_id) {
                continue;
            }
            let initial_path = candidate
                .view
                .map(|view| view.read(cx).state.remote.path.clone())
                .filter(|p| !p.is_empty() && p != ".")
                .or_else(|| candidate.cwd.as_deref().and_then(normalize_remote_cwd))
                .unwrap_or_else(|| ".".to_string());
            active_candidates.push((
                candidate.connection_id,
                candidate.display_name,
                initial_path,
            ));
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
            app_open.update(cx, |view, cx| {
                if view.state.mount_menu_open != is_open {
                    view.state.mount_menu_open = is_open;
                }
                cx.notify();
            });
        })
        .trigger(trigger)
        .content(move |_window, _cx, _area| {
            mount_menu(active_candidates.clone(), app_menu.clone(), palette.clone())
        })
        .into_any_element()
    }

    /// 渲染本地文件列折叠后的 32px 竖条。
    pub(crate) fn render_sftp_local_collapsed_bar(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expand_btn = crate::widgets::IconButton::new(
            "expand-sftp-local-btn",
            IconName::PanelLeftOpen,
            rust_i18n::t!("sftp.sidebar.expand_local").to_string(),
        )
        .size(crate::widgets::IconButtonSize::Compact)
        .render()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_local_collapse(cx);
        }));

        v_flex()
            .w(px(32.0))
            .h_full()
            .items_center()
            .py_2()
            .gap_2()
            .bg(p.sidebar)
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
                    .child(rust_i18n::t!("sftp.local_title").to_string()),
            )
            .into_any_element()
    }

    /// 渲染主远端列折叠后的 32px 紧凑竖条。
    pub(crate) fn render_sftp_remote_collapsed_bar(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expand_btn = crate::widgets::IconButton::new(
            "expand-sftp-remote-btn",
            IconName::PanelLeftOpen,
            rust_i18n::t!("sftp.sidebar.expand_remote").to_string(),
        )
        .size(crate::widgets::IconButtonSize::Compact)
        .render()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_remote_collapse(cx);
        }));

        v_flex()
            .w(px(32.0))
            .h_full()
            .items_center()
            .py_2()
            .gap_2()
            .bg(p.sidebar)
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
                    .child(rust_i18n::t!("sftp.sidebar.remote_column").to_string()),
            )
            .into_any_element()
    }

    /// 渲染单个并列远端列折叠后的 32px 紧凑竖条。
    pub(crate) fn render_sftp_side_remote_collapsed_bar(
        &self,
        side_index: usize,
        title: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expand_btn = crate::widgets::IconButton::new(
            format!("expand-side-{side_index}"),
            IconName::PanelRightOpen,
            rust_i18n::t!("sftp.sidebar.expand_side_remote").to_string(),
        )
        .size(crate::widgets::IconButtonSize::Compact)
        .render()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_side_remote_collapse(side_index, cx);
        }));

        let close_btn = crate::widgets::IconButton::new(
            format!("close-side-{side_index}"),
            IconName::X,
            rust_i18n::t!("sftp.sidebar.close_side_remote").to_string(),
        )
        .size(crate::widgets::IconButtonSize::Compact)
        .render()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.close_side_remote(side_index, cx);
        }));

        v_flex()
            .w(px(32.0))
            .h_full()
            .items_center()
            .py_2()
            .gap_2()
            .bg(p.sidebar)
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

/// 并列远端挂载弹层：候选为空时的提示与逐连接入口。
fn mount_menu(
    candidates: Vec<(ConnectionId, String, String)>,
    app: Entity<SftpPanelView>,
    palette: Palette,
) -> AnyElement {
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

    if candidates.is_empty() {
        return menu
            .p_3()
            .items_center()
            .text_xs()
            .text_color(palette.muted_foreground)
            .child(rust_i18n::t!("sftp.sidebar.no_candidates").to_string())
            .into_any_element();
    }

    menu = menu.child(
        div()
            .px_2()
            .py_1()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette.muted_foreground)
            .child(rust_i18n::t!("sftp.sidebar.candidates_header").to_string()),
    );
    for (conn_id, display_name, initial_path) in candidates.clone() {
        let app = app.clone();
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
                        this.mount_side_remote(
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
}
