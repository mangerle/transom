//! AppShell 顶层布局渲染。

use crate::app::AppShell;
use crate::keymap;
use crate::palette::Palette;
use crate::sftp::EditorDragTracker;
use crate::theme::tokens::{rem_px, size};
use gpui_kit::base::h_flex;
use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::*;

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        // 布局算术与 resizable 面板只接受 px：结构件常量以 rem 表达，这里统一
        // 按当前密度基准换算，密度切换时各面板宽度随之等比变化。
        let rem = window.rem_size();
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
        // ResizableState 按面板索引记录尺寸：折叠时若移除首个面板，
        // 主终端会继承侧边栏的固定尺寸，再次展开时布局被撑满。
        // 因此面板始终参与分组，仅切换可见性以保持索引稳定。
        let mut sidebar_panel = resizable_panel()
            .size(rem_px(size::SIDEBAR_WIDTH, rem))
            .fixed()
            .visible(self.sidebar.visible);
        if self.sidebar.visible {
            sidebar_panel = sidebar_panel.child(self.render_sidebar(window, cx, &p));
        }
        // 状态存在不等于面板展开：拖拽上传留下的隐藏状态不该把面板列撑出来。
        let show_sftp = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tabs.iter().find(|tab| tab.id == tab_id))
            .is_some_and(|tab| tab.sftp().is_some_and(|panel| panel.open));
        let sftp_panel = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tabs.iter().find(|tab| tab.id == tab_id))
            .and_then(|tab| tab.sftp());
        let sftp_local_only = sftp_panel.is_some_and(|panel| panel.local.only);
        let sftp_all_collapsed = sftp_panel.is_some_and(|panel| panel.is_all_collapsed());
        let collapsed_count = sftp_panel
            .map(|panel| panel.collapsed_columns_count())
            .unwrap_or(0);
        let sftp_collapsed_width = (collapsed_count as f32 * 32.0).max(32.0);

        let show_forwards = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tabs.iter().find(|tab| tab.id == tab_id))
            .is_some_and(|tab| tab.forwards_panel().is_some());
        let (sftp_width, sftp_min_width) = if sftp_all_collapsed {
            (sftp_collapsed_width, sftp_collapsed_width)
        } else if sftp_local_only && sftp_panel.is_some_and(|p| p.side_remotes.is_empty()) {
            (
                rem_px(size::SFTP_LOCAL_PANEL_WIDTH, rem).as_f32(),
                rem_px(size::SFTP_LOCAL_MIN_WIDTH, rem).as_f32(),
            )
        } else {
            (
                rem_px(size::SFTP_PANEL_WIDTH, rem).as_f32(),
                rem_px(size::SFTP_MIN_WIDTH, rem).as_f32(),
            )
        };
        // 固定面板不能把右侧关闭按钮挤出窗口：按当前窗口宽度给 SFTP 一个上限。
        // 预留活动栏、侧栏、前向面板与主面板最小宽度，剩余空间才归 SFTP。
        let sidebar_width = if self.sidebar.visible {
            self.workspace_panels
                .read(cx)
                .sizes()
                .first()
                .copied()
                .filter(|width| *width >= px(100.0))
                .unwrap_or(rem_px(size::SIDEBAR_WIDTH, rem))
                .as_f32()
        } else {
            0.0
        };
        let forward_width = if show_forwards {
            rem_px(size::FORWARD_PANEL_WIDTH, rem).as_f32()
        } else {
            0.0
        };
        let available_width = window.viewport_size().width.as_f32()
            - rem_px(size::ACTIVITY_BAR_WIDTH, rem).as_f32()
            - sidebar_width
            - forward_width
            - 100.0
            - 8.0;
        let sftp_max_width = if sftp_all_collapsed {
            sftp_collapsed_width
        } else {
            available_width.max(sftp_min_width)
        };
        // 两个方向各自持有列宽状态：面板顺序随停靠方向变化，共用一份按索引
        // 记录的尺寸会把宽度错位到别的面板上。
        let (side_left, panels_state) = if self.settings.window.sftp_panel_side.is_left() {
            (true, &self.workspace_panels_left)
        } else {
            (false, &self.workspace_panels)
        };
        let mut resizable_group = h_resizable(resizable_id).with_state(panels_state);
        if !self.immersive {
            resizable_group = resizable_group.child(sidebar_panel);
        }
        if !self.immersive && side_left {
            // 停靠左：SFTP 与转发整体挪到主面板之前，贴着侧栏。
            if show_sftp {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(px(sftp_width.min(sftp_max_width)))
                        .size_range(px(sftp_min_width)..px(sftp_max_width))
                        .fixed()
                        .child(self.render_sftp_panel(window, cx, &p)),
                );
            }
            if show_forwards {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(rem_px(size::FORWARD_PANEL_WIDTH, rem))
                        .fixed()
                        .child(self.render_forward_panel(window, cx, &p)),
                );
            }
        }
        resizable_group =
            resizable_group.child(resizable_panel().child(self.render_main(window, cx, &p)));
        if !self.immersive && !side_left {
            if show_sftp {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(px(sftp_width.min(sftp_max_width)))
                        .size_range(px(sftp_min_width)..px(sftp_max_width))
                        .fixed()
                        .child(self.render_sftp_panel(window, cx, &p)),
                );
            }
            if show_forwards {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(rem_px(size::FORWARD_PANEL_WIDTH, rem))
                        .fixed()
                        .child(self.render_forward_panel(window, cx, &p)),
                );
            }
        }

        if !self.immersive && self.monitor.open {
            // 监控抽屉固定在主面板右侧，关闭时完全不占宽度。
            resizable_group = resizable_group.child(
                resizable_panel()
                    .size(px(380.0))
                    .size_range(px(320.0)..px(720.0))
                    .fixed()
                    .child(self.render_monitor_panel(window, cx, &p)),
            );
        }

        let mut main_row = h_flex().flex_1().overflow_hidden();
        if !self.immersive {
            // 与顶栏、状态栏、活动栏的边框一起围合主内容区，补齐右侧轮廓；
            // 停靠左侧时工具面板贴侧栏，主面板成为最右段，轮廓补在左缘。
            if self.settings.window.sftp_panel_side.is_left() {
                main_row = main_row.border_l_1().border_color(p.border);
            } else {
                main_row = main_row.border_r_1().border_color(p.border);
            }
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
            let divider_x = if self.sidebar.visible {
                // 侧栏折叠期间仍以空面板参与分组，拖动其他分隔线可能把它的记录
                // 宽度同步成 0；小于最小宽度的值一律视为无效，回退到默认宽度。
                let sidebar_width = self
                    .workspace_panels
                    .read(cx)
                    .sizes()
                    .first()
                    .copied()
                    .filter(|width| *width >= px(100.0))
                    .unwrap_or(rem_px(size::SIDEBAR_WIDTH, rem))
                    .as_f32();
                rem_px(size::ACTIVITY_BAR_WIDTH, rem) + px(sidebar_width)
            } else {
                px(0.0)
            };
            self.render_status_bar(&p, cx, divider_x)
        });
        // 错误在写入时入队（见 AppShell::set_error），这里统一转成应用内通知。
        // 延迟到绘制结束后推送，避免在渲染期间修改通知层。
        let pending_errors = self.pending_notices.drain();
        if !pending_errors.is_empty() {
            cx.defer_in(window, move |_, window, cx| {
                for message in pending_errors {
                    window.push_notification(Notification::error(message).autohide(false), cx);
                }
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
            .on_action(cx.listener(Self::on_toggle_block_view))
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
            // Esc 的传输取消监听：键盘事件沿焦点路径回溯，终端（焦点方）
            // 先收到并向远端写一次转义序列，再由根节点截获成取消命令。
            .on_key_down(cx.listener(Self::on_zmodem_cancel_key))
            // 拖拽悬浮面板的位移与收尾由 EditorDragTracker 的窗口级监听接管：
            // 普通元素监听带"指针命中"门槛，拖拽中指针会移出面板乃至窗口，
            // 元素级监听收不到这些事件。追踪器以无视觉元素的形式挂在本容器
            // 末尾，paint 阶段注册的窗口级监听对所有鼠标事件可见。
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
        // 追踪器必须最后挂载：bubble 阶段按注册序的逆序派发，最后注册的
        // 监听最先执行，拖拽位移与收尾不会被其他监听的 stop_propagation 挡掉。
        root.child(EditorDragTracker::new(cx.entity()))
            .children(overlays)
    }
}
