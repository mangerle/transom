//! 工作区布局计划与可拖拽分组构建。
//!
//! 布局算术集中在 `plan_workspace_layout`，分组插入顺序集中在
//! `build_resizable_group`，两者只依赖窗口尺寸、状态与调色板。

use crate::app::*;
use crate::palette::Palette;
use crate::sftp::SftpPanelState;
use crate::sidebar::{SidebarSection, SidebarSlotLayout, sync_sidebar_slot};
use crate::theme::tokens::{rem_px, size};
mod layout;

use gpui_kit::base::{ResizablePanel, ResizablePanelGroup, h_resizable};
use gpui_kit::component::resizable::{ResizableState, resizable_panel};
use gpui_kit::*;

/// 外层布局的派生尺寸与可见性：把窗口宽度、侧栏实测宽度与面板折叠状态
/// 折算成 resizable 面板需要的像素值。
pub(crate) struct WorkspaceChromeLayout {
    pub(crate) show_sftp: bool,
    pub(crate) show_forwards: bool,
    pub(crate) sftp_width: f32,
    pub(crate) sftp_min_width: f32,
    pub(crate) sftp_max_width: f32,
    pub(crate) side_left: bool,
}

/// 工作区分组构建输入：布局计划、侧栏面板与尺寸状态。
pub(crate) struct WorkspaceGroupInput<'a> {
    pub(crate) resizable_id: &'static str,
    pub(crate) plan: &'a WorkspaceChromeLayout,
    pub(crate) sidebar_panel: ResizablePanel,
    pub(crate) panels_state: &'a Entity<ResizableState>,
    pub(crate) p: &'a Palette,
    pub(crate) rem: Pixels,
}

/// 工作区状态栏布局输入：宽度切分所需的调色板、密度与侧栏宽度。
pub(crate) struct StatusBarLayoutInput<'a> {
    pub(crate) p: &'a Palette,
    pub(crate) rem: Pixels,
    pub(crate) sidebar_visible: bool,
    pub(crate) sidebar_width: f32,
}

impl AppShell {
    /// 同步侧栏在可拖拽分组里的槽位宽度，返回本帧侧栏应使用的宽度。
    ///
    /// 设计原理（Why）：`ResizableState` 按面板索引记录宽度，侧栏收起后
    /// 0 号面板不再参与布局与测量，尺寸表却留着收起前的宽度；停靠侧切换还会
    /// 换成另一份分栏状态。这里在构建分组之前把槽位修正到与布局一致，
    /// 否则随后拖动任意分隔条都会用过期宽度计算差值，表现为面板跳动。
    pub(crate) fn sync_sidebar_panel_slot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> f32 {
        if self.state.ui.immersive {
            return 0.0;
        }
        let rem = window.rem_size();
        let side_left = app_settings(cx).window.sftp_panel_side.is_left();
        let active_tab = self.state.workspace.active_tab;
        let show_sftp = active_tab
            .and_then(|tab_id| crate::sftp::sftp_state(&self.state.workspace, tab_id, cx))
            .is_some_and(|panel| panel.open);
        let show_forwards = active_tab
            .and_then(|tab_id| self.state.workspace.tab(tab_id))
            .is_some_and(|tab| tab.forwards_view().is_some());
        // 侧栏之后的固定面板（SFTP、转发）在停靠左侧时紧邻 0 号槽位；
        // 写入侧栏宽度会先挤窄它们，同步函数需要据此恢复原宽度。
        let main_index = if side_left {
            1 + usize::from(show_sftp) + usize::from(show_forwards)
        } else {
            1
        };
        let panels = if side_left {
            self.state.ui.workspace_panels_left.clone()
        } else {
            self.state.ui.workspace_panels.clone()
        };
        let sidebar_visible = self.state.session.sidebar.visible();
        sync_sidebar_slot(
            &mut self.state.session.sidebar.slot,
            &panels,
            SidebarSlotLayout {
                visible: sidebar_visible,
                default_width: rem_px(size::SIDEBAR_WIDTH, rem).as_f32(),
                min_width: rem_px(size::SIDEBAR_MIN_WIDTH, rem).as_f32(),
                max_width: rem_px(size::SIDEBAR_MAX_WIDTH, rem).as_f32(),
                main_index,
            },
            window,
            cx,
        )
    }

