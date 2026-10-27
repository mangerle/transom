//! Kitty 图形协议常量、事件与解码类型契约。

use gpui_kit::RenderImage;
use std::sync::Arc;

/// 单张图片的像素上限（约 64 MiB RGBA），防止恶意/误发的大图撑爆内存。
pub(crate) const MAX_IMAGE_PIXELS: u64 = 16 * 1024 * 1024;
/// APC 载荷上限（含 base64 膨胀），超过即丢弃该序列。
pub(crate) const MAX_APC_PAYLOAD: usize = 32 * 1024 * 1024;
/// 同时保留的图片数量上限。
pub(crate) const MAX_IMAGES: usize = 32;
/// 图像显存总量上限（按 RGBA 字节估算）。
pub(crate) const MAX_GRAPHICS_BYTES: usize = 64 * 1024 * 1024;

pub(crate) const ESCAPE: u8 = 0x1b;
pub(crate) const APC_INTRO: u8 = 0x5f;
pub(crate) const ST_TAIL: u8 = 0x5c;
pub(crate) const BEL: u8 = 0x07;

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
pub(crate) struct PendingImage {
    pub(crate) format: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) cols: Option<usize>,
    pub(crate) rows: Option<usize>,
    pub(crate) display: bool,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) data: Vec<u8>,
}

/// 已解码图片条目。
pub(crate) struct StoredImage {
    pub(crate) image: Arc<RenderImage>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bytes: usize,
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

pub(crate) struct PlacedImage {
    pub(crate) image_id: u32,
    pub(crate) absolute_line: i64,
    pub(crate) col: usize,
    pub(crate) cols: Option<usize>,
    pub(crate) rows: Option<usize>,
    pub(crate) x: i32,
    pub(crate) y: i32,
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
