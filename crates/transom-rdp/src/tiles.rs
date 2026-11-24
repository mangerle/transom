//! 256x256 增量瓦片（Tile-based）呈现引擎。
//!
//! 设计原理（Why）：在现代 GPU 加速的 UI 框架（GPUI）中，全屏图像位图的反复上传会
//! 成为极大的显存带宽瓶颈。通过将整个虚拟桌面划分为固定 256x256 像素的网格，
//! 仅对发生损伤（Dirty）的瓦片执行像素提取与纹理更新，未受影响的干净瓦片在 GPUI 中
//! 直接复用上一帧的 `Arc<RenderImage>`，从而将重绘开销限定在微秒级局部刷新。
//!
//! 权衡：增加了网格划分与局部行复制的索引计算，但换取了高达 90% 以上的 GPU 上传流量减少。

use gpui_kit::RenderImage;
use image::{Frame, RgbaImage};
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_session::image::DecodedImage;
use std::sync::Arc;

/// 瓦片网格标准尺寸（256x256 像素）。
pub const TILE_SIZE: u32 = 256;

/// 瓦片网格行数与列数计算。
pub fn compute_grid_dimensions(width: u16, height: u16) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    (
        u32::from(width).div_ceil(TILE_SIZE),
        u32::from(height).div_ceil(TILE_SIZE),
    )
}

/// 计算指定网格位置 (col, row) 的像素包围盒 (x, y, width, height)。
pub fn compute_tile_bounds(width: u16, height: u16, col: u32, row: u32) -> (u32, u32, u32, u32) {
    let left = col * TILE_SIZE;
    let top = row * TILE_SIZE;
    let right = (left + TILE_SIZE).min(u32::from(width));
    let bottom = (top + TILE_SIZE).min(u32::from(height));
    (left, top, right - left, bottom - top)
}

/// 标记脏瓦片掩码矩阵（按行优先存储的布尔切片）。
pub fn compute_dirty_mask(
    width: u16,
    height: u16,
    dirty_rects: &[InclusiveRectangle],
) -> Vec<bool> {
    let (cols, rows) = compute_grid_dimensions(width, height);
    let total = (cols * rows) as usize;
    let mut mask = vec![false; total];
    if total == 0 {
        return mask;
    }

    for rect in dirty_rects {
        if rect.left > rect.right || rect.top > rect.bottom {
            continue;
        }
        let min_x = u32::from(rect.left).min(u32::from(width) - 1);
        let min_y = u32::from(rect.top).min(u32::from(height) - 1);
        let max_x = u32::from(rect.right).min(u32::from(width) - 1);
        let max_y = u32::from(rect.bottom).min(u32::from(height) - 1);

        let col_start = min_x / TILE_SIZE;
        let col_end = max_x / TILE_SIZE;
        let row_start = min_y / TILE_SIZE;
        let row_end = max_y / TILE_SIZE;

        for r in row_start..=row_end {
            for c in col_start..=col_end {
                let index = (r * cols + c) as usize;
                if index < total {
                    mask[index] = true;
                }
            }
        }
    }
    mask
}

/// 桌面完整帧瓦片缓存。
#[derive(Clone, Default)]
pub struct TiledDesktopFrame {
    /// 桌面总像素宽度。
    pub width: u16,
    /// 桌面总像素高度。
    pub height: u16,
    /// 网格列数。
    pub cols: u32,
    /// 网格行数。
    pub rows: u32,
    /// 瓦片渲染图像句柄列表（按行优先排序）。
    pub tiles: Vec<Arc<RenderImage>>,
    /// 帧单调递增序列号。
    pub seq: u64,
    /// 本次更新受到影响的瓦片网格索引列表。
    pub dirty_indices: Vec<usize>,
}