    /// 计算外层布局。设计原理（Why）：这些算术只在渲染时需要，抽出来后
    /// `render` 只负责按 plan 拼装面板，宽度规则可以单独核对与调整。
    ///
    /// `sidebar_width` 由 [`Self::sync_sidebar_panel_slot`] 给出，与侧栏面板
    /// 实际占位一致；不再从尺寸表反查，避免过渡帧读到旧宽度。
    pub(crate) fn plan_workspace_layout(
        &self,
        window: &Window,
        cx: &App,
        sidebar_width: f32,
    ) -> WorkspaceChromeLayout {
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
            .and_then(|tab_id| crate::sftp::sftp_state(&self.state.workspace, tab_id, cx));
        let sftp_plan = layout::sftp_layout_plan(sftp_panel, rem);
        let show_sftp = sftp_plan.show;
        let sftp_width = sftp_plan.width;
        let sftp_min_width = sftp_plan.min_width;
        let sftp_all_collapsed = sftp_panel.is_some_and(|panel| panel.is_all_collapsed());
        let sftp_collapsed_width = sftp_panel
            .map(SftpPanelState::all_collapsed_width)
            .unwrap_or(32.0);
        let show_forwards = active_panel.is_some_and(|tab| tab.forwards_view().is_some());
        let sidebar_width = if self.state.session.sidebar.visible() {
            sidebar_width
        } else {
            0.0
        };
        // 固定面板不能把右侧关闭按钮挤出窗口：按当前窗口宽度给 SFTP 一个上限。
        // 预留活动栏、侧栏、前向面板与主面板最小宽度，剩余空间才归 SFTP。
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
    pub(crate) fn build_sidebar_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
        rem: Pixels,
        sidebar_width: f32,
    ) -> ResizablePanel {
        let sidebar_min_width = rem_px(size::SIDEBAR_MIN_WIDTH, rem);
        let sidebar_max_width = rem_px(size::SIDEBAR_MAX_WIDTH, rem);
        let mut sidebar_panel = resizable_panel()
            .size(px(sidebar_width))
            .size_range(sidebar_min_width..sidebar_max_width)
            .fixed()
            .visible(self.state.session.sidebar.visible());
        if self.state.session.sidebar.visible() {
            // 左侧槽位一次只承载一种内容：容器连接、Git 仓库、云同步、纵向标签或会话树。
            let section = self
                .state
                .session
                .sidebar
                .section()
                .resolved_for_tab_position(app_settings(cx).window.tab_position);
            let content = match section {
                SidebarSection::Containers => {
                    // 目录与连接状态由外壳按渲染快照注入，侧栏实体不反查外壳。
                    let sidebar = self.state.features.containers.sidebar.clone();
                    let profile_states = self.state.container_profile_states(cx);
                    let active = self.state.active_engine_profile_id();
                    let state = sidebar.read(cx);
                    crate::container::render_container_sidebar(
                        crate::container::ContainerSidebarParams {
                            view: &sidebar,
                            state: &state.state,
                            cx,
                            catalog: &self.state.session.catalog,
                            active,
                            profile_states: &profile_states,
                        },
                        p,
                    )
                }
                SidebarSection::Git => {
                    let active_project = self
                        .state
                        .workspace
                        .active_tab
                        .and_then(|tab_id| self.state.git_workspace(tab_id))
                        .map(|workspace| workspace.read(cx).state.project.id);
                    let mut active_branches = std::collections::HashMap::with_capacity(
                        self.state.git_registry(cx).projects.len(),
                    );
                    for project in &self.state.git_registry(cx).projects {
                        if let Some(workspace) = self.state.git_workspace_of_project(project.id, cx)
                            && let Some(transom_core::HeadState::OnBranch(name)) =
                                workspace.read(cx).state.status.as_ref().map(|s| &s.head)
                        {
                            active_branches.insert(project.id, name.clone());
                        }
                    }
                    crate::git::sidebar::render_git_sidebar(
                        crate::git::GitSidebarParams {
                            view: &self.state.features.git_sidebar,
                            git: self.state.git_registry(cx),
                            active_project,
                            active_branches: &active_branches,
                        },
                        p,
                    )
                }
                SidebarSection::CloudSync => {
                    self.state.features.cloud_sync.clone().into_any_element()
                }
                SidebarSection::Tabs if self.show_vertical_tabs_panel(app_settings(cx)) => {
                    self.render_vertical_tabs_panel(p, cx)
                }
                SidebarSection::Sessions | SidebarSection::Tabs => {
                    let query = self
                        .state
                        .session
                        .sidebar
                        .search
                        .read(cx)
                        .value()
                        .to_string();
                    let rows = self.state.sidebar_rows(&query);
                    let active_session = self.state.active_session_id();
                    crate::sidebar::render_sidebar(
                        crate::sidebar::SessionSidebarParams {
                            view: &self.state.session.session_sidebar,
                            search: &self.state.session.sidebar.search,
                            rows,
                            query: &query,
                            selection_mode: self.state.session.sidebar.selection_mode,
                            selected_sessions: &self.state.session.sidebar.selected_sessions,
                            tree_selection: self.state.session.sidebar.tree_selection,
                            active_session,
                            probe_badges: &self.state.session.probe_badges,
                        },
                        window,
                        p,
                    )
                }
            };
            sidebar_panel = sidebar_panel.child(content);
        }
        sidebar_panel
    }

