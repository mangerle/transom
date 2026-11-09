//! AppShell 顶层布局渲染。

use super::workspace_render::{StatusBarLayoutInput, WorkspaceGroupInput};
use crate::app::AppShell;
use crate::app::app_settings;
use crate::divider::{DividerEdge, DividerStyled};
use crate::keymap;
use crate::palette::Palette;
use crate::theme::tokens::{rem_px, size};
use gpui_kit::base::h_flex;
use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

/// 工作区外层布局的渲染输入。
struct WorkspaceBodyInput<'a> {
    window: &'a mut Window,
    palette: &'a Palette,
    rem: Pixels,
    activity_bar: Option<AnyElement>,
}

/// 工作区外层布局的渲染结果：主内容行与可选状态栏。
struct WorkspaceBody {
    main_row: AnyElement,
    status_bar: Option<AnyElement>,
}

impl AppShell {
    /// 渲染侧栏、可拖拽分组、SFTP/转发面板、监控抽屉与状态栏。
    ///
    /// 设计原理（Why）：整壳 `Render::render` 负责生命周期与事件路由，具体的
    /// 面板拼装细节放到这里，避免单个函数超过两百行后无法定位布局规则；
    /// 返回主行与状态栏两个元素，调用方只决定层序。
    fn render_workspace_body(
        &mut self,
        cx: &mut Context<Self>,
        input: WorkspaceBodyInput<'_>,
    ) -> WorkspaceBody {
        let WorkspaceBodyInput {
            window,
            palette: p,
            rem,
            activity_bar,
        } = input;
        // 沉浸模式使用独立分组：侧边栏完全不参与布局，主面板成为首面板，
        // 避免其左侧残留一条用于拖动侧边栏的 1px 分隔线；
        // 独立 key 也保证退出沉浸后原分组的全部尺寸状态原样恢复。
        let resizable_id = if self.state.immersive {
            "workspace-panels-immersive"
        } else {
            "workspace-panels"
        };
        // ResizableState 按面板索引记录尺寸：折叠时若移除首个面板，
        // 主终端会继承侧边栏的固定尺寸，再次展开时布局被撑满。
        // 因此面板始终参与分组，仅切换可见性以保持索引稳定；侧栏槽位的
        // 宽度由同步函数修正，保证收起后尺寸表与实际占位一致。
        let sidebar_width = self.sync_sidebar_panel_slot(window, cx);
        let sidebar_panel = self.build_sidebar_panel(window, cx, p, rem, sidebar_width);

        let plan = self.plan_workspace_layout(window, cx, sidebar_width);
        // 折叠状态变化时把 SFTP 面板同步到内容宽度：必须赶在分组构建之前，
        // 本帧的面板尺寸才能反映新的宽度。
        self.sync_sftp_panel_width(window, cx);
        let panels_state = if plan.side_left {
            self.state.workspace_panels_left.clone()
        } else {
            self.state.workspace_panels.clone()
        };

        let show_sftp = plan.show_sftp;
        let show_forwards = plan.show_forwards;
        let resizable_group = self.build_resizable_group(
            WorkspaceGroupInput {
                resizable_id,
                plan: &plan,
                sidebar_panel,
                panels_state: &panels_state,
                p,
                rem,
            },
            window,
            cx,
        );

        let mut main_row = h_flex().flex_1().overflow_hidden();
        if !self.state.immersive {
            // 主内容区只在朝窗口边缘的一侧补轮廓，且只在右侧真的空到窗口边缘时
            // 才补：面板之间由可拖拽手柄画唯一的分割线，这里再叠一条轮廓/面板
            // 边框就会并排成 2px。停靠左侧时主面板右侧是监控抽屉或窗口边缘，
            // 左边一律交给手柄，不再补左轮廓。
            let has_right_panel =
                show_sftp || show_forwards || self.state.monitor.read(cx).is_open();
            if !app_settings(cx).window.sftp_panel_side.is_left() && !has_right_panel {
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
        let status_bar = self.render_workspace_status_bar(
            StatusBarLayoutInput {
                p,
                rem,
                sidebar_visible: self.state.sidebar.visible,
                sidebar_width,
            },
            cx,
        );

        WorkspaceBody {
            main_row: main_row.into_any_element(),
            status_bar,
        }
    }
}

impl AppShell {
    /// 把工作区级快捷键动作挂到根元素上。
    ///
    /// 动作链是声明式映射，单独成方法后 `render` 只保留布局与层序；
    /// 新增快捷键时只改这里，不会把整壳渲染再撑长。
    fn attach_workspace_actions(&self, root: Div, cx: &mut Context<Self>) -> Div {
        root.key_context(keymap::WORKSPACE_CONTEXT)
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
        // 元素级监听收不到这些事件。追踪器随编辑面板实体渲染挂载，
        // paint 阶段注册的窗口级监听对所有鼠标事件可见。
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.run_frame_effects(window, cx);
        let p = Palette::from(cx);
        // 弹窗栈对账必须每帧执行：状态变更点分散在菜单、快捷键、拖放与异步
        // 回调等二十多处，逐点补容器开关必然漏；渲染是唯一汇聚点。真正的
        // 开关动作被延到绘制之后执行，避免在渲染过程中移交焦点。
        self.reconcile_dialogs(window, cx);
        // 布局算术与 resizable 面板只接受 px：结构件常量以 rem 表达，这里统一
        // 按当前密度基准换算，密度切换时各面板宽度随之等比变化。
        let rem = window.rem_size();
        let top_bar = (!self.state.immersive).then(|| self.render_top_bar(window, cx, &p));
        let activity_bar =
            (!self.state.immersive).then(|| self.render_activity_bar(window, cx, &p));

        let workspace = self.render_workspace_body(
            cx,
            WorkspaceBodyInput {
                window,
                palette: &p,
                rem,
                activity_bar,
            },
        );

        // 错误在写入时入队（见 AppShell::set_error），这里统一转成应用内通知。
        // 延迟到绘制结束后推送，避免在渲染期间修改通知层。
        let pending_errors = self.state.pending_notices.drain();
        if !pending_errors.is_empty() {
            cx.defer_in(window, move |_, window, cx| {
                for message in pending_errors {
                    window.push_notification(Notification::error(message).autohide(false), cx);
                }
            });
        }
        // 编辑面板先于模态弹窗入层：模态遮罩应当盖住悬浮面板，反之会出现
        // 弹窗在面板下面、用户点不到确认按钮的层序错误。
        let overlays = self.render_overlays(window, cx);

        let mut root = self
            .attach_workspace_actions(div(), cx)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.background)
            .text_color(p.foreground);
        if let Some(top_bar) = top_bar {
            root = root.child(top_bar);
        }
        root = root.child(workspace.main_row);
        if let Some(status_bar) = workspace.status_bar {
            root = root.child(status_bar);
        }
        root = root.child(self.state.editors.clone());
        // 弹窗栈的压暗层铺在编辑面板之上：弹窗本体由组件库在窗口根上渲染，
        // 层序天然更高，这里只需要把窗面压暗并阻断下层命中。
        root = root.children(self.dialog_dim(&p));
        root.children(overlays)
    }
}

impl AppShell {
    /// 工作区状态栏：按活动栏加侧栏实测宽度切分，沉浸模式隐藏。
    fn render_workspace_status_bar(
        &mut self,
        input: StatusBarLayoutInput<'_>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.state.immersive {
            return None;
        }
        let StatusBarLayoutInput {
            p,
            rem,
            sidebar_visible,
            sidebar_width,
        } = input;
        // 分界线取同步函数给出的侧栏实际宽度，不反查分栏尺寸表：
        // 收起/展开的过渡帧里尺寸表还是旧值，直接读会让状态栏分界线跳一下。
        let divider_x = if sidebar_visible {
            rem_px(size::ACTIVITY_BAR_WIDTH, rem) + px(sidebar_width)
        } else {
            px(0.0)
        };
        Some(self.render_status_bar(p, cx, divider_x))
    }
}
