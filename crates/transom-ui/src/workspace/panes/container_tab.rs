//! 容器标签与空标签的窗口级渲染入口。

use super::*;

impl AppShell {
    /// 容器标签的窗口级入口：连接态分流、列表焦点/键盘包装与正文装配。
    ///
    /// 设计原理（Why）：焦点句柄、按键监听与通知转弹必须拿到 `Window`/外壳，
    /// 无法下放给纯渲染函数；这里只保留这层窗口级胶水，头部、列表、详情与抽屉
    /// 的正文全部由 `container::view` 的自由函数渲染，不再反查外壳。
    pub(super) fn render_container_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(view) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let list_panel = {
            let state = view.read(cx);
            if !matches!(
                state.state.status,
                crate::container::ContainerWorkspaceStatus::Connected
            ) {
                return crate::container::render_container_connection_state(&view, &state.state, p);
            }
            let list = crate::container::render_container_list_content(
                cx.entity(),
                &view,
                &state.state,
                cx,
                window.rem_size(),
                p,
            );
            // 焦点与按键仍归外壳：列表渲染函数只画内容，键盘事件到来时再按当前
            // 状态重算行集合，避免把窗口级焦点管理塞进纯渲染函数。
            let list_focus = state.state.list_focus_handle.clone();
            v_flex()
                .id("container-list-panel")
                .track_focus(&list_focus)
                .key_context("ContainerList")
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    crate::container::handle_container_list_key(this, tab_id, event, window, cx);
                }))
                .h_full()
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .child(list)
                .into_any_element()
        };

        self.flush_container_notice(tab_id, window, cx);
        let fullscreen = self.state.containers.fullscreen == Some(tab_id);
        let state = view.read(cx);
        crate::container::render_container_tab_body(
            &view,
            tab_id,
            &state.state,
            cx,
            fullscreen,
            list_panel,
            p,
        )
    }

    /// 把容器工作区暂存的操作提示转成右上角通知。
    ///
    /// 提示在动作发生时写入实体状态，渲染帧取走后再推送通知；这样不需要让每个
    /// 业务动作都持有窗口句柄，也不会在连续重绘中重复弹出同一条消息。
    fn flush_container_notice(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        let has_notice = workspace.read(cx).state.notice.is_some();
        if !has_notice {
            return;
        }
        // 渲染帧只读判定，真正的状态变更排到本轮效果之后：渲染路径不更新实体，
        // 避免渲染期写入容器实体造成二次租借或重入。
        cx.defer_in(window, move |this, window, cx| {
            let notice = this
                .state
                .container_workspace(tab_id)
                .and_then(|workspace| {
                    workspace.update(cx, |workspace, _| workspace.state.notice.take())
                });
            if let Some(message) = notice {
                this.show_info_toast(message, window, cx);
            }
        });
    }

    /// 全屏模态：复用同一份工作区状态，标签内显示占位避免重复渲染终端。
    ///
    /// 全屏覆盖层依赖 `Window` 级按键与遮罩，保留为最小外壳入口；正文仍走
    /// [`Self::render_container_tab`]。
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
                    crate::container::on_container_overlay_key(this, event, Some(tab_id), cx);
                }),
            )
            .child(content);
        crate::palette::overlay(dismiss, p)
    }
}
