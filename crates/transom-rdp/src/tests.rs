//! RDP 协议后端基础单元测试。

use crate::clipboard::file_stream::sanitize_destination_path;
use crate::clipboard::text::{decode_text_payload, encode_text_payload};
use crate::keyboard::KeyboardTranslator;
use crate::settings::SessionConfig;
use crate::tiles::{compute_dirty_mask, compute_grid_dimensions, compute_tile_bounds};
use ironrdp_pdu::geometry::InclusiveRectangle;
use transom_core::RdpConnection;

#[test]
fn tile_grid_dimensions_and_bounds() {
    let (cols, rows) = compute_grid_dimensions(1920, 1080);
    assert_eq!(cols, 8);
    assert_eq!(rows, 5);

    let (left, top, w, h) = compute_tile_bounds(1920, 1080, 0, 0);
    assert_eq!((left, top, w, h), (0, 0, 256, 256));

    // 边缘瓦片应正确裁剪
    let (left, top, w, h) = compute_tile_bounds(1920, 1080, 7, 4);
    assert_eq!((left, top, w, h), (7 * 256, 4 * 256, 128, 56));
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

    let fallback_ops = translator.release_all();
    assert_eq!(fallback_ops.len(), 4);
}

#[test]
fn path_traversal_protection_rejects_escape_without_creating_dirs() {
    let temp_dir = tempfile::tempdir().expect("临时目录");
    let safe_base = temp_dir.path().join("sandbox");
    let outside_dir = temp_dir.path().join("escaped_dir");

    for evil in [
        "../escaped_dir/evil.txt",
        "../../../etc/passwd",
        "/etc/passwd",
        r"C:\Windows\System32\cmd.exe",
        "sub/../../escaped_dir/file.txt",
        "",
        ".",
    ] {
        let result = sanitize_destination_path(&safe_base, evil);
        assert!(result.is_err(), "恶意路径必须被拒绝: {evil}");
    }

    assert!(
        !outside_dir.exists(),
        "拒绝路径穿越前严禁在沙箱外创建任何目录"
    );

    // 合法嵌套相对路径应正确解析到沙箱内
    let valid =
        sanitize_destination_path(&safe_base, r"docs\report.pdf").expect("合法相对路径应校验通过");
    let canonical_base = safe_base.canonicalize().expect("规范化沙箱根目录");
    assert!(valid.starts_with(&canonical_base));
    assert!(valid.ends_with("docs/report.pdf"));
}

#[test]
fn session_config_debug_redacts_password() {
    let conn = RdpConnection::new("10.0.0.1", "admin");
    let config = SessionConfig::new(&conn, "SuperSecretPassword123!".to_string(), (1280, 720));
    let debug_str = format!("{config:?}");
    assert!(!debug_str.contains("SuperSecretPassword123!"));
    assert!(debug_str.contains("[REDACTED]"));
}

#[test]
fn session_channel_backpressure_distinction() {
    use crate::session::channel::INPUT_CHANNEL_CAPACITY;
    assert_eq!(INPUT_CHANNEL_CAPACITY, 128);

    let (input_tx, _input_rx) = tokio::sync::mpsc::channel(1);
    let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
    let session = crate::RdpSession {
        input_tx,
        shutdown_tx,
    };
    assert!(
        session
            .try_send_input(crate::RdpInput::ReleaseAllKeys)
            .is_ok()
    );

    let full_res = session.try_send_input(crate::RdpInput::ReleaseAllKeys);
    assert!(matches!(full_res, Err(crate::RdpError::ChannelFull { .. })));
}
