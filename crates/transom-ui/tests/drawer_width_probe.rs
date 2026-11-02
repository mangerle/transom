//! 监控抽屉首次打开那一帧的宽度探针。
//!
//! 为什么需要实测：抽屉宽度同时来自“记住的宽度”和“分栏尺寸表”，而尺寸表按
//! 面板索引记录、抽屉关闭时没有它的槽位。这里用真实无头窗口复现“三面板加入
//! 第四面板”的一帧，直接读每帧的抽屉边界，确认打开首帧不会借用相邻面板宽度。
//!
//! 为什么不用 `#[gpui::test]`：本仓库经 gpui-kit 门面消费 gpui，门面里
//! `pub use crate as gpui`，测试宏展开出的 `gpui::` 路径会回指门面自身。

use gpui_kit::base::h_resizable;
use gpui_kit::component::Root;
use gpui_kit::component::resizable::{ResizableState, resizable_panel};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Pixels, Render,
    Size, Styled, TestAppContext, VisualTestContext, Window, div, init, px,
};
use std::cell::Cell;
use std::rc::Rc;

/// 抽屉记住的宽度。
const REMEMBERED_WIDTH: f32 = 420.0;
/// 抽屉之前的面板数（侧栏 + 主面板）。
const PANELS_BEFORE_DRAWER: usize = 2;

/// 探针宿主：侧栏 + 主面板，可动态加入监控抽屉。
struct Probe {
    state: Entity<ResizableState>,
    open: Rc<Cell<bool>>,
    /// true 表示按“尺寸表确实包含抽屉槽位”的规则取宽度。
    guarded: bool,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.clone();
        let sizes: Vec<Pixels> = state.read(cx).sizes().to_vec();
        let last = sizes.last().copied().map(|width| width.as_f32());
        let adopted = if self.guarded {
            drawer_slot_measured(sizes.len(), PANELS_BEFORE_DRAWER)
                .then_some(last)
                .flatten()
        } else {
            last
        };
        let drawer_width = adopted.unwrap_or(REMEMBERED_WIDTH);
        let open = self.open.get();
        let state = self.state.clone();
        h_resizable("drawer-probe")
            .with_state(&state)
            .child(
                resizable_panel()
                    .size(px(200.0))
                    .size_range(px(120.0)..px(400.0))
                    .fixed()
                    .child(div().size_full().debug_selector(|| "sidebar".to_string())),
            )
            .child(resizable_panel().child(div().size_full().debug_selector(|| "main".to_string())))
            .when(open, |group| {
                group.child(
                    resizable_panel()
                        .size(px(drawer_width))
                        .size_range(px(360.0)..px(2000.0))
                        .fixed()
                        .child(div().size_full().debug_selector(|| "drawer".to_string())),
                )
            })
    }
}

/// 尺寸表的最后一项是否就是抽屉槽位。
fn drawer_slot_measured(sizes_len: usize, panels_before_drawer: usize) -> bool {
    sizes_len == panels_before_drawer + 1
}

/// 画一帧并返回抽屉的实测宽度。
fn draw_frame(cx: &mut VisualTestContext) -> Option<f32> {
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.debug_bounds("drawer")
        .map(|bounds| bounds.size.width.as_f32())
}

/// 在 1200x800 的无头窗口里跑一次“三面板 -> 加入抽屉”，返回打开后前两帧的宽度。
fn probe_drawer_frames(guarded: bool) -> (f32, f32) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    let open = Rc::new(Cell::new(false));
    let open_for_view = Rc::clone(&open);
    let (_, cx) = app.add_window_view(move |window, cx| {
        let state = cx.new(|_| ResizableState::default());
        let probe = cx.new(|_| Probe {
            state,
            open: open_for_view,
            guarded,
        });
        Root::new(probe, window, cx)
    });
    cx.simulate_resize(Size {
        width: px(1200.),
        height: px(800.),
    });
    // 先让三面板布局落地，尺寸表稳定为三项
    let _ = draw_frame(cx);
    open.set(true);
    let first = draw_frame(cx).expect("抽屉应在第二帧绘制");
    let second = draw_frame(cx).expect("抽屉应在第三帧绘制");
    (first, second)
}

/// 打开首帧就必须使用记住的宽度，不能拿相邻面板的实测宽度顶替。
#[test]
fn drawer_first_frame_keeps_remembered_width() {
    let (first, second) = probe_drawer_frames(true);
    assert!(
        (first - REMEMBERED_WIDTH).abs() < 1.0,
        "打开首帧应为记住的宽度 {REMEMBERED_WIDTH}，实测 {first}"
    );
    assert!(
        (second - REMEMBERED_WIDTH).abs() < 1.0,
        "第二帧应保持同一宽度 {REMEMBERED_WIDTH}，实测 {second}"
    );
}

/// 对照：没有槽位判定时，首帧把主面板（1200 - 200 = 1000）的宽度当成抽屉宽度。
#[test]
fn drawer_without_slot_guard_borrows_neighbour_width() {
    let (first, second) = probe_drawer_frames(false);
    assert!(
        (first - 1000.0).abs() < 1.0,
        "缺少槽位判定时首帧会借用主面板宽度 1000，实测 {first}"
    );
    assert!(
        (second - 1000.0).abs() < 1.0,
        "错误宽度还会被当成抽屉自己的宽度写回，第二帧仍应为 1000，实测 {second}"
    );
}
