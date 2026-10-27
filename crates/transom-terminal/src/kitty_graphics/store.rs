//! Kitty 图形存储仓库与视口可见性裁剪。

use super::types::{
    GraphicsPlacement, MAX_GRAPHICS_BYTES, MAX_IMAGES, PlaceRequest, PlacedImage, StoredImage,
};
use gpui_kit::RenderImage;
use std::collections::HashMap;
use std::sync::Arc;

/// 图像与放置的生命周期仓库。
#[derive(Default)]
pub(crate) struct GraphicsStore {
    pub(crate) images: HashMap<u32, StoredImage>,
    pub(crate) placements: Vec<PlacedImage>,
    pub(crate) total_bytes: usize,
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
