//! 侧栏在可拖拽分组里的槽位宽度同步。
//!
//! # 为什么需要本模块
//! 工作区分组用 `ResizableState` 按面板索引记录宽度，拖动分隔条时以这张表
//! 和鼠标位置做差值计算。侧栏收起时只是把 0 号面板设为不可见：它不再参与
//! 布局与测量，但状态表里仍留着收起前的宽度，主面板被撑大的实测宽度也不会
//! 回写。此后任意一次分隔条拖动都会拿过期宽度算差值，面板随之跳动。
//!
//! # 处理方式
//! 显隐切换或停靠侧切换时，把 0 号槽位「移除再以 0 宽插入」：分栏状态会按
//! 容器宽度重算其余面板（主面板吸收余量），插入 0 宽不会按比例缩放其他固定
//! 面板。可见时同一帧先清掉槽位的记忆尺寸，让面板按构建元素上的记忆宽度排版，
//! 下一帧再把目标宽度写入尺寸表，避免收起或展开时闪一帧错误宽度。
//!
//! # 代价与局限
//! 宽度写入被延后一帧（等待面板完成一次测量以刷新尺寸区间）；这一帧内
//! 尺寸表与布局不一致，但视觉已经正确。正常点击与随后拖动分隔条之间必然
//! 隔着至少一帧，因此用户感知不到。

use gpui_kit::component::resizable::ResizableState;
use gpui_kit::{App, Entity, EntityId, Window, px};

/// 写入失败后的最大重试次数；超过后暂停同步，直到下一次显隐或实体切换。
const MAX_WRITE_ATTEMPTS: u8 = 2;

/// 侧栏槽位在可拖拽分组中的同步状态。
///
/// 这个状态跟着侧栏走（显隐与记忆宽度都是侧栏语义），不放在工作区状态里，
/// 避免停靠侧切换时被另一份分栏状态带走。
pub(crate) struct SidebarSlotSync {
    /// 侧栏最近一次生效的宽度（物理像素），用于收起后恢复。
    remembered_width: Option<f32>,
    /// 上次同步时承载侧栏槽位的分栏状态实体。
    state: Option<EntityId>,
    /// 上次同步时侧栏是否可见。
    visible: bool,
    /// 待写入的槽位宽度：重置当帧只按记忆宽度排版，下一帧再写入尺寸表。
    pending_width: Option<f32>,
    /// 上次同步时分组的面板数量：增减面板会让尺寸表里的主面板余量过期。
    panel_count: usize,
    /// 待写入宽度的尝试次数。
    attempts: u8,
    /// 写入失败后暂停同步，避免尺寸区间始终不满足时逐帧空转重绘。
    failed: bool,
}

impl SidebarSlotSync {
    /// 创建初始同步状态；`visible` 取启动时的侧栏显隐，避免首帧误判为切换。
    pub(crate) fn new(visible: bool) -> Self {
        Self {
            remembered_width: None,
            state: None,
            visible,
            panel_count: 0,
            pending_width: None,
            attempts: 0,
            failed: false,
        }
    }
}

/// 侧栏槽位同步所需的布局参数。
pub(crate) struct SidebarSlotLayout {
    /// 本帧侧栏是否可见。
    pub(crate) visible: bool,
    /// 侧栏默认宽度（密度换算后的物理像素）。
    pub(crate) default_width: f32,
    /// 侧栏允许的最小宽度。
    pub(crate) min_width: f32,
    /// 侧栏允许的最大宽度。
    pub(crate) max_width: f32,
    /// 主面板在分组里的索引；它之前（不含）的固定面板由侧栏让出的空间承担。
    pub(crate) main_index: usize,
}

