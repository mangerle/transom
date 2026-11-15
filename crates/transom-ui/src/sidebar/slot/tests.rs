use super::{SidebarSlotLayout, SidebarSlotSync, sync_sidebar_slot};
use gpui_kit::base::h_resizable;
use gpui_kit::component::Root;
use gpui_kit::component::resizable::{ResizableState, resizable_panel};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Bounds, Context, Entity, InteractiveElement, IntoElement, Modifiers, MouseButton,
    ParentElement, Pixels, Render, Size, Styled, TestAppContext, VisualTestContext, Window, div,
    init, point, px,
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
    scenario: impl FnOnce(&mut VisualTestContext, &Entity<SlotProbe>, &Rc<Cell<bool>>, &Rc<Cell<bool>>),
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
            assert!((width(cx, "main") - (WINDOW_WIDTH - SIDEBAR_WIDTH - SFTP_WIDTH)).abs() < 1.0);

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
                (width(cx, "main") - (WINDOW_WIDTH - SIDEBAR_WIDTH - sftp_after_drag)).abs() < 1.0
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
