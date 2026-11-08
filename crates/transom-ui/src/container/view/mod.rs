//! 容器工作区主视图：连接态、页签骨架与全屏入口。
//!
//! 头部与操作按钮在 header.rs。

mod header;

use crate::app::AppShell;
use crate::container::detail::{ContainerDetailParams, render_container_detail};
use crate::container::list::render_container_list;
use crate::container::types::ContainerWorkspaceStatus;
use crate::divider::h_resizable;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::{ResizableState, resizable_panel};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use header::render_container_header;
use transom_core::TabId;

impl AppShell {
    /// 管理工作区标签的完整视图。
    pub(crate) fn render_container_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(view) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        if !matches!(
            view.read(cx).state.status,
            ContainerWorkspaceStatus::Connected
        ) {
            return self.render_container_connection_state(tab_id, window, cx, p);
        }
        self.flush_container_notice(tab_id, window, cx);
        let fullscreen = self.state.containers.fullscreen == Some(tab_id);
        let header = {
            let workspace = view.read(cx);
            render_container_header(&view, &workspace.state, fullscreen, p)
        };
        let list = {
            let workspace = view.read(cx);
            let query = workspace.state.query(cx);
            render_container_list(&view, &workspace.state, &query, p)
        };
        // 焦点与按键仍归外壳：列表渲染函数只画内容，键盘事件到来时再按当前
        // 状态重算行集合，避免把窗口级焦点管理塞进纯渲染函数。
        let list_focus = view.read(cx).state.list_focus_handle.clone();
        let list = v_flex()
            .id("container-list-panel")
            .track_focus(&list_focus)
            .key_context("ContainerList")
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                this.handle_container_list_key(tab_id, event, window, cx);
            }))
            .h_full()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .child(list)
            .into_any_element();
        let detail = {
            let workspace = view.read(cx);
            let state = &workspace.state;
            // 输入框与编辑器文本必须经 App 读取；详情渲染函数只消费快照。
            let compose_yaml_text = state
                .compose_yaml_editor
                .as_ref()
                .map(|(_, _, editor, _)| editor.read(cx).value())
                .unwrap_or_default();
            let logs_filter = state.logs_search_query(cx);
            let changes_search_value = state.changes_search.read(cx).value().to_string();
            let inspect_keyword = state.inspect_search.read(cx).value().trim().to_string();
            render_container_detail(
                ContainerDetailParams {
                    tab_id,
                    view: &view,
                    workspace: state,
                    compose_yaml_text: &compose_yaml_text,
                    logs_filter: &logs_filter,
                    changes_search_value: &changes_search_value,
                    inspect_keyword: &inspect_keyword,
                },
                p,
            )
        };
        // 列表与详情宽度可拖拽：长容器名与镜像引用在固定宽度下必然截断，详情区在
        // 小窗下又铺不开，与主窗口其他分区一样交给用户拖。默认值只是首次打开的起点。
        let list_split = self
            .state
            .container_workspace(tab_id)
            .map(|workspace| workspace.read(cx).state.list_split.clone())
            .unwrap_or_else(|| cx.new(|_| ResizableState::default()));

        let list_and_detail = h_resizable("container-split-list")
            .with_state(&list_split)
            .child(
                resizable_panel()
                    .size(px(380.0))
                    .size_range(px(260.0)..px(680.0))
                    .fixed()
                    // 面板本身不裁剪内容：工具条与行一旦超出宽度就整体压到右栏上
                    // （真实窗口实测），这里补一层裁剪容器把中栏锁在自己的宽度内。
                    .child(div().size_full().min_w_0().overflow_hidden().child(list)),
            )
            .child(resizable_panel().min_w_0().child(detail));

        let pull_drawer = match self.state.container_workspace(tab_id) {
            Some(view) => {
                let state = view.read(cx);
                crate::container::render_pull_drawer(&view, &state.state, p)
            }
            None => div().into_any_element(),
        };

        v_flex()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(p.background)
            .child(header)
            .child(div().flex_1().min_w_0().min_h_0().child(list_and_detail))
            .child(pull_drawer)
            .into_any_element()
    }

    /// 把容器工作区暂存的操作提示转成右上角通知。
    ///
    /// 提示在动作发生时写入状态，渲染帧取走后再推送通知；这样不需要让每个
    /// 业务动作都持有窗口句柄，也不会在连续重绘中重复弹出同一条消息。
    fn flush_container_notice(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 渲染帧只读判定，真正的状态变更排到本轮效果之后：渲染路径不更新实体，
        // 避免渲染期写入容器实体造成二次租借或重入。
        let has_notice = self
            .with_container(tab_id, cx, |workspace| workspace.notice.is_some())
            .unwrap_or(false);
        if !has_notice {
            return;
        }
        cx.defer_in(window, move |this, window, cx| {
            let notice = this
                .with_container_mut(tab_id, cx, |workspace| workspace.notice.take())
                .flatten();
            if let Some(message) = notice {
                this.show_info_toast(message, window, cx);
            }
        });
    }

    fn render_container_connection_state(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let workspace = &workspace.read(cx).state;
        let status = workspace.status.clone();
        let profile = workspace.profile.clone();
        let (icon, title, detail, retry) = match status {
            ContainerWorkspaceStatus::Connecting => (
                IconName::Loader,
                rust_i18n::t!("container.state.connecting").to_string(),
                profile.engine.describe(),
                false,
            ),
            ContainerWorkspaceStatus::Failed(error) => (
                IconName::CircleAlert,
                rust_i18n::t!("container.state.failed").to_string(),
                error,
                true,
            ),
            ContainerWorkspaceStatus::Disconnected => (
                IconName::CircleAlert,
                rust_i18n::t!("container.state.disconnected").to_string(),
                profile.engine.describe(),
                true,
            ),
            ContainerWorkspaceStatus::Connected => {
                (IconName::Container, String::new(), String::new(), false)
            }
        };
        let mut body = v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                Icon::new(icon)
                    .with_size(px(28.0))
                    .text_color(p.muted_foreground),
            )
            .child(div().text_sm().child(title))
            .child(div().text_xs().text_color(p.muted_foreground).child(detail));
        if retry {
            body = body.child(
                Button::new("container-retry")
                    .primary()
                    .small()
                    .icon(IconName::RefreshCw)
                    .label(rust_i18n::t!("common.retry").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(workspace) = this.state.container_workspace(tab_id) {
                            workspace
                                .read(cx)
                                .send(transom_core::ContainerCommand::Connect {
                                    tab_id,
                                    profile: profile.clone(),
                                });
                        }
                        this.with_container_mut(tab_id, cx, |workspace| {
                            workspace.status = ContainerWorkspaceStatus::Connecting;
                        });
                        cx.notify();
                    })),
            );
        }
        body.into_any_element()
    }

    /// 全屏模态：复用同一份工作区状态，标签内显示占位避免重复渲染终端。
    pub(crate) fn render_container_fullscreen(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        if self.state.container_workspace(tab_id).is_none() {
            return div().into_any_element();
        }
        let content = self.render_container_tab(tab_id, window, cx, p);
        let dismiss = v_flex()
            .id("container-fullscreen-shell")
            .size_full()
            .bg(p.background)
            .key_context(crate::keymap::OVERLAY_CONTEXT)
            .on_key_down(
                cx.listener(move |this, event: &gpui_kit::KeyDownEvent, _, cx| {
                    this.on_container_overlay_key(event, Some(tab_id), cx);
                }),
            )
            .child(content);
        Self::overlay(dismiss, p)
    }
}