/// 同步侧栏槽位，返回本帧侧栏面板应使用的宽度（收起时为 0）。
///
/// 设计原理（Why）：显隐切换只能通过构建元素上的初始宽度表达一帧，
/// 尺寸表必须在下一帧才写入；把两件事拆开可以同时保证本帧视觉正确与
/// 下一帧分隔条拖动基于一致数据。返回宽度供外层布局计算与状态栏分界线共用，
/// 避免它们各自从尺寸表读到过渡帧里的旧值。
///
/// # Errors
/// 本函数不返回错误；尺寸表重建或宽度写入未达成时记录警告日志并降级为
/// 暂停同步，由下一次显隐或停靠侧切换重新触发。
pub(crate) fn sync_sidebar_slot(
    slot: &mut SidebarSlotSync,
    panels: &Entity<ResizableState>,
    layout: SidebarSlotLayout,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    let state_id = panels.entity_id();
    let (sizes, container) = read_panel_sizes(panels, cx);
    let transition = slot.state != Some(state_id) || slot.visible != layout.visible;
    if transition {
        slot.state = Some(state_id);
        slot.visible = layout.visible;
        slot.pending_width = None;
        slot.attempts = 0;
        slot.failed = false;
        // 分栏状态尚未完成首次测量时不需要重建，首次测量会写入正确宽度。
        if sizes.len() >= 2 && container > 0.5 && !rebuild_slot(panels, layout.visible, cx) {
            slot.failed = true;
            slot.panel_count = sizes.len();
            return target_width(slot, &layout);
        }
        if layout.visible && sizes.len() >= 2 && container > 0.5 {
            slot.pending_width = Some(target_width(slot, &layout));
        }
        slot.panel_count = sizes.len();
        return target_width(slot, &layout);
    }

    if slot.failed {
        return target_width(slot, &layout);
    }

    if let Some(pending) = slot.pending_width {
        if write_slot_width(panels, pending, layout.main_index, window, cx) {
            slot.pending_width = None;
            slot.attempts = 0;
        } else {
            slot.attempts = slot.attempts.saturating_add(1);
            if slot.attempts >= MAX_WRITE_ATTEMPTS {
                log::warn!(
                    "侧栏槽位宽度写入失败，目标 {pending:.1}px；已暂停同步，等待下一次显隐或停靠侧切换"
                );
                slot.pending_width = None;
                slot.failed = true;
            }
        }
        return if layout.visible { pending } else { 0.0 };
    }

    // 稳态：面板增减会让尺寸表里的主面板余量过期，收起的槽位则必须保持 0 宽。
    // 这里按面板数量变化与槽位自身宽度判断，不按尺寸和与容器的差值判断：
    // 窗口被拖到极窄时固定面板本就装不下，按差值重建会把侧栏莫名收起。
    let count_changed = sizes.len() != slot.panel_count;
    slot.panel_count = sizes.len();
    let target = target_width(slot, &layout);
    let slot_drifted = match sizes.first().copied() {
        None => false,
        Some(width) if layout.visible => width < layout.min_width - 0.5,
        Some(width) => width > 0.5,
    };
    let needs_rebuild = sizes.len() >= 2 && container > 0.5 && (count_changed || slot_drifted);
    if needs_rebuild {
        if !rebuild_slot(panels, layout.visible, cx) {
            slot.failed = true;
            return target;
        }
        if !layout.visible {
            return 0.0;
        }
        slot.pending_width = Some(target);
    }

    // 用户拖动侧栏分隔条后，把实测宽度记为下一次收起前要恢复的记忆宽度。
    if layout.visible
        && let Some(width) = sizes.first().copied()
        && (layout.min_width - 0.5..=layout.max_width + 0.5).contains(&width)
    {
        slot.remembered_width = Some(width);
    }
    target_width(slot, &layout)
}

/// 本帧侧栏应使用的宽度；收起时为 0，可见时按记忆宽度并收敛到当前密度区间。
fn target_width(slot: &SidebarSlotSync, layout: &SidebarSlotLayout) -> f32 {
    if !layout.visible {
        return 0.0;
    }
    slot.remembered_width
        .unwrap_or(layout.default_width)
        .clamp(layout.min_width, layout.max_width)
}

/// 读取分栏尺寸表与容器宽度；一次借用取完，避免多次读实体。
fn read_panel_sizes(panels: &Entity<ResizableState>, cx: &App) -> (Vec<f32>, f32) {
    let state = panels.read(cx);
    let sizes = state.sizes().iter().map(|width| width.as_f32()).collect();
    (sizes, state.container_size().as_f32())
}

