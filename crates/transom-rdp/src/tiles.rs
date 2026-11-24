//! 256x256 增量瓦片（Tile-based）像素提取引擎。
//!
//! 设计原理（Why）：
//! 1. 与 UI 框架彻底解耦：协议层仅产出纯 RGBA8 增量瓦片补丁（[`TilePatch`]），
//!    不持有任何 GUI 纹理句柄（如 GPUI `RenderImage`），确保 `transom-rdp` 可独立编译与测试；
//! 2. 增量传输零整表克隆：每次刷新仅传输实际受损瓦片的像素，UI 侧按 `(col, row)` 原地替换
//!    对应瓦片纹理，彻底消除每帧克隆整张瓦片句柄表的开销；
//! 3. SIMD 友好热路径：按行切片使用 `as_chunks::<4>()` / `as_chunks_mut::<4>()` 成对拷贝，
//!    避免内层循环逐像素 `extend_from_slice`，便于 LLVM 自动向量化；同时复用 `dirty_mask`
//!    布尔缓冲消除每帧堆分配。

use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_session::image::DecodedImage;

/// 瓦片网格标准尺寸（256x256 像素）。
pub const TILE_SIZE: u32 = 256;

/// 单个受损瓦片的 RGBA8 像素补丁。
#[derive(Debug, Clone)]
pub struct TilePatch {
    /// 瓦片所在列索引（0 起）。
    pub col: u32,
    /// 瓦片所在行索引（0 起）。
    pub row: u32,
    /// 瓦片实际像素宽度（边缘瓦片可能小于 [`TILE_SIZE`]）。
    pub width: u32,
    /// 瓦片实际像素高度（边缘瓦片可能小于 [`TILE_SIZE`]）。
    pub height: u32,
    /// 紧凑排布的 RGBA8 像素字节（长度严格等于 `width * height * 4`）。
    pub rgba: Vec<u8>,
}

/// 一帧桌面增量更新载荷。
#[derive(Debug, Clone)]
pub struct FrameUpdate {
    /// 桌面总像素宽度。
    pub width: u16,
    /// 桌面总像素高度。
    pub height: u16,
    /// 网格列数。
    pub cols: u32,
    /// 网格行数。
    pub rows: u32,
    /// 帧单调递增序列号。
    pub seq: u64,
    /// 是否因首次渲染或分辨率变化触发全量重置。
    pub full_reset: bool,
    /// 本帧发生变化的瓦片补丁列表。
    pub patches: Vec<TilePatch>,
}

/// 瓦片网格列数与行数计算。
pub fn compute_grid_dimensions(width: u16, height: u16) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    (
        u32::from(width).div_ceil(TILE_SIZE),
        u32::from(height).div_ceil(TILE_SIZE),
    )
}

/// 计算指定网格位置 `(col, row)` 的像素包围盒 `(x, y, width, height)`。
pub fn compute_tile_bounds(width: u16, height: u16, col: u32, row: u32) -> (u32, u32, u32, u32) {
    let left = col.saturating_mul(TILE_SIZE);
    let top = row.saturating_mul(TILE_SIZE);
    let right = left.saturating_add(TILE_SIZE).min(u32::from(width));
    let bottom = top.saturating_add(TILE_SIZE).min(u32::from(height));
    (
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    )
}

/// 标记脏瓦片掩码矩阵（按行优先存储的布尔切片），复用调用方提供的缓冲避免重复分配。
pub fn fill_dirty_mask(
    width: u16,
    height: u16,
    dirty_rects: &[InclusiveRectangle],
    mask: &mut Vec<bool>,
) {
    let (cols, rows) = compute_grid_dimensions(width, height);
    let total = (cols * rows) as usize;
    mask.resize(total, false);
    mask.fill(false);
    if total == 0 {
        return;
    }

    let max_valid_x = u32::from(width) - 1;
    let max_valid_y = u32::from(height) - 1;

    for rect in dirty_rects {
        if rect.left > rect.right || rect.top > rect.bottom {
            continue;
        }
        let min_x = u32::from(rect.left).min(max_valid_x);
        let min_y = u32::from(rect.top).min(max_valid_y);
        let max_x = u32::from(rect.right).min(max_valid_x);
        let max_y = u32::from(rect.bottom).min(max_valid_y);

        let col_start = min_x / TILE_SIZE;
        let col_end = max_x / TILE_SIZE;
        let row_start = min_y / TILE_SIZE;
        let row_end = max_y / TILE_SIZE;

        for r in row_start..=row_end {
            let row_base = (r * cols) as usize;
            for c in col_start..=col_end {
                let index = row_base + c as usize;
                if let Some(slot) = mask.get_mut(index) {
                    *slot = true;
                }
            }
        }
    }
}

/// 便捷纯函数：计算并返回新的脏瓦片掩码矩阵。
pub fn compute_dirty_mask(
    width: u16,
    height: u16,
    dirty_rects: &[InclusiveRectangle],
) -> Vec<bool> {
    let mut mask = Vec::new();
    fill_dirty_mask(width, height, dirty_rects, &mut mask);
    mask
}

