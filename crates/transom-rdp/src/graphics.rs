//! RDP 图形损伤与几何计算辅助模块。
//!
//! 设计原理（Why）：RDP 协议的图形更新采用分片与表面命令机制。本模块提供针对
//! 损伤包围盒的合并与边界约束等纯函数计算辅助，保证几何运算的高内聚与可测性。

use ironrdp_pdu::geometry::InclusiveRectangle;

/// 合并两个包含性矩形为最小外包围盒。
pub fn merge_rectangles(a: &InclusiveRectangle, b: &InclusiveRectangle) -> InclusiveRectangle {
    InclusiveRectangle {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}

/// 将矩形裁剪至指定桌面边界内。
pub fn clip_rectangle_to_bounds(
    rect: &InclusiveRectangle,
    max_width: u16,
    max_height: u16,
) -> Option<InclusiveRectangle> {
    if max_width == 0 || max_height == 0 {
        return None;
    }
    let limit_right = max_width.saturating_sub(1);
    let limit_bottom = max_height.saturating_sub(1);

    if rect.left > limit_right || rect.top > limit_bottom {
        return None;
    }

    Some(InclusiveRectangle {
        left: rect.left,
        top: rect.top,
        right: rect.right.min(limit_right),
        bottom: rect.bottom.min(limit_bottom),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_rectangles() {
        let r1 = InclusiveRectangle {
            left: 10,
            top: 20,
            right: 50,
            bottom: 60,
        };
        let r2 = InclusiveRectangle {
            left: 5,
            top: 25,
            right: 55,
            bottom: 40,
        };
        let merged = merge_rectangles(&r1, &r2);
        assert_eq!(merged.left, 5);
        assert_eq!(merged.top, 20);
        assert_eq!(merged.right, 55);
        assert_eq!(merged.bottom, 60);
    }

    #[test]
    fn test_clip_rectangle_to_bounds() {
        let r = InclusiveRectangle {
            left: 10,
            top: 20,
            right: 1200,
            bottom: 900,
        };
        let clipped = clip_rectangle_to_bounds(&r, 1024, 768).expect("应成功裁剪");
        assert_eq!(clipped.left, 10);
        assert_eq!(clipped.top, 20);
        assert_eq!(clipped.right, 1023);
        assert_eq!(clipped.bottom, 767);

        let outside = InclusiveRectangle {
            left: 2000,
            top: 2000,
            right: 2100,
            bottom: 2100,
        };
        assert!(clip_rectangle_to_bounds(&outside, 1024, 768).is_none());
    }
}