/// 把 0 号槽位重建为 0 宽，并让其余面板按容器剩余空间重新分配。
///
/// 先移除再插入是唯一能在公开 API 下写入 0 宽的方式：移除会触发按容器
/// 重算主面板余量，随后插入 0 宽不会按比例缩放其他固定面板；直接
/// `resize_panel(0, 0)` 会被旧的尺寸区间夹到最小宽度。
///
/// 返回重建后的尺寸表是否重新与容器一致。
fn rebuild_slot(panels: &Entity<ResizableState>, visible: bool, cx: &mut App) -> bool {
    panels.update(cx, |state, cx| {
        if state.sizes().len() < 2 {
            return;
        }
        state.remove_panel(0, cx);
        state.insert_panel(Some(px(0.0)), Some(0), cx);
        // 可见时清掉槽位的记忆尺寸，让本帧按构建元素上的初始宽度排版，
        // 下一帧再把目标宽度写入尺寸表。
        if visible {
            state.reset_panel(0, cx);
        }
    });
    let (sizes, container_after) = read_panel_sizes(panels, cx);
    let sum: f32 = sizes.iter().copied().sum();
    let consistent = (sum - container_after).abs() <= 0.5;
    if !consistent {
        log::warn!(
            "侧栏槽位重建后尺寸表仍与容器不一致：容器 {container_after:.1}px，尺寸和 {sum:.1}px"
        );
    }
    consistent
}

/// 把目标宽度写入 0 号槽位。
///
/// 停靠左侧时，侧栏展开会先挤窄紧随其后的固定面板（SFTP、转发）；写入后
/// 按原宽度恢复这些面板，让主面板承担宽度变化，固定面板宽度保持不变。
///
/// 返回 0 号槽位是否已达到目标宽度。
fn write_slot_width(
    panels: &Entity<ResizableState>,
    width: f32,
    main_index: usize,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let (sizes, _) = read_panel_sizes(panels, cx);
    if sizes.len() < 2 {
        return false;
    }
    let restore_end = main_index.min(sizes.len().saturating_sub(1));
    let restore: Vec<f32> = sizes
        .get(1..restore_end)
        .map(<[f32]>::to_vec)
        .unwrap_or_default();
    panels.update(cx, |state, cx| {
        state.resize_panel(0, px(width), window, cx);
        for (offset, original) in restore.iter().enumerate() {
            state.resize_panel(offset + 1, px(*original), window, cx);
        }
    });
    read_panel_sizes(panels, cx)
        .0
        .first()
        .is_some_and(|actual| (actual - width).abs() <= 0.5)
}

#[cfg(test)]
mod tests {
    use super::{SidebarSlotLayout, SidebarSlotSync, sync_sidebar_slot};
    use gpui_kit::base::h_resizable;
    use gpui_kit::component::Root;
    use gpui_kit::component::resizable::{ResizableState, resizable_panel};
    use gpui_kit::prelude::FluentBuilder;
    use gpui_kit::{
        AppContext, Bounds, Context, Entity, InteractiveElement, IntoElement, Modifiers,
        MouseButton, ParentElement, Pixels, Render, Size, Styled, TestAppContext,
        VisualTestContext, Window, div, init, point, px,
    };
    use std::cell::{Cell, OnceCell};
    use std::rc::Rc;

    /// 侧栏默认宽度。
    const SIDEBAR_WIDTH: f32 = 260.0;
    /// 侧栏允许的最小宽度。
    const SIDEBAR_MIN: f32 = 200.0;
    /// 侧栏允许的最大宽度。
    const SIDEBAR_MAX: f32 = 400.0;
    /// SFTP 面板初始宽度。
    const SFTP_WIDTH: f32 = 760.0;
    /// SFTP 面板最小宽度。
    const SFTP_MIN: f32 = 360.0;
    /// 转发面板宽度。
    const FORWARDS_WIDTH: f32 = 320.0;
    /// 窗口宽度。
    const WINDOW_WIDTH: f32 = 1600.0;

