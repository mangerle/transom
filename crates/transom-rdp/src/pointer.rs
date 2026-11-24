//! 远端光标位图数据结构与校验。
//!
//! 设计原理（Why）：RDP 协议通过独立的 Pointer PDU 下发光标形状与热点坐标。
//! 协议层只产出与 GUI 框架无关的纯 RGBA8 像素数据（[`CursorUpdate`]），
//! 由上层 UI 视图自行转换为 GPU 纹理并在客户端零延迟合成绘制，保持协议层纯净。

use std::sync::Arc;

/// 远端光标形状更新事件。
#[derive(Clone, Debug)]
pub enum CursorUpdate {
    /// 隐藏当前光标。
    Hidden,
    /// 恢复默认箭头光标。
    Default,
    /// 远端下发的新位图光标（32 位 RGBA8 像素）。
    Custom {
        /// 光标像素宽度。
        width: u16,
        /// 光标像素高度。
        height: u16,
        /// 光标热点水平偏移（像素）。
        hot_x: u16,
        /// 光标热点垂直偏移（像素）。
        hot_y: u16,
        /// 紧凑排布的 RGBA8 像素数据（长度严格等于 `width * height * 4`）。
        rgba: Arc<[u8]>,
    },
}

/// 校验并封装 32 位 RGBA8 光标像素载荷。
pub fn create_custom_cursor(
    width: u16,
    height: u16,
    hot_x: u16,
    hot_y: u16,
    rgba_data: &[u8],
) -> Option<CursorUpdate> {
    let expected_len = usize::from(width)
        .checked_mul(usize::from(height))?
        .checked_mul(4)?;
    if width == 0 || height == 0 || rgba_data.len() != expected_len {
        return None;
    }

    Some(CursorUpdate::Custom {
        width,
        height,
        hot_x: hot_x.min(width.saturating_sub(1)),
        hot_y: hot_y.min(height.saturating_sub(1)),
        rgba: Arc::from(rgba_data),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_cursor_validates_dimensions_and_clamps_hotspot() {
        let data = vec![255u8; 4 * 4 * 4];
        let cursor = create_custom_cursor(4, 4, 10, 10, &data).expect("合法光标应构造成功");
        match cursor {
            CursorUpdate::Custom {
                width,
                height,
                hot_x,
                hot_y,
                rgba,
            } => {
                assert_eq!((width, height), (4, 4));
                assert_eq!((hot_x, hot_y), (3, 3));
                assert_eq!(rgba.len(), 64);
            }
            _ => panic!("预期为 Custom 变体"),
        }

        assert!(create_custom_cursor(0, 4, 0, 0, &[]).is_none());
        assert!(create_custom_cursor(4, 4, 0, 0, &[0; 12]).is_none());
    }
}
