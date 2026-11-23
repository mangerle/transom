//! RDP 界面组件单元测试。

use super::input::{map_mouse_button, map_pointer_position};
use gpui_kit::{Bounds, Point, px, size};
use transom_rdp::MouseBtn;

#[test]
fn pointer_position_mapping_scale_and_fit() {
    let bounds = Bounds {
        origin: Point::new(px(100.0), px(50.0)),
        size: size(px(800.0), px(600.0)),
    };

    // 视口内的点
    let pt = Point::new(px(500.0), px(350.0));
    let mapped = map_pointer_position(pt, bounds, 1920, 1080, true);
    assert!(mapped.is_some());
    let (x, y) = mapped.unwrap();
    assert!(x < 1920);
    assert!(y < 1080);

    // 视口外的点应被安全丢弃
    let out_pt = Point::new(px(50.0), px(30.0));
    assert!(map_pointer_position(out_pt, bounds, 1920, 1080, true).is_none());
}

#[test]
fn mouse_button_conversion() {
    assert!(matches!(
        map_mouse_button(gpui_kit::MouseButton::Left, true),
        Some(transom_rdp::RdpInput::MouseButton {
            button: MouseBtn::Left,
            pressed: true
        })
    ));
}
