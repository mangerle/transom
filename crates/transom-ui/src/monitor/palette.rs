//! 监控面板专用分类色板。
//!
//! 设计原理（Why）：主界面主题色表达的是控件层级，不适合区分 CPU、内存、
//! 磁盘与网络。这里把资源类别固定成稳定色相，同时把超载阈值继续交给主题的
//! warning / danger 语义色，避免为了美观丢掉异常辨识。
//!
//! 代价与局限：分类色是监控领域的固定视觉语言，不随用户主题整体变化；浅色
//! 主题使用更深的同色相版本保证小字号可读性，深色主题沿用高亮版本。

use crate::palette::Palette;
use gpui_kit::{Hsla, rgb};

/// CPU、内存、磁盘与网络使用的固定类别色。
#[derive(Clone, Copy)]
pub(crate) struct MonitorPalette {
    pub(crate) cpu: Hsla,
    pub(crate) memory: Hsla,
    pub(crate) disk: Hsla,
    pub(crate) network_out: Hsla,
    pub(crate) network_in: Hsla,
    core_colors: [Hsla; 8],
}

impl MonitorPalette {
    /// 根据当前背景明度选择可读性合适的分类色。
    pub(crate) fn new(p: &Palette) -> Self {
        Self::for_dark(p.background.l < 0.5)
    }

    fn for_dark(dark: bool) -> Self {
        if dark {
            Self {
                cpu: rgb(0x569cd6).into(),
                memory: rgb(0x4ec9b0).into(),
                disk: rgb(0xce9178).into(),
                network_out: rgb(0xc586c0).into(),
                network_in: rgb(0x4ec9b0).into(),
                core_colors: [
                    rgb(0x569cd6).into(),
                    rgb(0x4ec9b0).into(),
                    rgb(0xc586c0).into(),
                    rgb(0xce9178).into(),
                    rgb(0xdcdcaa).into(),
                    rgb(0x9cdcfe).into(),
                    rgb(0xb5cea8).into(),
                    rgb(0xd7ba7d).into(),
                ],
            }
        } else {
            Self {
                cpu: rgb(0x2f6fb5).into(),
                memory: rgb(0x168c7a).into(),
                disk: rgb(0xb96143).into(),
                network_out: rgb(0x9b5fb7).into(),
                network_in: rgb(0x168c7a).into(),
                core_colors: [
                    rgb(0x2f6fb5).into(),
                    rgb(0x168c7a).into(),
                    rgb(0x9b4f9f).into(),
                    rgb(0xb35d3e).into(),
                    rgb(0x8a6c17).into(),
                    rgb(0x247faf).into(),
                    rgb(0x55772e).into(),
                    rgb(0x9a7028).into(),
                ],
            }
        }
    }

    /// 正常时保留类别色；超过阈值后切到主题警告色或危险色。
    pub(crate) fn usage_color(&self, base: Hsla, percent: f32, p: &Palette) -> Hsla {
        threshold_color(base, percent, p.warning, p.danger)
    }

    /// CPU 总使用率颜色。
    pub(crate) fn cpu_color(&self, percent: f32, p: &Palette) -> Hsla {
        self.usage_color(self.cpu, percent, p)
    }

    /// 内存使用率颜色。
    pub(crate) fn memory_color(&self, percent: f32, p: &Palette) -> Hsla {
        self.usage_color(self.memory, percent, p)
    }

    /// 磁盘使用率颜色。
    pub(crate) fn disk_color(&self, percent: f32, p: &Palette) -> Hsla {
        self.usage_color(self.disk, percent, p)
    }

    /// 单核负载颜色：正常使用固定八色轮盘，超载后统一切到警告语义。
    pub(crate) fn core_color(&self, index: usize, percent: f32, p: &Palette) -> Hsla {
        let base = self.core_colors[index % self.core_colors.len()];
        self.usage_color(base, percent, p)
    }
}

/// 把类别色按使用率映射到异常语义色。
fn threshold_color(base: Hsla, percent: f32, warning: Hsla, danger: Hsla) -> Hsla {
    if percent > 85.0 {
        danger
    } else if percent > 65.0 {
        warning
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::{MonitorPalette, threshold_color};
    use gpui_kit::hsla;

    #[test]
    fn category_color_survives_normal_range_and_thresholds_replace_it() {
        let base = hsla(0.0, 0.0, 0.2, 1.0);
        let warning = hsla(0.11, 0.9, 0.5, 1.0);
        let danger = hsla(0.0, 0.9, 0.5, 1.0);
        assert_eq!(threshold_color(base, 50.0, warning, danger), base);
        assert_eq!(threshold_color(base, 70.0, warning, danger), warning);
        assert_eq!(threshold_color(base, 90.0, warning, danger), danger);
    }

    #[test]
    fn core_wheel_has_eight_distinct_entries() {
        let colors = MonitorPalette::for_dark(true);
        assert_eq!(colors.core_colors.len(), 8);
        assert!(colors.core_colors.windows(2).all(|pair| pair[0] != pair[1]));
    }
}
