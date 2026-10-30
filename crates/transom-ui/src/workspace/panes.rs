//! 标签内分屏渲染：按分割树铺满内容区，每一刀都是可拖拽的分隔线。

use crate::app::AppShell;
use crate::divider::{h_resizable, v_resizable};
use crate::palette::Palette;
use crate::pane_tree::LayoutNode;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::resizable::{ResizableState, resizable_panel};
use gpui_kit::*;
use transom_core::{SplitAxis, SplitId, TabId};

/// 渲染一帧窗格树时共享的上下文。
///
/// 递归渲染子节点时只读地需要活动窗格、最大化目标与视觉序号，而 `self` 在同一
/// 趟渲染里还要被可变借用去取终端实体；因此先把这些标量从标签组里取出来。
struct PaneFrame<'a> {
    palette: &'a Palette,
    active: TabId,
    zoomed: Option<TabId>,
    order: &'a [TabId],
}

impl AppShell {
    pub(super) fn render_active_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        // 并排网格是持久的，但只有活动标签属于某张网格时才铺格子；否则单屏显示
        // 当前标签，那张网格保留在标签栏里等用户点回来。
        if let Some(tiled) = self.active_tiling().cloned()
            && let Some(content) = self.render_tiled_content(&tiled, window, cx, p)
        {
            return content;
        }
        let Some(group_index) = self.active_group_index() else {
            return self.render_empty_state(window, cx, p);
        };
        self.render_group_content(group_index, window, cx, p)
    }

    /// 渲染一个标签组的完整内容：单窗格、最大化或按分割树铺满。
    ///
    /// 从 `render_active_content` 抽出来，供单标签视图与并排视图的两格共用；
    /// 组内布局与拖拽比例在两种视图下完全一致。
    pub(super) fn render_group_content(
        &mut self,
        group_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let group = &self.workspace.groups[group_index];
        // 克隆布局树：渲染期间会改动 self.workspace.tabs，避免与 self.workspace.groups 的借用冲突。
        let root = group.root.clone();
        let order = root.pane_ids();
        let frame = PaneFrame {
            palette: p,
            active: group.active_pane,
            zoomed: group.zoomed,
            order: &order,
        };

        if let Some(pane) = frame.zoomed.filter(|pane| order.contains(pane)) {
            // 最大化：只铺这一个窗格。布局树与拖拽状态都没被改动，恢复即原样。
            return self.render_split_pane(pane, &frame, window, cx);
        }
        if let [only] = order.as_slice() {
            // 单窗格不显示窗格头：状态与标题已由标签栏表达，再叠一条纯属重复。
            return self.render_pane_body(*only, window, cx, p);
        }
        self.render_layout(&root, &frame, window, cx)
    }

    /// 递归渲染分割树：内部节点用可拖拽容器把空间分给两个子节点。
    fn render_layout(
        &mut self,
        node: &LayoutNode,
        frame: &PaneFrame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            LayoutNode::Pane(tab) => self.render_split_pane(*tab, frame, window, cx),
            LayoutNode::Split {
                id,
                axis,
                first,
                second,
            } => {
                let first_el = self.render_layout(first, frame, window, cx);
                let second_el = self.render_layout(second, frame, window, cx);
                let state = self.pane_split_state(*id, cx);
                let group = match axis {
                    SplitAxis::Horizontal => h_resizable(split_element_id(*id)),
                    SplitAxis::Vertical => v_resizable(split_element_id(*id)),
                }
                .with_state(&state)
                .child(resizable_panel().child(first_el))
                .child(resizable_panel().child(second_el));
                div().flex_1().size_full().child(group).into_any_element()
            }
        }
    }

    /// 取（必要时创建）某个分割节点的拖拽状态。
    ///
    /// 状态由 AppShell 持有而不是交给 gpui 的 keyed state：后者只在元素连续出现
    /// 的帧里存活，切换标签或单窗格最大化后再回来，用户拖出的比例会被清零。
    pub(super) fn pane_split_state(
        &mut self,
        id: SplitId,
        cx: &mut Context<Self>,
    ) -> Entity<ResizableState> {
        if let Some(state) = self.workspace.pane_split_states.get(&id) {
            return state.clone();
        }
        let state = cx.new(|_| ResizableState::default());
        self.workspace.pane_split_states.insert(id, state.clone());
        state
    }

    /// 丢弃已从分割树上消失的拖拽状态，避免长期积累无用实体。
    ///
    /// 只在布局被改动后调用（渲染期不能调用：那会儿新节点的状态刚刚登记）。
    pub(crate) fn prune_pane_split_states(&mut self) {
        self.workspace.reconcile_split_states();
    }

    /// 渲染单个窗格：细条头部（状态灯、序号、标题、关闭）加终端本体。
    fn render_split_pane(
        &mut self,
        tab_id: TabId,
        frame: &PaneFrame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(index) = self.workspace.tab_index(tab_id) else {
            return div().into_any_element();
        };
        let p = frame.palette;
        let active = tab_id == frame.active;
        let ordinal = frame
            .order
            .iter()
            .position(|id| *id == tab_id)
            .map_or(1, |position| position + 1);
        let zoomed_here = frame.zoomed == Some(tab_id);
        let app_entity = cx.entity().clone();
        let (dot_color, status_icon) =
            super::pane_status_indicator(&self.workspace.tabs[index].status, p);
        let (zoom_label, zoom_icon) = if zoomed_here {
            (
                rust_i18n::t!("menu.restore_panes").to_string(),
                IconName::Minimize2,
            )
        } else {
            (
                rust_i18n::t!("menu.zoom_pane").to_string(),
                IconName::Maximize2,
            )
        };

        let header = super::pane_header::pane_header(
            super::pane_header::PaneHeaderProps {
                id: format!("pane-header-{tab_id}"),
                close_id: format!("close-pane-{tab_id}"),
                active,
                dot_color,
                icon: status_icon,
                title: self.workspace.tabs[index].display_name(),
                ordinal: Some(ordinal),
                palette: p,
            },
            cx.listener(move |this, _, window, cx| {
                this.activate_tab(tab_id, window, cx);
            }),
            cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.close_pane(tab_id, window, cx);
            }),
        )
        .context_menu(move |menu, _window, _cx| {
            let app = app_entity.clone();
            menu.item(
                PopupMenuItem::new(rust_i18n::t!("menu.split_right").to_string())
                    .icon(IconName::Columns2)
                    .on_click({
                        let app = app.clone();
                        move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.activate_tab(tab_id, window, cx);
                                this.split_active_pane(SplitAxis::Horizontal, window, cx);
                            });
                        }
                    }),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("menu.split_down").to_string())
                    .icon(IconName::Rows2)
                    .on_click({
                        let app = app.clone();
                        move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.activate_tab(tab_id, window, cx);
                                this.split_active_pane(SplitAxis::Vertical, window, cx);
                            });
                        }
                    }),
            )
            .item(PopupMenuItem::separator())
            .item(
                PopupMenuItem::new(zoom_label.clone())
                    .icon(zoom_icon)
                    .on_click({
                        let app = app.clone();
                        move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.activate_tab(tab_id, window, cx);
                                this.toggle_zoom_pane(tab_id, cx);
                            });
                        }
                    }),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("menu.detach_pane").to_string())
                    .icon(IconName::ArrowUpRight)
                    .on_click({
                        let app = app.clone();
                        move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.activate_tab(tab_id, window, cx);
                                this.detach_pane_to_tab(tab_id, window, cx);
                            });
                        }
                    }),
            )
        });

        let body = self.render_pane_body(tab_id, window, cx, p);
        v_flex()
            .size_full()
            .overflow_hidden()
            .child(header)
            .child(body)
            // 点窗格任意位置即接管焦点：终端自身会消费事件，冒泡到这里只发生在
            // 头部与空白区，代价是每次点击多一次焦点设置，换来不必非点标题条。
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.activate_tab(tab_id, window, cx);
                }),
            )
            .into_any_element()
    }

    fn render_pane_body(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(index) = self.workspace.tab_index(tab_id) else {
            return div().into_any_element();
        };
        if let Some(player) = self.workspace.tabs[index].player() {
            return div().size_full().child(player).into_any_element();
        }
        if let crate::app::TabContent::Monitor(target_id) = self.workspace.tabs[index].content {
            return self.render_monitor_tab_content(tab_id, target_id, window, cx, p);
        }
        if let Some(terminal) = self.workspace.tabs[index].terminal() {
            // 拖拽上传的进度浮层锚在这个容器右下角：它是"这个窗格的终端区"的
            // 最小外包块，绝对定位因此不受侧栏与面板列宽度变化的影响。
            let overlay = self.render_drop_overlay(tab_id, p, cx);
            div()
                .flex_1()
                .relative()
                .overflow_hidden()
                .child(terminal)
                .children(overlay)
                .into_any_element()
        } else {
            self.render_tab_status(index, window, cx, p)
        }
    }
}

/// 分割节点的可拖拽容器标识：同一 SplitId 在任意一帧都拿到同一个 ElementId，
/// 兄弟窗格增删后祖先重建也不会让子树误当成新元素而重置尺寸。
pub(super) fn split_element_id(id: SplitId) -> ElementId {
    ElementId::Name(format!("pane-split-{id}").into())
}
