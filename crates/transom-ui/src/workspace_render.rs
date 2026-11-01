//! 工作区布局计划与可拖拽分组构建。
//!
//! 布局算术集中在 `plan_workspace_layout`，分组插入顺序集中在
//! `build_resizable_group`，两者只依赖窗口尺寸、状态与调色板。

use super::*;
use crate::palette::Palette;
use crate::sftp::SftpPanelState;
use crate::theme::tokens::{rem_px, size};
use gpui_kit::base::{ResizablePanel, ResizablePanelGroup, h_resizable};
use gpui_kit::component::resizable::resizable_panel;

/// 外层布局的派生尺寸与可见性：把窗口宽度、侧栏实测宽度与面板折叠状态
/// 折算成 resizable 面板需要的像素值。
pub(super) struct WorkspaceChromeLayout {
    pub(super) show_sftp: bool,
    pub(super) show_forwards: bool,
    pub(super) sftp_width: f32,
    pub(super) sftp_min_width: f32,
    pub(super) sftp_max_width: f32,
    pub(super) side_left: bool,
}

/// 工作区分组构建输入：布局计划、侧栏面板与尺寸状态。
pub(super) struct WorkspaceGroupInput<'a> {
    pub(super) resizable_id: &'static str,
    pub(super) plan: &'a WorkspaceChromeLayout,
    pub(super) sidebar_panel: ResizablePanel,
    pub(super) panels_state: &'a Entity<ResizableState>,
    pub(super) p: &'a Palette,
    pub(super) rem: Pixels,
}

/// 工作区状态栏布局输入：宽度切分所需的调色板、密度与分栏状态。
pub(super) struct StatusBarLayoutInput<'a> {
    pub(super) p: &'a Palette,
    pub(super) rem: Pixels,
    pub(super) panels_state: &'a Entity<ResizableState>,
    pub(super) sidebar_min_width: Pixels,
    pub(super) sidebar_max_width: Pixels,
}

