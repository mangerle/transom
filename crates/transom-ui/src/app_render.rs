//! AppShell 顶层布局渲染。

use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled, h_resizable};
use crate::keymap;
use crate::palette::Palette;
use crate::sftp::{EditorDragTracker, SftpPanelState};
use crate::theme::tokens::{rem_px, size};
use gpui_kit::base::h_flex;
use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::resizable_panel;
use gpui_kit::*;

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        // 弹窗栈对账必须每帧执行：状态变更点分散在菜单、快捷键、拖放与异步
        // 回调等二十多处，逐点补容器开关必然漏；渲染是唯一汇聚点。真正的
        // 开关动作被延到绘制之后执行，避免在渲染过程中移交焦点。
        self.reconcile_dialogs(window, cx);
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
        let sidebar_min_width = rem_px(size::SIDEBAR_MIN_WIDTH, rem);
        let sidebar_max_width = rem_px(size::SIDEBAR_MAX_WIDTH, rem);
        let mut sidebar_panel = resizable_panel()
            .size(rem_px(size::SIDEBAR_WIDTH, rem))
            .size_range(sidebar_min_width..sidebar_max_width)
            .fixed()
            .visible(self.sidebar.visible);
        if self.sidebar.visible {
            // 左侧槽位一次只承载一种内容：纵向标签面板或会话树。
            let content = if self.show_vertical_tabs_panel() {
                self.render_vertical_tabs_panel(&p, cx)
            } else {
                self.render_sidebar(window, cx, &p)
            };
            sidebar_panel = sidebar_panel.child(content);
        }
        let plan = self.plan_workspace_layout(window, cx);
        // 折叠状态变化时把 SFTP 面板同步到内容宽度：必须赶在分组构建之前，
        // 本帧的面板尺寸才能反映新的宽度。
        self.sync_sftp_panel_width(window, cx);
        let panels_state = if plan.side_left {
            self.workspace_panels_left.clone()
        } else {
            self.workspace_panels.clone()
        };

        let side_left = plan.side_left;
        let show_sftp = plan.show_sftp;
        let show_forwards = plan.show_forwards;
        let mut resizable_group = if self.immersive {
            // 沉浸模式只保留主面板，使用独立分组与临时状态；
            // 严禁复用 panels_state，否则主面板在全屏下会覆盖索引 0（侧栏）的尺寸记录，
            // 导致退出沉浸模式后侧边栏继承全屏宽度并撑满窗口。
            h_resizable(resizable_id)
        } else {
            h_resizable(resizable_id).with_state(&panels_state)
        };
        if !self.immersive {
            resizable_group = resizable_group.child(sidebar_panel);
        }
        if !self.immersive && side_left {
            // 停靠左：SFTP 与转发整体挪到主面板之前，贴着侧栏。
            if show_sftp {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(px(plan.sftp_width.min(plan.sftp_max_width)))
                        .size_range(px(plan.sftp_min_width)..px(plan.sftp_max_width))
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
                        .size(px(plan.sftp_width.min(plan.sftp_max_width)))
                        .size_range(px(plan.sftp_min_width)..px(plan.sftp_max_width))
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
            // 监控抽屉停靠在主面板右侧，支持拖拽调整宽度（360px 至屏幕宽度的 75%）。
            let max_drawer_width = (window.viewport_size().width.as_f32() * 0.75).max(520.0);
            if let Some(actual_w) = panels_state.read(cx).sizes().last().copied() {
                let actual_f32 = actual_w.as_f32();
                if actual_f32 >= 360.0
                    && actual_f32 <= max_drawer_width
                    && (self.monitor.drawer_width - actual_f32).abs() > 1.0
                {
                    self.monitor.drawer_width = actual_f32;
                    self.settings.monitor.drawer_width = actual_f32;
                }
            }
            let monitor_width = self.monitor.drawer_width.clamp(360.0, max_drawer_width);
            resizable_group = resizable_group.child(
                resizable_panel()
                    .size(px(monitor_width))
                    .size_range(px(360.0)..px(max_drawer_width))
                    .child(self.render_monitor_panel(window, cx, &p)),
            );
        }

        let mut main_row = h_flex().flex_1().overflow_hidden();
        if !self.immersive {
            // 主内容区只在朝窗口边缘的一侧补轮廓，且只在右侧真的空到窗口边缘时
            // 才补：面板之间由可拖拽手柄画唯一的分割线，这里再叠一条轮廓/面板
            // 边框就会并排成 2px。停靠左侧时主面板右侧是监控抽屉或窗口边缘，
            // 左边一律交给手柄，不再补左轮廓。
            let has_right_panel = show_sftp || show_forwards || self.monitor.open;
            if !self.settings.window.sftp_panel_side.is_left() && !has_right_panel {
                main_row = main_row
                    .border_color(p.divider(p.border))
                    .divider_edge(DividerEdge::Right, p.divider_width);
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
                // 宽度同步成 0；小于最小宽度或大于最大宽度的值一律视为无效，回退到默认宽度。
                let sidebar_width = panels_state
                    .read(cx)
                    .sizes()
                    .first()
                    .copied()
                    .filter(|width| *width >= sidebar_min_width && *width <= sidebar_max_width)
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
            .on_action(cx.listener(Self::on_toggle_copy_mode))
            .on_action(cx.listener(Self::on_toggle_block_view))
            .on_action(cx.listener(Self::on_prev_prompt))
            .on_action(cx.listener(Self::on_next_prompt))
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
        // 弹窗栈的压暗层铺在编辑面板之上：弹窗本体由组件库在窗口根上渲染，
        // 层序天然更高，这里只需要把窗面压暗并阻断下层命中。
        root = root.children(self.dialog_dim(&p));
        // 追踪器必须最后挂载：bubble 阶段按注册序的逆序派发，最后注册的
        // 监听最先执行，拖拽位移与收尾不会被其他监听的 stop_propagation 挡掉。
        root.child(EditorDragTracker::new(cx.entity()))
            .children(overlays)
    }
}

/// 外层布局的派生尺寸与可见性：把窗口宽度、侧栏实测宽度与面板折叠状态
/// 折算成 resizable 面板需要的像素值。
struct WorkspaceChromeLayout {
    show_sftp: bool,
    show_forwards: bool,
    sftp_width: f32,
    sftp_min_width: f32,
    sftp_max_width: f32,
    side_left: bool,
}

impl AppShell {
    /// 计算外层布局。设计原理（Why）：这些算术只在渲染时需要，抽出来后
    /// `render` 只负责按 plan 拼装面板，宽度规则可以单独核对与调整。
    fn plan_workspace_layout(&self, window: &Window, cx: &App) -> WorkspaceChromeLayout {
        let rem = window.rem_size();
        let active_panel = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tab(tab_id));
        let show_sftp = active_panel.is_some_and(|tab| tab.sftp().is_some_and(|panel| panel.open));
        let sftp_panel = active_panel.and_then(|tab| tab.sftp());
        let sftp_local_only = sftp_panel.is_some_and(|panel| panel.local.only);
        let sftp_all_collapsed = sftp_panel.is_some_and(|panel| panel.is_all_collapsed());
        let collapsed_count = sftp_panel
            .map(|panel| panel.collapsed_columns_count())
            .unwrap_or(0);
        // 全收起时的目标宽度必须包含面板自身边框，否则折叠状态下每次按内容
        // 宽度回写外层槽位都会少 1px，面板会逐帧向右收缩。
        let sftp_collapsed_width = sftp_panel
            .map(SftpPanelState::all_collapsed_width)
            .unwrap_or(32.0);
        let show_forwards = active_panel.is_some_and(|tab| tab.forwards_panel().is_some());
        let (sftp_width, mut sftp_min_width) = if sftp_all_collapsed {
            (sftp_collapsed_width, sftp_collapsed_width)
        } else if sftp_local_only && sftp_panel.is_some_and(|panel| panel.side_remotes.is_empty()) {
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
        // 有列折叠时允许 SFTP 面板收缩到实际内容宽度：固定的 SFTP_MIN_WIDTH
        // 会把面板卡在更宽的位置，折叠条右侧就会留出一大片空白。
        if collapsed_count > 0
            && !sftp_all_collapsed
            && let Some(content_width) =
                sftp_panel.and_then(|panel| panel.collapsed_content_width())
        {
            sftp_min_width = sftp_min_width.min(content_width);
        }
        // 固定面板不能把右侧关闭按钮挤出窗口：按当前窗口宽度给 SFTP 一个上限。
        // 预留活动栏、侧栏、前向面板与主面板最小宽度，剩余空间才归 SFTP。
        let sidebar_min_width = rem_px(size::SIDEBAR_MIN_WIDTH, rem);
        let sidebar_max_width = rem_px(size::SIDEBAR_MAX_WIDTH, rem);
        let sidebar_width = if self.sidebar.visible {
            let panels_state = if self.settings.window.sftp_panel_side.is_left() {
                &self.workspace_panels_left
            } else {
                &self.workspace_panels
            };
            panels_state
                .read(cx)
                .sizes()
                .first()
                .copied()
                .filter(|width| *width >= sidebar_min_width && *width <= sidebar_max_width)
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
        WorkspaceChromeLayout {
            show_sftp,
            show_forwards,
            sftp_width,
            sftp_min_width,
            sftp_max_width,
            side_left: self.settings.window.sftp_panel_side.is_left(),
        }
    }
}
