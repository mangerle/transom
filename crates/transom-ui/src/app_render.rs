//! AppShell 顶层布局渲染。

use crate::activity_bar::ACTIVITY_BAR_WIDTH;
use crate::app::AppShell;
use crate::keymap;
use crate::palette::Palette;
use gpui_kit::base::h_flex;
use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::*;

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let top_bar = (!self.immersive).then(|| self.render_top_bar(window, cx, &p));
        let activity_bar = (!self.immersive).then(|| self.render_activity_bar(window, cx, &p));

        // 沉浸模式使用独立分组：侧边栏完全不参与布局，主面板成为首面板，
        // 避免其左侧残留一条用于拖动侧边栏的 1px 分隔线；
        // 独立 key 也保证退出沉浸后原分组的全部尺寸状态原样恢复。
        let resizable_id = if self.immersive {
            "workspace-panels-immersive"
        } else {
            "workspace-panels"
        };
        let mut resizable_group = h_resizable(resizable_id);
        if !self.immersive {
            // 非沉浸分组由外壳持有列宽状态，状态栏才能读到与侧栏一致的分界位置。
            resizable_group = resizable_group.with_state(&self.workspace_panels);
            // ResizableState 按面板索引记录尺寸：折叠时若移除首个面板，
            // 主终端会继承侧边栏的固定尺寸，再次展开时布局被撑满。
            // 因此面板始终参与分组，仅切换可见性以保持索引稳定。
            let mut sidebar_panel = resizable_panel()
                .size(px(270.0))
                .fixed()
                .visible(self.sidebar_visible);
            if self.sidebar_visible {
                sidebar_panel = sidebar_panel.child(self.render_sidebar(window, cx, &p));
            }
            resizable_group = resizable_group.child(sidebar_panel);
        }
        resizable_group =
            resizable_group.child(resizable_panel().child(self.render_main(window, cx, &p)));
        let show_sftp = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .is_some_and(|tab| tab.sftp.is_some());
        let sftp_local_only = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .and_then(|tab| tab.sftp.as_ref())
            .is_some_and(|panel| panel.local_only);
        let show_forwards = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .is_some_and(|tab| tab.forwards_panel.is_some());
        let sftp_width: f32 = if sftp_local_only { 480.0 } else { 760.0 };
        let sftp_min_width: f32 = if sftp_local_only { 320.0 } else { 360.0 };
        // 固定面板不能把右侧关闭按钮挤出窗口：按当前窗口宽度给 SFTP 一个上限。
        // 预留活动栏、侧栏、前向面板与主面板最小宽度，剩余空间才归 SFTP。
        let sidebar_width = if self.sidebar_visible {
            self.workspace_panels
                .read(cx)
                .sizes()
                .first()
                .copied()
                .filter(|width| *width >= px(100.0))
                .unwrap_or(px(270.0))
                .as_f32()
        } else {
            0.0
        };
        let forward_width = if show_forwards { 340.0 } else { 0.0 };
        let available_width = window.viewport_size().width.as_f32()
            - ACTIVITY_BAR_WIDTH
            - sidebar_width
            - forward_width
            - 100.0
            - 8.0;
        let sftp_max_width = available_width.max(sftp_min_width);
        if show_sftp && !self.immersive {
            resizable_group = resizable_group.child(
                resizable_panel()
                    .size(px(sftp_width.min(sftp_max_width)))
                    .size_range(px(sftp_min_width)..px(sftp_max_width))
                    .fixed()
                    .child(self.render_sftp_panel(window, cx, &p)),
            );
        }
        if show_forwards && !self.immersive {
            resizable_group = resizable_group.child(
                resizable_panel()
                    .size(px(340.0))
                    .fixed()
                    .child(self.render_forward_panel(window, cx, &p)),
            );
        }

        let mut main_row = h_flex().flex_1().overflow_hidden();
        if !self.immersive {
            // 与顶栏、状态栏、活动栏的边框一起围合主内容区，补齐右侧轮廓。
            main_row = main_row.border_r_1().border_color(p.border);
        }
        if let Some(activity_bar) = activity_bar {
            main_row = main_row.child(activity_bar);
        }
        main_row = main_row.child(
            div()
                .flex_1()
                .h_full()
                .overflow_hidden()
                .child(resizable_group),
        );
        // 状态栏按活动栏加侧栏的实测宽度切分。尺寸取自工作区分组的上一帧布局，
        // 拖动侧栏时由 ResizableState 的通知驱动整帧重绘，分界线落后一帧。
        let status_bar = (!self.immersive).then(|| {
            let divider_x = if self.sidebar_visible {
                // 侧栏折叠期间仍以空面板参与分组，拖动其他分隔线可能把它的记录
                // 宽度同步成 0；小于最小宽度的值一律视为无效，回退到默认宽度。
                let sidebar_width = self
                    .workspace_panels
                    .read(cx)
                    .sizes()
                    .first()
                    .copied()
                    .filter(|width| *width >= px(100.0))
                    .unwrap_or(px(270.0));
                px(ACTIVITY_BAR_WIDTH) + sidebar_width
            } else {
                px(0.0)
            };
            self.render_status_bar(&p, cx, divider_x)
        });
        // 错误横幅已移除：统一转成应用内通知。设置与触发点仍写 self.error，
        // 这里取走并延迟到绘制结束后推送，避免在渲染期间修改通知层。
        if let Some(message) = self.error.take() {
            cx.defer_in(window, move |_, window, cx| {
                window.push_notification(Notification::error(message).autohide(false), cx);
            });
        }
        // 编辑面板先于模态弹窗入层：模态遮罩应当盖住悬浮面板，反之会出现
        // 弹窗在面板下面、用户点不到确认按钮的层序错误。
        let editor_layers = self.render_editor_panels(window, &p, cx);
        let overlays = self.render_overlays(window, cx);

        let mut root = div()
            .key_context(keymap::WORKSPACE_CONTEXT)
            .on_action(cx.listener(Self::on_new_session))
            .on_action(cx.listener(Self::on_new_local_terminal))
            .on_action(cx.listener(Self::on_quick_open))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_prev_tab))
            .on_action(cx.listener(Self::on_reconnect))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_toggle_sftp))
            .on_action(cx.listener(Self::on_toggle_forward))
            .on_action(cx.listener(Self::on_toggle_immersive))
            .on_action(cx.listener(Self::on_search_buffer))
            .on_action(cx.listener(Self::on_toggle_snippets))
            .on_action(cx.listener(Self::on_split_right))
            .on_action(cx.listener(Self::on_split_down))
            .on_action(cx.listener(Self::on_close_pane))
            .on_action(cx.listener(Self::on_toggle_split_axis))
            .on_action(cx.listener(Self::on_zoom_pane))
            .on_action(cx.listener(Self::on_focus_pane_left))
            .on_action(cx.listener(Self::on_focus_pane_right))
            .on_action(cx.listener(Self::on_focus_pane_up))
            .on_action(cx.listener(Self::on_focus_pane_down))
            // 拖拽悬浮面板时指针会移出面板，位移与收尾由根元素的监听接管。
            // mouse_up 可能丢失：元素级监听只投递给命中元素，指针移出窗口
            // 松开、或抬起落在根元素树之外的浮层（弹出菜单、补全菜单）上时，
            // 收不到抬起事件，拖拽位残留会让面板此后一直跟随鼠标。移动事件
            // 自带按键状态，未按键时先收尾再返回，从根上杜绝卡拖拽。
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button.is_none() {
                    this.end_editor_panel_drag(cx);
                    return;
                }
                this.drag_editor_panels(event.position, window.viewport_size(), cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.end_editor_panel_drag(cx);
                }),
            )
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.background)
            .text_color(p.foreground);
        if let Some(top_bar) = top_bar {
            root = root.child(top_bar);
        }
        root = root.child(main_row);
        if let Some(status_bar) = status_bar {
            root = root.child(status_bar);
        }
        root = root.children(editor_layers);
        root.children(overlays)
    }
}
