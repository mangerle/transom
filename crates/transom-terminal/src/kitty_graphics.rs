//! Kitty 图形协议（APC `ESC _ G ... ESC \`）解码与图像存储。
//!
//! 设计原理（Why）：
//! - Kitty 协议是终端内显示图片的事实标准（yazi、ranger、matplotlib、chafa
//!   等都在用），它通过 APC 传递 base64 载荷，不占用字符网格；
//! - 解析与像素解码收敛在终端 crate：内核只需要在屏幕上"留出矩形"，渲染层
//!   用 GPUI 纹理一次性绘制，`vim`/`tmux` 等文本程序完全不受影响；
//! - 图像以 `Arc<RenderImage>` 持有，同一图片多次放置共享一份显存，滚动回收
//!   时按引用计数自动释放。
//!
//! 代价与局限：
//! - 只支持直接传输（`t=d`）与 PNG/RGB/RGBA 三种格式，不支持文件/临时文件
//!   传输与动画帧（`a=f` 只取首帧之外的帧会被忽略）；
//! - 图像锚定绝对行号，滚动缓冲写满淘汰旧行后不再精确，引擎在饱和时整体清空。

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use gpui_kit::RenderImage;
use std::collections::HashMap;
use std::sync::Arc;

/// 单张图片的像素上限（约 64 MiB RGBA），防止恶意/误发的大图撑爆内存。
pub(crate) const MAX_IMAGE_PIXELS: u64 = 16 * 1024 * 1024;
/// APC 载荷上限（含 base64 膨胀），超过即丢弃该序列。
pub(crate) const MAX_APC_PAYLOAD: usize = 32 * 1024 * 1024;
/// 同时保留的图片数量上限。
pub(crate) const MAX_IMAGES: usize = 32;
/// 图像显存总量上限（按 RGBA 字节估算）。
pub(crate) const MAX_GRAPHICS_BYTES: usize = 64 * 1024 * 1024;

const ESCAPE: u8 = 0x1b;
const APC_INTRO: u8 = 0x5f;
const ST_TAIL: u8 = 0x5c;
const BEL: u8 = 0x07;

/// 一次图像传输/放置的解码结果。
#[derive(Clone)]
pub(crate) enum KittyEvent {
    /// 新图片解码完成；`display` 表示是否同时按当前光标位置放置。
    Transmit {
        id: u32,
        image: Arc<RenderImage>,
        width: u32,
        height: u32,
        cols: Option<usize>,
        rows: Option<usize>,
        display: bool,
        x: i32,
        y: i32,
    },
    /// 把已存在的图片放到当前光标位置。
    Place {
        id: u32,
        cols: Option<usize>,
        rows: Option<usize>,
        x: i32,
        y: i32,
    },
    /// 删除图片；`None` 表示清空全部。
    Delete { id: Option<u32> },
}

/// 分块传输中的图片缓冲。
struct PendingImage {
    format: u32,
    width: u32,
    height: u32,
    cols: Option<usize>,
    rows: Option<usize>,
    display: bool,
    x: i32,
    y: i32,
    data: Vec<u8>,
}

/// Kitty APC 增量扫描器。
#[derive(Default)]
pub(crate) struct KittyParser {
    awaiting_intro: bool,
    in_payload: bool,
    awaiting_st: bool,
    overflow: bool,
    payload: Vec<u8>,
    pending: HashMap<u32, PendingImage>,
    next_id: u32,
}

impl KittyParser {
    /// 喂入一段字节流，返回带字节偏移的事件。不消费输入。
    pub(crate) fn feed_with_offsets(&mut self, bytes: &[u8]) -> Vec<(usize, KittyEvent)> {
        let mut events = Vec::new();
        for (offset, &byte) in bytes.iter().enumerate() {
            self.push(byte, offset, &mut events);
        }
        events
    }

