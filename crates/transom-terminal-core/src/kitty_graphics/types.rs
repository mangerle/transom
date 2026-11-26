//! Kitty 图形协议常量、事件与解码类型契约。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// 单张图片的像素上限（约 64 MiB RGBA），防止恶意/误发的大图撑爆内存。
pub const MAX_IMAGE_PIXELS: u64 = 16 * 1024 * 1024;
/// APC 载荷上限（含 base64 膨胀），超过即丢弃该序列。
pub const MAX_APC_PAYLOAD: usize = 32 * 1024 * 1024;
/// 同时保留的图片数量上限。
pub const MAX_IMAGES: usize = 32;
/// 图像显存总量上限（按 RGBA 字节估算）。
pub const MAX_GRAPHICS_BYTES: usize = 64 * 1024 * 1024;

pub const APC_INTRO: u8 = 0x5f;

/// 解码后的终端图像像素。
///
/// 设计原理（Why）：核心只保存 BGRA 字节与尺寸，不持有 GPUI 纹理；视图层在
/// 真正绘制时按图像 id 缓存转换结果，既保持核心无 UI 依赖，也避免每帧重复解码。
pub struct TerminalImage {
    id: u64,
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

impl TerminalImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Arc<Self> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Arc::new(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            width,
            height,
            pixels: Arc::from(pixels),
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// 一次图像传输/放置的解码结果。
#[derive(Clone)]
pub enum KittyEvent {
    /// 新图片解码完成；`display` 表示是否同时按当前光标位置放置。
    Transmit {
        id: u32,
        image: Arc<TerminalImage>,
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
pub struct PendingImage {
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub cols: Option<usize>,
    pub rows: Option<usize>,
    pub display: bool,
    pub x: i32,
    pub y: i32,
    pub data: Vec<u8>,
}

/// 已解码图片条目。
pub struct StoredImage {
    pub image: Arc<TerminalImage>,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
}

/// 一次放置（同图可多处放置）。
#[derive(Clone)]
pub struct GraphicsPlacement {
    /// 网格行；渲染时由行映射换算为可见行。
    pub line: alacritty_terminal::index::Line,
    pub col: usize,
    pub image: Arc<TerminalImage>,
    pub width: u32,
    pub height: u32,
    pub cols: Option<usize>,
    pub rows: Option<usize>,
    pub x: i32,
    pub y: i32,
}

pub struct PlacedImage {
    pub image_id: u32,
    pub absolute_line: i64,
    pub col: usize,
    pub cols: Option<usize>,
    pub rows: Option<usize>,
    pub x: i32,
    pub y: i32,
}

/// 一次放置请求：图像、网格锚点与可选的像素偏移。
pub struct PlaceRequest {
    pub image_id: u32,
    pub absolute_line: i64,
    pub col: usize,
    pub cols: Option<usize>,
    pub rows: Option<usize>,
    pub x: i32,
    pub y: i32,
}