impl TiledDesktopFrame {
    /// 增量更新或全量重建桌面瓦片集。
    ///
    /// 优化核心：若分辨率未发生变化，仅重新采样并生成标记为脏的瓦片，
    /// 干净瓦片直接继承已有 `Arc<RenderImage>` 引用，避免昂贵的 GPU 纹理再分配。
    /// 若计算后无任何实际受影响瓦片，返回 false 并跳过后续开销。
    pub fn update(&mut self, canvas: &DecodedImage, dirty_rects: &[InclusiveRectangle]) -> bool {
        let width = canvas.width();
        let height = canvas.height();
        let (cols, rows) = compute_grid_dimensions(width, height);
        let is_dimension_changed = self.width != width || self.height != height;

        if is_dimension_changed {
            self.width = width;
            self.height = height;
            self.cols = cols;
            self.rows = rows;
            self.tiles = Vec::with_capacity((cols * rows) as usize);
        }

        let dirty_mask = if is_dimension_changed {
            vec![true; (cols * rows) as usize]
        } else {
            compute_dirty_mask(width, height, dirty_rects)
        };

        let has_any_dirty = dirty_mask.iter().any(|&d| d);
        if !has_any_dirty && !is_dimension_changed {
            self.dirty_indices.clear();
            return false;
        }

        let mut dirty_indices = Vec::new();
        let mut new_tiles = Vec::with_capacity((cols * rows) as usize);
        for r in 0..rows {
            for c in 0..cols {
                let index = (r * cols + c) as usize;
                let is_dirty = dirty_mask.get(index).copied().unwrap_or(true);

                if !is_dirty && index < self.tiles.len() {
                    new_tiles.push(self.tiles[index].clone());
                } else {
                    let tile_image = extract_tile_image(canvas, c, r);
                    new_tiles.push(Arc::new(tile_image));
                    dirty_indices.push(index);
                }
            }
        }

        self.tiles = new_tiles;
        self.dirty_indices = dirty_indices;
        self.seq = self.seq.wrapping_add(1);
        true
    }
}

/// 从已解码的位图画布中提取单个瓦片的像素并转换为 RGBA 格式。
///
/// 优化说明：使用成行对齐切片与 `chunks_exact(4)` 替代逐像素点坐标计算与越界检测，
/// 循环体完全消除 bounds check，便于 LLVM 编译器进行循环展开与 SIMD 自动向量化。
fn extract_tile_image(canvas: &DecodedImage, col: u32, row: u32) -> RenderImage {
    let (left, top, tile_w, tile_h) =
        compute_tile_bounds(canvas.width(), canvas.height(), col, row);

    let mut rgba_buffer = RgbaImage::new(tile_w, tile_h);
    let canvas_stride = usize::from(canvas.width()) * 4;
    let raw_pixels = canvas.data();
    let target_raw: &mut [u8] = &mut rgba_buffer;
    let tile_w_bytes = (tile_w as usize) * 4;

    for y in 0..tile_h as usize {
        let canvas_y = top as usize + y;
        let src_start = canvas_y * canvas_stride + (left as usize) * 4;
        let src_end = src_start + tile_w_bytes;
        let dst_start = y * tile_w_bytes;
        let dst_end = dst_start + tile_w_bytes;

        if src_end <= raw_pixels.len() && dst_end <= target_raw.len() {
            let src_slice = &raw_pixels[src_start..src_end];
            let dst_slice = &mut target_raw[dst_start..dst_end];

            for (src, dst) in src_slice
                .as_chunks::<4>()
                .0
                .iter()
                .zip(dst_slice.as_chunks_mut::<4>().0.iter_mut())
            {
                // BgrX32 转换为 RGBA8：原 BGR 重新排布为 RGB，并填充不透明 Alpha
                dst[0] = src[2];
                dst[1] = src[1];
                dst[2] = src[0];
                dst[3] = 255;
            }
        }
    }

    RenderImage::new(vec![Frame::new(rgba_buffer)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_graphics::image_processing::PixelFormat;

    #[test]
    fn test_extract_tile_image_and_update() {
        let canvas = DecodedImage::new(PixelFormat::BgrX32, 512, 512);

        let _tile = extract_tile_image(&canvas, 0, 0);

        let mut desktop_frame = TiledDesktopFrame::default();
        let changed = desktop_frame.update(&canvas, &[]);
        assert!(changed);
        assert_eq!(desktop_frame.cols, 2);
        assert_eq!(desktop_frame.rows, 2);
        assert_eq!(desktop_frame.seq, 1);
        assert_eq!(desktop_frame.dirty_indices.len(), 4);

        // 无损伤时再次更新，应返回 false 且序列号不增加
        let changed_again = desktop_frame.update(&canvas, &[]);
        assert!(!changed_again);
        assert_eq!(desktop_frame.seq, 1);
        assert!(desktop_frame.dirty_indices.is_empty());
    }
}