    /// 探针宿主：与工作区一致的槽位顺序（侧栏 / 可选 SFTP / 主面板）。
    struct SlotProbe {
        panels: Entity<ResizableState>,
        slot: SidebarSlotSync,
        visible: Rc<Cell<bool>>,
        sftp_open: Rc<Cell<bool>>,
        forwards_open: Rc<Cell<bool>>,
        side_left: Rc<Cell<bool>>,
    }

    impl Render for SlotProbe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let visible = self.visible.get();
            let sftp_open = self.sftp_open.get();
            let forwards_open = self.forwards_open.get();
            let side_left = self.side_left.get();
            let main_index = if side_left {
                1 + usize::from(sftp_open) + usize::from(forwards_open)
            } else {
                1
            };
            let width = sync_sidebar_slot(
                &mut self.slot,
                &self.panels,
                SidebarSlotLayout {
                    visible,
                    default_width: SIDEBAR_WIDTH,
                    min_width: SIDEBAR_MIN,
                    max_width: SIDEBAR_MAX,
                    main_index,
                },
                window,
                cx,
            );
            let sidebar = resizable_panel()
                .size(px(width))
                .size_range(px(SIDEBAR_MIN)..px(SIDEBAR_MAX))
                .fixed()
                .visible(visible)
                .when(visible, |panel| {
                    panel.child(div().size_full().debug_selector(|| "sidebar".to_string()))
                });
            let sftp = || {
                resizable_panel()
                    .size(px(SFTP_WIDTH))
                    .size_range(px(SFTP_MIN)..px(1320.0))
                    .fixed()
                    .child(div().size_full().debug_selector(|| "sftp".to_string()))
            };
            let forwards = || {
                resizable_panel()
                    .size(px(FORWARDS_WIDTH))
                    .fixed()
                    .child(div().size_full().debug_selector(|| "forwards".to_string()))
            };
            let mut group = h_resizable("sidebar-slot-probe")
                .with_state(&self.panels)
                .child(sidebar);
            if side_left {
                if sftp_open {
                    group = group.child(sftp());
                }
                if forwards_open {
                    group = group.child(forwards());
                }
            }
            group = group.child(
                resizable_panel().child(div().size_full().debug_selector(|| "main".to_string())),
            );
            if !side_left {
                if sftp_open {
                    group = group.child(sftp());
                }
                if forwards_open {
                    group = group.child(forwards());
                }
            }
            div().w(px(WINDOW_WIDTH)).h(px(400.0)).child(group)
        }
    }

    /// 画一帧。
    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// 面板实测宽度。
    fn width(cx: &mut VisualTestContext, selector: &'static str) -> f32 {
        let bounds: Bounds<Pixels> = cx.debug_bounds(selector).expect("面板应已绘制");
        bounds.size.width.as_f32()
    }

    /// 连续画若干帧，覆盖重建、待写入与稳定三个阶段。
    fn draw_until_settled(cx: &mut VisualTestContext) {
        for _ in 0..3 {
            draw(cx);
        }
    }

    /// 切换探针的可变条件并请求重绘。
    fn toggle(probe: &Entity<SlotProbe>, cx: &mut VisualTestContext, change: impl FnOnce()) {
        change();
        cx.update(|_, cx| probe.update(cx, |_, cx| cx.notify()));
    }

    /// 无头窗口里跑一段场景：起窗口、画到稳定，然后把上下文交给场景函数。
    fn with_probe(
        initial_visible: bool,
        side_left: bool,
        sftp_open: bool,
        forwards_open: bool,
        scenario: impl FnOnce(
            &mut VisualTestContext,
            &Entity<SlotProbe>,
            &Rc<Cell<bool>>,
            &Rc<Cell<bool>>,
        ),
    ) {
        let mut app = TestAppContext::single();
        app.update(|cx| {
            init(cx);
            cx.set_reduce_motion(true);
        });
        let visible = Rc::new(Cell::new(initial_visible));
        let sftp_open = Rc::new(Cell::new(sftp_open));
        let forwards_open = Rc::new(Cell::new(forwards_open));
        let side_left = Rc::new(Cell::new(side_left));
        let probe_slot: Rc<OnceCell<Entity<SlotProbe>>> = Rc::new(OnceCell::new());
        let (_, cx) = {
            let visible = Rc::clone(&visible);
            let sftp_open = Rc::clone(&sftp_open);
            let forwards_open = Rc::clone(&forwards_open);
            let side_left = Rc::clone(&side_left);
            let probe_slot = Rc::clone(&probe_slot);
            app.add_window_view(move |window, cx| {
                let panels = cx.new(|_| ResizableState::default());
                let probe = cx.new(|_| SlotProbe {
                    panels,
                    slot: SidebarSlotSync::new(initial_visible),
                    visible,
                    sftp_open,
                    forwards_open,
                    side_left,
                });
                let _ = probe_slot.set(probe.clone());
                Root::new(probe, window, cx)
            })
        };
        cx.simulate_resize(Size {
            width: px(WINDOW_WIDTH),
            height: px(800.),
        });
        draw_until_settled(cx);
        let probe = probe_slot.get().expect("探针实体应已创建").clone();
        scenario(cx, &probe, &visible, &sftp_open);
    }

    /// 在 SFTP 与主面板之间的分隔条上按下并向右拖动 `distance` 像素。
    fn drag_sftp_handle(cx: &mut VisualTestContext, side_left: bool, distance: f32) {
        let sftp = cx.debug_bounds("sftp").expect("SFTP 应已绘制");
        // 停靠右侧时 SFTP 在分隔条右边，向左拖大、向右拖小；停靠左侧时相反。
        let boundary = if side_left {
            sftp.right().as_f32()
        } else {
            sftp.left().as_f32()
        };
        cx.simulate_mouse_down(
            point(px(boundary - 2.0), px(150.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        draw(cx);
        for step in 1..=5 {
            let x = boundary + distance * step as f32 / 5.0;
            cx.simulate_mouse_move(
                point(px(x), px(150.0)),
                Some(MouseButton::Left),
                Modifiers::default(),
            );
            draw(cx);
        }
        cx.simulate_mouse_up(
            point(px(boundary + distance), px(150.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        draw_until_settled(cx);
    }

    /// 回归：收起侧栏后拖动 SFTP 分隔条，面板必须稳定跟随鼠标，不能跳动。
    ///
    /// 根因是 0 号槽位收起后仍保留旧宽度，拖动时拿它做差值，第一次移动就会
    /// 把 SFTP 面板拽偏几百像素。
    #[test]
    fn drag_after_sidebar_collapsed_follows_pointer() {
        with_probe(
            true,
            false,
            true,
            false,
            |cx, probe, visible, _sftp_open| {
                assert!((width(cx, "sftp") - SFTP_WIDTH).abs() < 1.0);
                assert!(
                    (width(cx, "main") - (WINDOW_WIDTH - SIDEBAR_WIDTH - SFTP_WIDTH)).abs() < 1.0
                );

                toggle(probe, cx, || visible.set(false));
                draw_until_settled(cx);
                assert!((width(cx, "main") - (WINDOW_WIDTH - SFTP_WIDTH)).abs() < 1.0);

                drag_sftp_handle(cx, false, 100.0);
                let sftp_after_drag = width(cx, "sftp");
                assert!(
                    (sftp_after_drag - (SFTP_WIDTH - 100.0)).abs() < 2.0,
                    "收起侧栏后拖动应稳定跟随鼠标：期望约 {}，实际 {sftp_after_drag}",
                    SFTP_WIDTH - 100.0
                );

                toggle(probe, cx, || visible.set(true));
                draw_until_settled(cx);
                assert!((width(cx, "sidebar") - SIDEBAR_WIDTH).abs() < 1.0);
                let sftp_after_restore = width(cx, "sftp");
                assert!(
                    (sftp_after_restore - sftp_after_drag).abs() < 1.0,
                    "展开侧栏不应改变 SFTP 宽度：拖动后 {sftp_after_drag}，展开后 {sftp_after_restore}"
                );
                assert!(
                    (width(cx, "main") - (WINDOW_WIDTH - SIDEBAR_WIDTH - sftp_after_drag)).abs()
                        < 1.0
                );
            },
        );
    }

    /// 停靠左侧时展开侧栏会先挤窄紧邻的 SFTP 面板，恢复后必须保持原宽度。
    #[test]
    fn sidebar_restore_keeps_left_docked_sftp_width() {
        with_probe(true, true, true, false, |cx, probe, visible, _sftp_open| {
            assert!((width(cx, "sftp") - SFTP_WIDTH).abs() < 1.0);
            toggle(probe, cx, || visible.set(false));
            draw_until_settled(cx);
            toggle(probe, cx, || visible.set(true));
            draw_until_settled(cx);
            assert!((width(cx, "sidebar") - SIDEBAR_WIDTH).abs() < 1.0);
            let sftp = width(cx, "sftp");
            assert!(
                (sftp - SFTP_WIDTH).abs() < 1.0,
                "展开侧栏不应挤窄 SFTP：期望 {SFTP_WIDTH}，实际 {sftp}"
            );
            assert!((width(cx, "main") - (WINDOW_WIDTH - SIDEBAR_WIDTH - SFTP_WIDTH)).abs() < 1.0);
        });
    }

    /// 停靠左侧且 SFTP、转发同时打开：恢复侧栏不能改变任一固定面板宽度。
    #[test]
    fn sidebar_restore_keeps_fixed_panel_chain() {
        with_probe(true, true, true, true, |cx, probe, visible, _sftp_open| {
            let main_before = width(cx, "main");
            let forwards_before = width(cx, "forwards");
            toggle(probe, cx, || visible.set(false));
            draw_until_settled(cx);
            toggle(probe, cx, || visible.set(true));
            draw_until_settled(cx);
            assert!((width(cx, "sidebar") - SIDEBAR_WIDTH).abs() < 1.0);
            assert!(
                (width(cx, "forwards") - forwards_before).abs() < 1.0,
                "展开侧栏不应改变转发面板宽度：收起前 {forwards_before}，展开后 {}",
                width(cx, "forwards")
            );
            assert!(
                (width(cx, "main") - main_before).abs() < 1.0,
                "展开侧栏后主面板应回到原宽度：收起前 {main_before}，展开后 {}",
                width(cx, "main")
            );
        });
    }

    /// 启动时就收起侧栏：尺寸表必须先归零，拖动同样不能跳动。
    #[test]
    fn drag_stays_stable_when_sidebar_starts_hidden() {
        with_probe(
            false,
            false,
            true,
            false,
            |cx, _probe, _visible, _sftp_open| {
                assert!((width(cx, "main") - (WINDOW_WIDTH - SFTP_WIDTH)).abs() < 1.0);
                drag_sftp_handle(cx, false, 100.0);
                let sftp = width(cx, "sftp");
                assert!(
                    (sftp - (SFTP_WIDTH - 100.0)).abs() < 2.0,
                    "侧栏启动即收起时拖动应稳定：期望约 {}，实际 {sftp}",
                    SFTP_WIDTH - 100.0
                );
            },
        );
    }

    /// 侧栏收起期间再打开 SFTP：新面板加入会让尺寸表脱节，拖动仍需稳定。
    #[test]
    fn drag_stays_stable_after_sftp_reopens_while_hidden() {
        with_probe(
            false,
            false,
            false,
            false,
            |cx, probe, _visible, sftp_open| {
                toggle(probe, cx, || sftp_open.set(true));
                draw_until_settled(cx);
                assert!((width(cx, "sftp") - SFTP_WIDTH).abs() < 1.0);
                drag_sftp_handle(cx, false, 100.0);
                let sftp = width(cx, "sftp");
                assert!(
                    (sftp - (SFTP_WIDTH - 100.0)).abs() < 2.0,
                    "侧栏收起期间打开 SFTP 后拖动应稳定：期望约 {}，实际 {sftp}",
                    SFTP_WIDTH - 100.0
                );
            },
        );
    }
}
