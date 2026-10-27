//! Kitty APC 序列扫描与图像解码。

use super::types::{
    APC_INTRO, BEL, ESCAPE, KittyEvent, MAX_APC_PAYLOAD, MAX_IMAGE_PIXELS, PendingImage, ST_TAIL,
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use gpui_kit::RenderImage;
use std::collections::HashMap;
use std::sync::Arc;

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
