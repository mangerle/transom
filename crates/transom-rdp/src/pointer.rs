//! 远端光标位图解码与 GPUI 系统光标适配。
//!
//! 设计原理（Why）：RDP 协议通过单独的 Pointer PDU 发送光标形状与热点信息，
//! 不应把光标强行画进桌面位图画布；由客户端将远端光标位图解码为平台原生的
//! 系统光标，能够确保本地鼠标移动完全跟手、零网络延迟，并正确呈现反色与形状。

use gpui_kit::RenderImage;
use image::{Frame, RgbaImage};
use std::sync::Arc;

/// 远端光标形状更新事件。
#[derive(Clone, Debug)]
pub enum CursorUpdate {
    /// 隐藏当前光标。
    Hidden,
    /// 恢复默认箭头光标。
    Default,
    /// 远端下发的新位图光标。
    Custom {
        /// 光标像素宽度。
        width: u16,
        /// 光标像素高度。
        height: u16,
        /// 光标热点水平偏移（像素）。
        hot_x: u16,
        /// 光标热点垂直偏移（像素）。
        hot_y: u16,
        /// 光标位图图像。
        image: Arc<RenderImage>,
    },
}

/// 将 32 位 RGBA 光标像素缓冲封装为 GPUI `RenderImage`。
pub fn create_custom_cursor(
    width: u16,
    height: u16,
    hot_x: u16,
    hot_y: u16,
    rgba_data: &[u8],
) -> Option<CursorUpdate> {
    if width == 0 || height == 0 || rgba_data.len() != usize::from(width) * usize::from(height) * 4
    {
        return None;
    }

    let rgba_image = RgbaImage::from_raw(u32::from(width), u32::from(height), rgba_data.to_vec())?;
    let render_image = RenderImage::new(vec![Frame::new(rgba_image)]);

    Some(CursorUpdate::Custom {
        width,
        height,
        hot_x,
        hot_y,
        image: Arc::new(render_image),
    })
}

/// 解析单色 1bpp 掩码光标（XOR / AND 掩码）。
pub fn decode_monochrome_cursor(
    width: u16,
    height: u16,
    hot_x: u16,
    hot_y: u16,
    and_mask: &[u8],
    xor_mask: &[u8],
) -> Option<CursorUpdate> {
    let width_u32 = u32::from(width);
    let height_u32 = u32::from(height);
    let stride = (width_u32.div_ceil(8)) as usize;

    if and_mask.len() < stride * height as usize || xor_mask.len() < stride * height as usize {
        return None;
    }

    let mut rgba_image = RgbaImage::new(width_u32, height_u32);

    for y in 0..height_u32 {
        let row_offset = y as usize * stride;
        for x in 0..width_u32 {
            let byte_idx = row_offset + (x as usize / 8);
            let bit_mask = 0x80 >> (x % 8);

            let and_bit = (and_mask[byte_idx] & bit_mask) != 0;
            let xor_bit = (xor_mask[byte_idx] & bit_mask) != 0;

            let pixel = match (and_bit, xor_bit) {
                // AND=0, XOR=0: 黑色不透明
                (false, false) => image::Rgba([0, 0, 0, 255]),
                // AND=0, XOR=1: 白色不透明
                (false, true) => image::Rgba([255, 255, 255, 255]),
                // AND=1, XOR=0: 完全透明
                (true, false) => image::Rgba([0, 0, 0, 0]),
                // AND=1, XOR=1: 反色（在 RGBA 中以半透明高亮表示）
                (true, true) => image::Rgba([128, 128, 128, 180]),
            };
            rgba_image.put_pixel(x, y, pixel);
        }
    }

    let render_image = RenderImage::new(vec![Frame::new(rgba_image)]);
    Some(CursorUpdate::Custom {
        width,
        height,
        hot_x,
        hot_y,
        image: Arc::new(render_image),
    })
}