    /// 工作区可拖拽分组：侧栏、主面板、SFTP/转发与监控抽屉。
    pub(crate) fn build_resizable_group(
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
        let mut resizable_group = if self.state.ui.immersive {
            // 沉浸模式只保留主面板，使用独立分组与临时状态；
            // 严禁复用 panels_state，否则主面板在全屏下会覆盖索引 0（侧栏）的尺寸记录，
            // 导致退出沉浸模式后侧边栏继承全屏宽度并撑满窗口。
            h_resizable(resizable_id)
        } else {
            h_resizable(resizable_id).with_state(panels_state)
        };
        // 抽屉之前的面板数：只有尺寸表已经包含抽屉槽位时，最后一项才是抽屉宽度。
        let mut panels_before_drawer = 0usize;
        if !self.state.ui.immersive {
            resizable_group = resizable_group.child(sidebar_panel);
            panels_before_drawer += 1;
        }
        if !self.state.ui.immersive && side_left {
            // 停靠左：SFTP 与转发整体挪到主面板之前，贴着侧栏。
            if show_sftp {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(px(plan.sftp_width.min(plan.sftp_max_width)))
                        .size_range(px(plan.sftp_min_width)..px(plan.sftp_max_width))
                        .fixed()
                        .child(
                            self.state
                                .workspace
                                .active_tab
                                .and_then(|tab_id| self.state.sftp_view(tab_id))
                                .map(IntoElement::into_any_element)
                                .unwrap_or_else(|| div().into_any_element()),
                        ),
                );
                panels_before_drawer += 1;
            }
            if show_forwards {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(rem_px(size::FORWARD_PANEL_WIDTH, rem))
                        .fixed()
                        .child(self.forward_view_element(cx)),
                );
                panels_before_drawer += 1;
            }
        }
        resizable_group =
            resizable_group.child(resizable_panel().child(self.render_main(window, cx, p)));
        panels_before_drawer += 1;
        if !self.state.ui.immersive && !side_left {
            if show_sftp {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(px(plan.sftp_width.min(plan.sftp_max_width)))
                        .size_range(px(plan.sftp_min_width)..px(plan.sftp_max_width))
                        .fixed()
                        .child(
                            self.state
                                .workspace
                                .active_tab
                                .and_then(|tab_id| self.state.sftp_view(tab_id))
                                .map(IntoElement::into_any_element)
                                .unwrap_or_else(|| div().into_any_element()),
                        ),
                );
                panels_before_drawer += 1;
            }
            if show_forwards {
                resizable_group = resizable_group.child(
                    resizable_panel()
                        .size(rem_px(size::FORWARD_PANEL_WIDTH, rem))
                        .fixed()
                        .child(self.forward_view_element(cx)),
                );
                panels_before_drawer += 1;
            }
        }

        if let Some(monitor_width) =
            self.monitor_drawer_width(window, cx, panels_state, panels_before_drawer)
        {
            resizable_group = resizable_group.child(
                resizable_panel()
                    .size(px(monitor_width))
                    .size_range(
                        px(360.0)..px((window.viewport_size().width.as_f32() * 0.75).max(520.0)),
                    )
                    // 与侧栏、SFTP 一样属于用户定宽的固定面板：窗口缩放时由主面板
                    // 吸收差额，抽屉宽度不被按比例重新分配。
                    .fixed()
                    .child(self.state.features.monitor.clone()),
            );
        }

        resizable_group
    }

    /// 监控抽屉宽度：同步实测宽度到状态与设置，返回到布局使用的宽度。
    ///
    /// `panels_before_drawer` 是本帧抽屉之前的面板数。尺寸表按面板索引记录宽度，
    /// 抽屉关闭时它没有槽位，最后一项是 SFTP 或主面板；只有确认尺寸表已经包含
    /// 抽屉槽位，最后一项才能当抽屉宽度回写。否则打开抽屉的第一帧会拿相邻面板
    /// 的宽度当抽屉宽度，抽屉先按错误宽度出现一帧再跳回来。
    pub(crate) fn monitor_drawer_width(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        panels_state: &Entity<ResizableState>,
        panels_before_drawer: usize,
    ) -> Option<f32> {
        if self.state.ui.immersive || !self.state.features.monitor.read(cx).is_open() {
            return None;
        }
        let max_drawer_width = (window.viewport_size().width.as_f32() * 0.75).max(520.0);
        // 只借用尺寸表取一次末项，避免为一次判断克隆整张表
        let measured = {
            let state = panels_state.read(cx);
            let sizes = state.sizes();
            layout::drawer_slot_measured(sizes.len(), panels_before_drawer)
                .then(|| sizes.last().copied())
                .flatten()
        };
        if let Some(actual_w) = measured {
            let actual_f32 = actual_w.as_f32();
            if actual_f32 >= 360.0
                && actual_f32 <= max_drawer_width
                && (self.state.features.monitor.read(cx).drawer_width() - actual_f32).abs() > 1.0
            {
                self.state
                    .features
                    .monitor
                    .update(cx, |view, _| view.set_drawer_width(actual_f32));
                update_app_settings(cx, |s| s.monitor.drawer_width = actual_f32);
            }
        }
        Some(
            self.state
                .features
                .monitor
                .read(cx)
                .drawer_width()
                .clamp(360.0, max_drawer_width),
        )
    }
}