    fn push(&mut self, byte: u8, offset: usize, events: &mut Vec<(usize, KittyEvent)>) {
        if !self.in_payload {
            if self.awaiting_intro {
                self.awaiting_intro = false;
                if byte == APC_INTRO {
                    self.in_payload = true;
                    self.payload.clear();
                    self.overflow = false;
                } else if byte == ESCAPE {
                    self.awaiting_intro = true;
                }
                return;
            }
            if byte == ESCAPE {
                self.awaiting_intro = true;
            }
            return;
        }
        if self.awaiting_st {
            self.awaiting_st = false;
            if byte == ST_TAIL || byte == BEL {
                self.close(offset, events);
            } else {
                // ESC 后不是 ST：本身属于载荷内容。
                self.collect(ESCAPE);
                self.collect(byte);
            }
            return;
        }
        match byte {
            BEL => self.close(offset, events),
            ESCAPE => self.awaiting_st = true,
            other => self.collect(other),
        }
    }

    fn collect(&mut self, byte: u8) {
        if self.overflow {
            return;
        }
        self.payload.push(byte);
        if self.payload.len() > MAX_APC_PAYLOAD {
            self.payload.clear();
            self.overflow = true;
        }
    }

    fn close(&mut self, offset: usize, events: &mut Vec<(usize, KittyEvent)>) {
        let overflowed = self.overflow;
        self.in_payload = false;
        self.awaiting_st = false;
        self.overflow = false;
        if overflowed {
            self.payload.clear();
            return;
        }
        if let Some(event) = self.parse() {
            events.push((offset, event));
        }
        self.payload.clear();
    }

    fn parse(&mut self) -> Option<KittyEvent> {
        let payload = std::str::from_utf8(&self.payload).ok()?;
        let rest = payload.strip_prefix('G')?;
        let (controls, data) = rest.split_once(';').unwrap_or((rest, ""));
        let mut action = "t".to_string();
        let mut format = 0u32;
        let mut width = 0u32;
        let mut height = 0u32;
        let mut cols = None;
        let mut rows = None;
        let mut id = 0u32;
        let mut more = false;
        let mut delete = None;
        let mut x = 0i32;
        let mut y = 0i32;
        for field in controls.split(',') {
            let Some((key, value)) = field.split_once('=') else {
                continue;
            };
            match key {
                "a" => action = value.to_string(),
                "f" => format = value.parse().unwrap_or(format),
                "s" => width = value.parse().unwrap_or(0),
                "v" => height = value.parse().unwrap_or(0),
                "c" => cols = value.parse::<usize>().ok().filter(|value| *value > 0),
                "r" => rows = value.parse::<usize>().ok().filter(|value| *value > 0),
                "i" => id = value.parse().unwrap_or(0),
                "m" => more = value == "1",
                "d" => delete = Some(value.to_string()),
                "x" => x = value.parse().unwrap_or(0),
                "y" => y = value.parse().unwrap_or(0),
                _ => {}
            }
        }
        match action.as_str() {
            "d" => {
                let id = match delete.as_deref() {
                    Some("i") if id > 0 => Some(id),
                    Some("a") | None => None,
                    _ => Some(id).filter(|value| *value > 0),
                };
                Some(KittyEvent::Delete { id })
            }
            "p" => Some(KittyEvent::Place {
                id,
                cols,
                rows,
                x,
                y,
            }),
            "t" | "T" => {
                let id = if id == 0 {
                    // 续传块常省略 `i`：只有一个在途图片时直接接上它。
                    if self.pending.len() == 1 {
                        *self.pending.keys().next().unwrap_or(&0)
                    } else {
                        self.next_id = self.next_id.wrapping_add(1);
                        self.next_id
                    }
                } else {
                    id
                };
                let display = action == "T";
                if data.is_empty() && more {
                    // 首块只带控制字段，载荷要在后续块里累积。
                }
                let entry = self.pending.entry(id).or_insert_with(|| PendingImage {
                    format,
                    width,
                    height,
                    cols,
                    rows,
                    display,
                    x,
                    y,
                    data: Vec::new(),
                });
                if format > 0 {
                    entry.format = format;
                }
                if width > 0 {
                    entry.width = width;
                }
                if height > 0 {
                    entry.height = height;
                }
                entry.cols = cols.or(entry.cols);
                entry.rows = rows.or(entry.rows);
                entry.display |= display;
                entry.x = x;
                entry.y = y;
                if !data.is_empty() {
                    let Ok(decoded) = STANDARD.decode(data) else {
                        self.pending.remove(&id);
                        return None;
                    };
                    entry.data.extend_from_slice(&decoded);
                    if entry.data.len() > MAX_APC_PAYLOAD {
                        self.pending.remove(&id);
                        return None;
                    }
                }
                if more {
                    return None;
                }
                let pending = self.pending.remove(&id)?;
                let (image, actual_width, actual_height) =
                    decode_image(pending.format, pending.width, pending.height, &pending.data)?;
                Some(KittyEvent::Transmit {
                    id,
                    image,
                    width: actual_width,
                    height: actual_height,
                    cols: pending.cols,
                    rows: pending.rows,
                    display: pending.display,
                    x: pending.x,
                    y: pending.y,
                })
            }
            _ => None,
        }
    }
}

