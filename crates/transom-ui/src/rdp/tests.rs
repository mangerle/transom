//! RDP 界面组件单元测试。

use super::frame_cache::{TiledFrameCache, ViewportTransform};
use super::input::{map_mouse_button, map_pointer_position};
use gpui_kit::{Bounds, Point, px, size};
use transom_rdp::{FrameUpdate, MouseBtn, TilePatch};

#[test]
fn pointer_position_mapping_scale_and_fit() {
    let bounds = Bounds {
        origin: Point::new(px(100.0), px(50.0)),
        size: size(px(800.0), px(600.0)),
    };

    // 视口中心点应映射到远端桌面中心（1920x1080 居中在 800x600 内）
    let center = Point::new(px(500.0), px(350.0));
    let mapped =
        map_pointer_position(center, bounds, 1920, 1080, true, (0.0, 0.0)).expect("中心点应命中");
    assert!((i32::from(mapped.0) - 960).abs() <= 1);
    assert!((i32::from(mapped.1) - 540).abs() <= 1);

    // 落在上下黑边（Letterbox）内的点应返回 None
    let letterbox_top = Point::new(px(500.0), px(60.0));
    assert!(map_pointer_position(letterbox_top, bounds, 1920, 1080, true, (0.0, 0.0)).is_none());

    // 视口外的点应被安全丢弃
    let out_pt = Point::new(px(50.0), px(30.0));
    assert!(map_pointer_position(out_pt, bounds, 1920, 1080, true, (0.0, 0.0)).is_none());
}

#[test]
fn viewport_transform_roundtrip_consistency() {
    let bounds = Bounds {
        origin: Point::new(px(20.0), px(40.0)),
        size: size(px(1000.0), px(500.0)),
    };
    let transform =
        ViewportTransform::compute(bounds, 2000, 1000, true, (0.0, 0.0)).expect("应算出有效变换");
    assert!((transform.scale - 0.5).abs() < 1e-5);

    let rect = transform.map_desktop_rect(bounds.origin, 200.0, 100.0, 100.0, 50.0);
    let back = transform
        .unmap_point(bounds, rect.origin, 2000, 1000)
        .expect("正向映射点反算必须命中");
    assert_eq!(back, (200, 100));

    // 1:1 原始比例平移钳制与坐标反算一致性
    let pan_transform = ViewportTransform::compute(bounds, 1920, 1080, false, (-2000.0, -200.0))
        .expect("1:1 变换应有效");
    assert!((pan_transform.offset_x - (-920.0)).abs() < 1e-4);
    assert!((pan_transform.offset_y - (-200.0)).abs() < 1e-4);
    let panned_origin = pan_transform
        .unmap_point(bounds, bounds.origin, 1920, 1080)
        .expect("视口左上角应映射到平移后的远端像素");
    assert_eq!(panned_origin, (920, 200));
}

#[test]
fn frame_cache_applies_incremental_patches_in_place() {
    let mut cache = TiledFrameCache::default();
    let patch_a = TilePatch {
        col: 0,
        row: 0,
        width: 4,
        height: 4,
        rgba: vec![255; 4 * 4 * 4],
    };
    cache.apply_update(FrameUpdate {
        width: 512,
        height: 256,
        cols: 2,
        rows: 1,
        seq: 1,
        full_reset: true,
        patches: vec![patch_a],
    });
    cache.commit_pending();
    assert_eq!(cache.tiles.len(), 2);
    assert!(cache.tiles[0].is_some());
    assert!(cache.tiles[1].is_none());

    let first_ptr = std::sync::Arc::as_ptr(cache.tiles[0].as_ref().expect("首瓦片应存在"));

    // 仅更新第 2 个瓦片，第 1 个瓦片指针保持不变（零重配）
    let patch_b = TilePatch {
        col: 1,
        row: 0,
        width: 4,
        height: 4,
        rgba: vec![128; 4 * 4 * 4],
    };
    cache.apply_update(FrameUpdate {
        width: 512,
        height: 256,
        cols: 2,
        rows: 1,
        seq: 2,
        full_reset: false,
        patches: vec![patch_b],
    });
    cache.commit_pending();
    let after_ptr = std::sync::Arc::as_ptr(cache.tiles[0].as_ref().expect("首瓦片应存在"));
    assert_eq!(first_ptr, after_ptr);
    assert!(cache.tiles[1].is_some());
    assert_eq!(cache.stale_count(), 0, "新增空槽位不应产生淘汰纹理");

    // 覆盖已有瓦片 0，旧纹理必须进入待回收队列等待 window.drop_image 释放
    let patch_a2 = TilePatch {
        col: 0,
        row: 0,
        width: 4,
        height: 4,
        rgba: vec![64; 4 * 4 * 4],
    };
    cache.apply_update(FrameUpdate {
        width: 512,
        height: 256,
        cols: 2,
        rows: 1,
        seq: 3,
        full_reset: false,
        patches: vec![patch_a2],
    });
    cache.commit_pending();
    assert_eq!(cache.stale_count(), 1, "覆盖已有瓦片应登记 1 张待回收纹理");

    // clear 应将剩余 2 张现役瓦片一并登记回收
    cache.clear();
    assert_eq!(cache.stale_count(), 3);
}

#[test]
fn frame_cache_coalesces_background_updates_before_commit() {
    let mut cache = TiledFrameCache::default();
    let patch_1 = TilePatch {
        col: 0,
        row: 0,
        width: 4,
        height: 4,
        rgba: vec![10; 4 * 4 * 4],
    };
    let patch_2 = TilePatch {
        col: 0,
        row: 0,
        width: 4,
        height: 4,
        rgba: vec![20; 4 * 4 * 4],
    };

    cache.apply_update(FrameUpdate {
        width: 256,
        height: 256,
        cols: 1,
        rows: 1,
        seq: 1,
        full_reset: true,
        patches: vec![patch_1],
    });
    cache.apply_update(FrameUpdate {
        width: 256,
        height: 256,
        cols: 1,
        rows: 1,
        seq: 2,
        full_reset: false,
        patches: vec![patch_2],
    });

    // 未执行绘制提交前，不产生任何 GPU 纹理与淘汰队列积压
    assert!(cache.tiles.is_empty());
    assert_eq!(cache.stale_count(), 0);

    cache.commit_pending();
    assert_eq!(cache.tiles.len(), 1);
    assert!(cache.tiles[0].is_some());
    assert_eq!(cache.stale_count(), 0);
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