impl AppShell {
    /// 计算外层布局。设计原理（Why）：这些算术只在渲染时需要，抽出来后
    /// `render` 只负责按 plan 拼装面板，宽度规则可以单独核对与调整。
    pub(super) fn plan_workspace_layout(&self, window: &Window, cx: &App) -> WorkspaceChromeLayout {
        let rem = window.rem_size();
        let active_panel = self
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| self.state.workspace.tab(tab_id));
        let sftp_panel = self
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| self.sftp_state(tab_id, cx));
        let show_sftp = sftp_panel.is_some_and(|panel| panel.open);
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
        let show_forwards = active_panel.is_some_and(|tab| tab.forwards_view().is_some());
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
        let sidebar_width = if self.state.sidebar.visible {
            let panels_state = if app_settings(cx).window.sftp_panel_side.is_left() {
                &self.state.workspace_panels_left
            } else {
                &self.state.workspace_panels
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
            side_left: app_settings(cx).window.sftp_panel_side.is_left(),
        }
    }

    /// 左侧槽位面板：纵向标签或会话树，折叠时仅隐藏不退出分组。
    pub(super) fn build_sidebar_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
        rem: Pixels,
    ) -> ResizablePanel {
        let sidebar_min_width = rem_px(size::SIDEBAR_MIN_WIDTH, rem);
        let sidebar_max_width = rem_px(size::SIDEBAR_MAX_WIDTH, rem);
        let mut sidebar_panel = resizable_panel()
            .size(rem_px(size::SIDEBAR_WIDTH, rem))
            .size_range(sidebar_min_width..sidebar_max_width)
            .fixed()
            .visible(self.state.sidebar.visible);
        if self.state.sidebar.visible {
            // 左侧槽位一次只承载一种内容：纵向标签面板或会话树。
            let content = if self.show_vertical_tabs_panel(app_settings(cx)) {
                self.render_vertical_tabs_panel(p, cx)
            } else {
                self.render_sidebar(window, cx, p)
            };
            sidebar_panel = sidebar_panel.child(content);
        }
        sidebar_panel
    }

    /// 工作区可拖拽分组：侧栏、主面板、SFTP/转发与监控抽屉。
    pub(super) fn build_resizable_group(
        &mut self,
        input: WorkspaceGroupInput<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ResizablePanelGroup {
        let WorkspaceGroupInput {
            resizable_id,
            plan,
            sidebar_panel,
            panels_state,
            p,
            rem,
        } = input;
        let side_left = plan.side_left;
        let show_sftp = plan.show_sftp;
        let show_forwards = plan.show_forwards;
        let mut resizable_group = if self.state.immersive {
            // 沉浸模式只保留主面板，使用独立分组与临时状态；
            // 严禁复用 panels_state，否则主面板在全屏下会覆盖索引 0（侧栏）的尺寸记录，
            // 导致退出沉浸模式后侧边栏继承全屏宽度并撑满窗口。
            h_resizable(resizable_id)
        } else {
            h_resizable(resizable_id).with_state(panels_state)
        };
        if !self.state.immersive {
            resizable_group = resizable_group.child(sidebar_panel);
        }
        if !self.state.immersive && side_left {
            // 停靠左：SFTP 与转发整体挪到主面板之前，贴着侧栏。
            if show_sftp {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(px(plan.sftp_width.min(plan.sftp_max_width)))
                        .size_range(px(plan.sftp_min_width)..px(plan.sftp_max_width))
                        .fixed()
                        .child(self.render_sftp_panel_element()),
                );
            }
            if show_forwards {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(rem_px(size::FORWARD_PANEL_WIDTH, rem))
                        .fixed()
                        .child(self.forward_view_element(cx)),
                );
            }
        }
        resizable_group =
            resizable_group.child(resizable_panel().child(self.render_main(window, cx, p)));
        if !self.state.immersive && !side_left {
            if show_sftp {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(px(plan.sftp_width.min(plan.sftp_max_width)))
                        .size_range(px(plan.sftp_min_width)..px(plan.sftp_max_width))
                        .fixed()
                        .child(self.render_sftp_panel_element()),
                );
            }
            if show_forwards {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(rem_px(size::FORWARD_PANEL_WIDTH, rem))
                        .fixed()
                        .child(self.forward_view_element(cx)),
                );
            }
        }

        if let Some(monitor_width) = self.monitor_drawer_width(window, cx, panels_state) {
            resizable_group = resizable_group.child(
                resizable_panel()
                    .size(px(monitor_width))
                    .size_range(
                        px(360.0)..px((window.viewport_size().width.as_f32() * 0.75).max(520.0)),
                    )
                    .child(self.state.monitor.clone()),
            );
        }

        resizable_group
    }

    /// 监控抽屉宽度：同步实测宽度到状态与设置，返回到布局使用的宽度。
    pub(super) fn monitor_drawer_width(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        panels_state: &Entity<ResizableState>,
    ) -> Option<f32> {
        if self.state.immersive || !self.state.monitor.read(cx).is_open() {
            return None;
        }
        let max_drawer_width = (window.viewport_size().width.as_f32() * 0.75).max(520.0);
        if let Some(actual_w) = panels_state.read(cx).sizes().last().copied() {
            let actual_f32 = actual_w.as_f32();
            if actual_f32 >= 360.0
                && actual_f32 <= max_drawer_width
                && (self.state.monitor.read(cx).drawer_width() - actual_f32).abs() > 1.0
            {
                self.state
                    .monitor
                    .update(cx, |view, _| view.set_drawer_width(actual_f32));
                update_app_settings(cx, |s| s.monitor.drawer_width = actual_f32);
            }
        }
        Some(
            self.state
                .monitor
                .read(cx)
                .drawer_width()
                .clamp(360.0, max_drawer_width),
        )
    }
}