/// 解码图片载荷为 GPUI 纹理（BGRA）。
fn decode_image(
    format: u32,
    width: u32,
    height: u32,
    data: &[u8],
) -> Option<(Arc<RenderImage>, u32, u32)> {
    let rgba = match format {
        100 => {
            let pixels = u64::from(width).saturating_mul(u64::from(height));
            if pixels > MAX_IMAGE_PIXELS {
                log::warn!("Kitty 图像超过像素上限，已丢弃: {width}x{height}");
                return None;
            }
            let reader = image::ImageReader::new(std::io::Cursor::new(data))
                .with_guessed_format()
                .ok()?;
            let (decoded_width, decoded_height) = reader.into_dimensions().ok()?;
            let pixels = u64::from(decoded_width).saturating_mul(u64::from(decoded_height));
            if pixels > MAX_IMAGE_PIXELS {
                log::warn!("Kitty PNG 超过像素上限，已丢弃: {decoded_width}x{decoded_height}");
                return None;
            }
            let image = image::load_from_memory(data).ok()?.into_rgba8();
            (image, decoded_width, decoded_height)
        }
        24 | 32 => {
            let pixels = u64::from(width).saturating_mul(u64::from(height));
            if width == 0 || height == 0 || pixels > MAX_IMAGE_PIXELS {
                return None;
            }
            let channels = if format == 24 { 3 } else { 4 };
            if data.len() != (pixels as usize).saturating_mul(channels) {
                return None;
            }
            let mut rgba = image::RgbaImage::new(width, height);
            for (index, pixel) in rgba.pixels_mut().enumerate() {
                let offset = index * channels;
                let (r, g, b, a) = if channels == 3 {
                    (data[offset], data[offset + 1], data[offset + 2], 255)
                } else {
                    (
                        data[offset],
                        data[offset + 1],
                        data[offset + 2],
                        data[offset + 3],
                    )
                };
                *pixel = image::Rgba([r, g, b, a]);
            }
            (rgba, width, height)
        }
        other => {
            log::debug!("忽略不支持的 Kitty 图像格式: {other}");
            return None;
        }
    };
    let (mut rgba, width, height) = rgba;
    // GPUI 纹理约定 BGRA。
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some((
        Arc::new(RenderImage::new([image::Frame::new(rgba)])),
        width,
        height,
    ))
}

/// 已解码图片条目。
struct StoredImage {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
    bytes: usize,
}

/// 一次放置（同图可多处放置）。
#[derive(Clone)]
pub(crate) struct GraphicsPlacement {
    /// 网格行；渲染时由行映射换算为可见行。
    pub(crate) line: alacritty_terminal::index::Line,
    pub(crate) col: usize,
    pub(crate) image: Arc<RenderImage>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) cols: Option<usize>,
    pub(crate) rows: Option<usize>,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

