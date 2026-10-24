//! AppShell 顶层布局渲染。

use crate::app::AppShell;
use crate::keymap;
use crate::palette::Palette;
use gpui_kit::base::h_flex;
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
        if show_sftp && !self.immersive {
            resizable_group = resizable_group.child(
                resizable_panel()
                    .size(px(760.0))
                    .fixed()
                    .child(self.render_sftp_panel(window, cx, &p)),
            );
        }
        let show_forwards = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .is_some_and(|tab| tab.forwards_panel.is_some());
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
        // 快捷命令条贴在状态栏上方：底栏属于工作区全局，不随窗格分屏重复出现。
        let snippets_bar =
            (!self.immersive && self.snippets_expanded).then(|| self.render_snippets_bar(&p, cx));
        let error_banner = self
            .error
            .as_ref()
            .map(|_| self.render_error_banner(&p, cx));
        let status_bar = (!self.immersive).then(|| self.render_status_bar(&p, cx));
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
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
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
        if let Some(snippets_bar) = snippets_bar {
            root = root.child(snippets_bar);
        }
        if let Some(error_banner) = error_banner {
            root = root.child(error_banner);
        }
        if let Some(status_bar) = status_bar {
            root = root.child(status_bar);
        }
        root = root.children(editor_layers);
        root.children(overlays)
    }
}
