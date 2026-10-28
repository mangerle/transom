//! SFTP 并列远端列头部导航与操作工具栏渲染。
//!
//! 设计原理（Why）：
//! - 实现初衷：将并列远端面板顶部的路径输入、导航跳转、刷新及下载按钮单独隔离；
//! - 核心优势：使并列远端主视图关注虚拟列表和拖拽事件。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{ConnectionId, TabId};

/// 并列远端头部工具栏渲染参数。
pub(super) struct SideRemoteHeaderProps<'a> {
    pub(super) tab_id: TabId,
    pub(super) side_index: usize,
    pub(super) connection_id: ConnectionId,
    pub(super) title: String,
    pub(super) path: String,
    pub(super) path_input: &'a Entity<InputState>,
}

/// 渲染并列远端面板头部导航与工具栏。
pub(super) fn render_side_remote_header(
    props: SideRemoteHeaderProps<'_>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let tab_id = props.tab_id;
    let side_index = props.side_index;
    let connection_id = props.connection_id;
    let title = props.title;
    let path = props.path;
    let path_input = props.path_input;

    let collapse_btn = Button::new(format!("collapse-side-{side_index}"))
        .ghost()
        .xsmall()
        .icon(IconName::PanelRightClose)
        .tooltip("收起此列")
        .on_click(cx.listener(move |this, _, _, cx| {
            this.sftp_toggle_side_remote_collapse(tab_id, side_index, cx);
        }));

    let close_btn = Button::new(format!("close-side-expanded-{side_index}"))
        .ghost()
        .xsmall()
        .icon(IconName::X)
        .tooltip("关闭并列远端")
        .on_click(cx.listener(move |this, _, _, cx| {
            this.sftp_close_side_remote(tab_id, side_index, cx);
        }));

    let path_val = path.clone();
    let up_btn = Button::new(format!("side-up-{side_index}"))
        .ghost()
        .xsmall()
        .icon(IconName::ArrowUp)
        .tooltip("上一级")
        .on_click(cx.listener(move |this, _, _, cx| {
            let parent = if path_val == "/" {
                "/".to_string()
            } else {
                let trimmed = path_val.trim_end_matches('/');
                match trimmed.rsplit_once('/') {
                    Some(("", _)) => "/".to_string(),
                    Some((p, _)) => p.to_string(),
                    None => "/".to_string(),
                }
            };
            this.request_sftp_list_for_connection(tab_id, connection_id, parent, cx);
        }));

    let cur_path = path;
    let refresh_btn = Button::new(format!("side-refresh-{side_index}"))
        .ghost()
        .xsmall()
        .icon(IconName::RefreshCw)
        .tooltip("刷新目录")
        .on_click(cx.listener(move |this, _, _, cx| {
            this.request_sftp_list_for_connection(tab_id, connection_id, cur_path.clone(), cx);
        }));

    let enter_path_input = path_input.clone();
    let enter_btn = Button::new(format!("side-enter-{side_index}"))
        .ghost()
        .xsmall()
        .icon(IconName::ArrowRight)
        .tooltip("进入路径")
        .on_click(cx.listener(move |this, _, _, cx| {
            let p = enter_path_input.read(cx).value().trim().to_string();
            if !p.is_empty() {
                this.request_sftp_list_for_connection(tab_id, connection_id, p, cx);
            }
        }));

    let home_btn = Button::new(format!("side-home-{side_index}"))
        .ghost()
        .xsmall()
        .icon(IconName::House)
        .tooltip(rust_i18n::t!("sftp.home").to_string())
        .on_click(cx.listener(move |this, _, _, cx| {
            this.request_sftp_list_for_connection(tab_id, connection_id, ".".to_string(), cx);
        }));

    let download_btn = Button::new(format!("side-download-{side_index}"))
        .primary()
        .xsmall()
        .icon(IconName::Download)
        .tooltip(rust_i18n::t!("sftp.download").to_string())
        .on_click(cx.listener(move |this, _, _, cx| {
            let (selected_paths, local_path) = this
                .workspace
                .tab(tab_id)
                .and_then(|t| t.sftp())
                .map(|p| {
                    let paths = p
                        .side_remotes
                        .get(side_index)
                        .map(|s| s.selection.iter().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    (paths, p.local.path.clone())
                })
                .unwrap_or_default();

            for path in selected_paths {
                let fname = path.rsplit('/').next().unwrap_or(&path);
                let local_target = match transom_core::safe_local_join(&local_path, fname) {
                    Ok(path) => path,
                    Err(_) => {
                        if let Some(panel) = this.sftp_panel_mut_tab(tab_id) {
                            panel.set_error(rust_i18n::t!("sftp.invalid_name").to_string());
                        }
                        cx.notify();
                        continue;
                    }
                };
                this.sftp_enqueue_download_path_with_conn(
                    tab_id,
                    Some(connection_id),
                    path,
                    local_target,
                    cx,
                );
            }
        }));

    v_flex()
        .border_b_1()
        .border_color(p.sidebar_border)
        .p_2()
        .gap_1()
        .child(
            h_flex()
                .gap_1()
                .items_center()
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
                        .child(title),
                )
                .child(div().flex_1().child(Input::new(path_input).w_full()))
                .child(up_btn)
                .child(refresh_btn)
                .child(enter_btn)
                .child(collapse_btn)
                .child(close_btn),
        )
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(home_btn)
                .child(div().w(px(1.0)).h(px(14.0)).mx_1().bg(p.sidebar_border))
                .child(download_btn),
        )
        .into_any_element()
}