struct PlacedImage {
    image_id: u32,
    absolute_line: i64,
    col: usize,
    cols: Option<usize>,
    rows: Option<usize>,
    x: i32,
    y: i32,
}

/// 一次放置请求：图像、网格锚点与可选的像素偏移。
pub(crate) struct PlaceRequest {
    pub(crate) image_id: u32,
    pub(crate) absolute_line: i64,
    pub(crate) col: usize,
    pub(crate) cols: Option<usize>,
    pub(crate) rows: Option<usize>,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

/// 图像与放置的生命周期仓库。
#[derive(Default)]
pub(crate) struct GraphicsStore {
    images: HashMap<u32, StoredImage>,
    placements: Vec<PlacedImage>,
    total_bytes: usize,
}

impl GraphicsStore {
    pub(crate) fn store(&mut self, id: u32, image: Arc<RenderImage>, width: u32, height: u32) {
        let bytes = (u64::from(width) * u64::from(height) * 4) as usize;
        self.images.insert(
            id,
            StoredImage {
                image,
                width,
                height,
                bytes,
            },
        );
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.enforce_limits();
    }

    pub(crate) fn place(&mut self, request: PlaceRequest) {
        if !self.images.contains_key(&request.image_id) {
            return;
        }
        self.placements.push(PlacedImage {
            image_id: request.image_id,
            absolute_line: request.absolute_line,
            col: request.col,
            cols: request.cols,
            rows: request.rows,
            x: request.x,
            y: request.y,
        });
    }

    pub(crate) fn delete(&mut self, id: Option<u32>) {
        match id {
            Some(id) => {
                self.placements.retain(|placement| placement.image_id != id);
                if let Some(image) = self.images.remove(&id) {
                    self.total_bytes = self.total_bytes.saturating_sub(image.bytes);
                }
            }
            None => self.clear(),
        }
    }

    /// 回收滚出缓冲区的图像：行号已经不在 `[0, history + screen)` 内。
    pub(crate) fn retain_in_buffer(&mut self, history_size: usize, screen_lines: usize) {
        let limit = (history_size + screen_lines) as i64;
        self.placements
            .retain(|placement| (0..limit).contains(&placement.absolute_line));
        let live: std::collections::HashSet<u32> =
            self.placements.iter().map(|p| p.image_id).collect();
        let dead: Vec<u32> = self
            .images
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect();
        for id in dead {
            if let Some(image) = self.images.remove(&id) {
                self.total_bytes = self.total_bytes.saturating_sub(image.bytes);
            }
        }
    }

    pub(crate) fn placements(&self, history_size: usize) -> Vec<GraphicsPlacement> {
        let history = history_size as i64;
        let mut placements = Vec::with_capacity(self.placements.len());
        for placement in &self.placements {
            let Some(image) = self.images.get(&placement.image_id) else {
                continue;
            };
            placements.push(GraphicsPlacement {
                line: alacritty_terminal::index::Line((placement.absolute_line - history) as i32),
                col: placement.col,
                image: image.image.clone(),
                width: image.width,
                height: image.height,
                cols: placement.cols,
                rows: placement.rows,
                x: placement.x,
                y: placement.y,
            });
        }
        placements
    }

    pub(crate) fn clear(&mut self) {
        self.images.clear();
        self.placements.clear();
        self.total_bytes = 0;
    }

    fn enforce_limits(&mut self) {
        while self.images.len() > MAX_IMAGES || self.total_bytes > MAX_GRAPHICS_BYTES {
            let Some(oldest) = self.images.keys().min().copied() else {
                break;
            };
            self.delete(Some(oldest));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 分块传输：第一块 m=1 不产出事件，末块补齐后只产出一次。
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

    /// 存储仓库在超量时按最旧 id 淘汰，放置随图片一起消失。
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
}
