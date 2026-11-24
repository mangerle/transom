//! RDP 协议后端基础单元测试。

use crate::clipboard::file_stream::sanitize_destination_path;
use crate::clipboard::text::{decode_text_payload, encode_text_payload};
use crate::keyboard::KeyboardTranslator;
use crate::tiles::{compute_dirty_mask, compute_grid_dimensions, compute_tile_bounds};
use ironrdp_pdu::geometry::InclusiveRectangle;

#[test]
fn tile_grid_dimensions_and_bounds() {
    let (cols, rows) = compute_grid_dimensions(1920, 1080);
    assert_eq!(cols, 8); // 1920 / 256 = 7.5 -> 8
    assert_eq!(rows, 5); // 1080 / 256 = 4.21 -> 5

    let (left, top, w, h) = compute_tile_bounds(1920, 1080, 0, 0);
    assert_eq!((left, top, w, h), (0, 0, 256, 256));

    // 最后一个边缘瓦片应正确裁剪
    let (left, top, w, h) = compute_tile_bounds(1920, 1080, 7, 4);
    assert_eq!(left, 7 * 256);
    assert_eq!(top, 4 * 256);
    assert_eq!(w, 1920 - 7 * 256); // 128
    assert_eq!(h, 1080 - 4 * 256); // 56
}

#[test]
fn dirty_mask_calculation() {
    let dirty = vec![InclusiveRectangle {
        left: 10,
        top: 10,
        right: 100,
        bottom: 100,
    }];
    let mask = compute_dirty_mask(1920, 1080, &dirty);
    assert_eq!(mask.len(), 40);
    assert!(mask[0], "左上角第一个瓦片必须被标记为脏");
    assert!(!mask[1], "第二个未被覆盖的瓦片必须是干净的");
}

#[test]
fn clipboard_utf16_round_trip() {
    let raw = "测试 RDP 远程桌面 UTF-16 剪贴板！";
    let encoded = encode_text_payload(raw);
    let decoded = decode_text_payload(&encoded).expect("必须成功解码");
    assert_eq!(decoded, raw);
}

#[test]
fn keyboard_scancode_translation() {
    let mut translator = KeyboardTranslator::default();
    let ops = translator.handle_key_down("enter");
    assert_eq!(ops.len(), 1);
    let release_ops = translator.release_all();
    assert_eq!(release_ops.len(), 1);

    // 未按下任何键时，调用 release_all 释放 4 个修饰键以兜底
    let fallback_ops = translator.release_all();
    assert_eq!(fallback_ops.len(), 4);
}

#[test]
fn path_traversal_protection() {
    let temp_dir = tempfile::tempdir().expect("临时目录");
    let safe_base = temp_dir.path();

    // 尝试传入带 ../ 的逃逸路径
    let evil_path = "../../../etc/passwd";
    let result = sanitize_destination_path(safe_base, evil_path);
    // 即使路径被规范化拼接，最终落盘也必须在 safe_base 之内，绝不允许逃逸
    if let Ok(path) = result {
        assert!(path.starts_with(safe_base));
    }
}

#[test]
fn session_channel_backpressure_distinction() {
    use crate::session::channel::INPUT_CHANNEL_CAPACITY;
    assert_eq!(INPUT_CHANNEL_CAPACITY, 128);

    let (input_tx, _input_rx) = tokio::sync::mpsc::channel(1);
    let session = crate::RdpSession { input_tx };
    assert!(
        session
            .try_send_input(crate::RdpInput::ReleaseAllKeys)
            .is_ok()
    );

    // 缓冲区打满后，try_send_input 必须返回 ChannelFull 而非 ChannelClosed
    let full_res = session.try_send_input(crate::RdpInput::ReleaseAllKeys);
    assert!(matches!(full_res, Err(crate::RdpError::ChannelFull { .. })));
}