/// 有状态的增量瓦片提取器，复用内部脏掩码缓冲以减少高频帧更新的内存分配。
#[derive(Default)]
pub struct TileExtractor {
    width: u16,
    height: u16,
    seq: u64,
    dirty_mask: Vec<bool>,
}

impl TileExtractor {
    /// 强制重置状态（如收到 `DeactivateAll` 重建画布时调用），确保下一帧执行全量同步。
    pub fn reset(&mut self) {
        self.width = 0;
        self.height = 0;
        self.dirty_mask.clear();
    }

    /// 根据当前画布与损伤矩形集合提取增量瓦片补丁；无任何受损瓦片时返回 `None`。
    pub fn extract(
        &mut self,
        canvas: &DecodedImage,
        dirty_rects: &[InclusiveRectangle],
    ) -> Option<FrameUpdate> {
        let width = canvas.width();
        let height = canvas.height();
        let (cols, rows) = compute_grid_dimensions(width, height);
        let total = (cols * rows) as usize;
        if total == 0 {
            return None;
        }

        let full_reset = self.width != width || self.height != height;
        if full_reset {
            self.width = width;
            self.height = height;
            let mut patches = Vec::with_capacity(total);
            for r in 0..rows {
                for c in 0..cols {
                    patches.push(extract_tile_patch(canvas, c, r));
                }
            }
            self.seq = self.seq.wrapping_add(1);
            return Some(FrameUpdate {
                width,
                height,
                cols,
                rows,
                seq: self.seq,
                full_reset: true,
                patches,
            });
        }

        fill_dirty_mask(width, height, dirty_rects, &mut self.dirty_mask);
        let dirty_count = self.dirty_mask.iter().filter(|&&d| d).count();
        if dirty_count == 0 {
            return None;
        }

        let mut patches = Vec::with_capacity(dirty_count);
        for (index, &is_dirty) in self.dirty_mask.iter().enumerate() {
            if is_dirty {
                let c = (index as u32) % cols;
                let r = (index as u32) / cols;
                patches.push(extract_tile_patch(canvas, c, r));
            }
        }

        self.seq = self.seq.wrapping_add(1);
        Some(FrameUpdate {
            width,
            height,
            cols,
            rows,
            seq: self.seq,
            full_reset: false,
            patches,
        })
    }
}

/// 从已解码的 BgrX32 位图画布中提取单个瓦片的像素并转换为 RGBA8。
fn extract_tile_patch(canvas: &DecodedImage, col: u32, row: u32) -> TilePatch {
    let (left, top, tile_w, tile_h) =
        compute_tile_bounds(canvas.width(), canvas.height(), col, row);
    let tile_w_bytes = (tile_w as usize) * 4;
    let total_bytes = tile_w_bytes * (tile_h as usize);
    let mut rgba = vec![0u8; total_bytes];

    let canvas_stride = usize::from(canvas.width()) * 4;
    let raw_pixels = canvas.data();

    for y in 0..tile_h as usize {
        let src_start = (top as usize + y) * canvas_stride + (left as usize) * 4;
        let src_end = src_start + tile_w_bytes;
        let dst_start = y * tile_w_bytes;
        let dst_end = dst_start + tile_w_bytes;

        if src_end <= raw_pixels.len() {
            let src_row = &raw_pixels[src_start..src_end];
            let dst_row = &mut rgba[dst_start..dst_end];
            for (dst, src) in dst_row
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(src_row.as_chunks::<4>().0)
            {
                // BgrX32 -> RGBA8：通道重排并填充不透明 Alpha
                *dst = [src[2], src[1], src[0], 255];
            }
        }
    }

    TilePatch {
        col,
        row,
        width: tile_w,
        height: tile_h,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_graphics::image_processing::PixelFormat;

    #[test]
    fn extractor_full_reset_and_incremental_update() {
        let canvas = DecodedImage::new(PixelFormat::BgrX32, 512, 512);
        let mut extractor = TileExtractor::default();

        // 首次提取触发全量重置（4 个 256x256 瓦片）
        let first = extractor
            .extract(&canvas, &[])
            .expect("首次提取应产出全量帧");
        assert!(first.full_reset);
        assert_eq!(first.cols, 2);
        assert_eq!(first.rows, 2);
        assert_eq!(first.seq, 1);
        assert_eq!(first.patches.len(), 4);
        assert_eq!(first.patches[0].rgba.len(), 256 * 256 * 4);

        // 无损伤时跳过更新
        assert!(extractor.extract(&canvas, &[]).is_none());

        // 局部损伤仅提取对应瓦片
        let rect = InclusiveRectangle {
            left: 10,
            top: 10,
            right: 20,
            bottom: 20,
        };
        let inc = extractor
            .extract(&canvas, &[rect])
            .expect("局部损伤应产出增量帧");
        assert!(!inc.full_reset);
        assert_eq!(inc.seq, 2);
        assert_eq!(inc.patches.len(), 1);
        assert_eq!((inc.patches[0].col, inc.patches[0].row), (0, 0));
    }
}
