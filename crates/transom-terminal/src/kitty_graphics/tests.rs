//! Kitty 图形协议编解码单元测试。

use super::parser::KittyParser;
use super::store::GraphicsStore;
use super::types::{
    APC_INTRO, ESCAPE, KittyEvent, MAX_APC_PAYLOAD, MAX_GRAPHICS_BYTES, MAX_IMAGES, PlaceRequest,
    ST_TAIL,
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;

fn encode_base64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

fn apc(payload: &str) -> Vec<u8> {
    let mut wire = vec![ESCAPE, APC_INTRO, b'G'];
    wire.extend_from_slice(payload.as_bytes());
    wire.extend_from_slice(&[ESCAPE, ST_TAIL]);
    wire
}

fn rgb_payload() -> String {
    // 2x2 纯红 RGB 图像。
    let pixels = [255u8, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0];
    format!("a=T,f=24,s=2,v=2,c=2,r=2,i=7;{}", encode_base64(&pixels))
}

#[test]
fn transmits_and_places_rgb_image() {
    let mut parser = KittyParser::default();
    let events = parser.feed_with_offsets(&apc(&rgb_payload()));
    assert_eq!(events.len(), 1);
    match &events[0].1 {
        KittyEvent::Transmit {
            id,
            width,
            height,
            cols,
            rows,
            display,
            ..
        } => {
            assert_eq!(*id, 7);
            assert_eq!((*width, *height), (2, 2));
            assert_eq!((*cols, *rows), (Some(2), Some(2)));
            assert!(*display);
        }
        _ => panic!("应解码为图像传输事件"),
    }
}

#[test]
fn assembles_chunked_transfer() {
    let pixels = [0u8, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0];
    let encoded = encode_base64(&pixels);
    let (first, second) = encoded.split_at(encoded.len() / 2);
    let mut parser = KittyParser::default();
    let head = format!("a=t,f=24,s=2,v=2,i=9,m=1;{first}");
    assert!(parser.feed_with_offsets(&apc(&head)).is_empty());
    let tail = format!("m=0;{second}");
    let events = parser.feed_with_offsets(&apc(&tail));
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0].1,
        KittyEvent::Transmit {
            id: 9,
            display: false,
            ..
        }
    ));
}

#[test]
fn delete_events_clear_images() {
    let mut parser = KittyParser::default();
    let events = parser.feed_with_offsets(&apc("a=d,d=i,i=7;"));
    assert!(matches!(events[0].1, KittyEvent::Delete { id: Some(7) }));
    let events = parser.feed_with_offsets(&apc("a=d,d=a;"));
    assert!(matches!(events[0].1, KittyEvent::Delete { id: None }));
}

#[test]
fn ignores_non_kitty_apc_and_oversize_payload() {
    let mut parser = KittyParser::default();
    assert!(parser.feed_with_offsets(&apc("other;data")).is_empty());
    let huge = format!("a=t,f=24,s=1,v=1;{}", "A".repeat(MAX_APC_PAYLOAD + 16));
    assert!(parser.feed_with_offsets(&apc(&huge)).is_empty());
}

#[test]
fn store_enforces_memory_and_image_limits() {
    let mut store = GraphicsStore::default();
    let mut parser = KittyParser::default();
    for id in 1..=(MAX_IMAGES as u32 + 2) {
        let pixels = [1u8, 2, 3, 4];
        let payload = format!("a=T,f=32,s=1,v=1,i={id};{}", encode_base64(&pixels));
        let events = parser.feed_with_offsets(&apc(&payload));
        if let KittyEvent::Transmit {
            id,
            image,
            width,
            height,
            cols,
            rows,
            display,
            x,
            y,
        } = &events[0].1
        {
            store.store(*id, image.clone(), *width, *height);
            if *display {
                store.place(PlaceRequest {
                    image_id: *id,
                    absolute_line: 0,
                    col: 0,
                    cols: *cols,
                    rows: *rows,
                    x: *x,
                    y: *y,
                });
            }
        }
    }
    assert!(store.images.len() <= MAX_IMAGES);
    assert!(store.total_bytes <= MAX_GRAPHICS_BYTES);
}
