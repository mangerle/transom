//! 终端正文的最小对比度修正。
//!
//! 修正只发生在绘制层：ANSI、真彩色、复制文本和会话记录均保持原值。算法在
//! 原始前景色与黑/白端点之间二分搜索离原色最近、且达到目标对比度的颜色，并用
//! 有界缓存避免同一颜色组合在每帧重复计算。

use gpui_kit::{Hsla, rgb};
use std::collections::{HashMap, VecDeque};

const CONTRAST_CACHE_CAP: usize = 4096;

pub(super) fn normalize_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(1.0, 21.0)
    } else {
        1.0
    }
}

fn luminance(value: u32) -> f64 {
    let channel = |shift: u32| {
        let component = ((value >> shift) & 0xff) as f64 / 255.0;
        if component <= 0.04045 {
            component / 12.92
        } else {
            ((component + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

pub(super) fn contrast_ratio(foreground: u32, background: u32) -> f32 {
    let foreground = luminance(foreground);
    let background = luminance(background);
    ((foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)) as f32
}

fn color_to_u32(color: Hsla) -> u32 {
    let rgba = color.to_rgb();
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    (channel(rgba.r) << 16) | (channel(rgba.g) << 8) | channel(rgba.b)
}

fn mix(foreground: u32, target: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let foreground = ((foreground >> shift) & 0xff) as f32;
        let target = ((target >> shift) & 0xff) as f32;
        (foreground + (target - foreground) * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn candidate(foreground: u32, background: u32, target: u32, ratio: f32) -> Option<u32> {
    if contrast_ratio(target, background) < ratio {
        return None;
    }
    let (mut low, mut high) = (0.0f32, 1.0f32);
    let mut best = target;
    for _ in 0..16 {
        let mid = (low + high) / 2.0;
        let candidate = mix(foreground, target, mid);
        if contrast_ratio(candidate, background) >= ratio {
            high = mid;
            best = candidate;
        } else {
            low = mid;
        }
    }
    Some(best)
}

fn distance(left: u32, right: u32) -> u32 {
    [16, 8, 0]
        .into_iter()
        .map(|shift| {
            let delta = ((left >> shift) & 0xff) as i32 - ((right >> shift) & 0xff) as i32;
            (delta * delta) as u32
        })
        .sum()
}

fn ensure_contrast(foreground: u32, background: u32, ratio: f32) -> u32 {
    if ratio <= 1.0 || contrast_ratio(foreground, background) >= ratio {
        return foreground;
    }
    match (
        candidate(foreground, background, 0, ratio),
        candidate(foreground, background, 0xffffff, ratio),
    ) {
        (Some(dark), Some(light)) => {
            if distance(foreground, dark) <= distance(foreground, light) {
                dark
            } else {
                light
            }
        }
        (Some(color), None) | (None, Some(color)) => color,
        (None, None) => {
            if contrast_ratio(0, background) >= contrast_ratio(0xffffff, background) {
                0
            } else {
                0xffffff
            }
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct TerminalContrastCache {
    colors: HashMap<(u32, u32, u32), u32>,
    order: VecDeque<(u32, u32, u32)>,
}

impl TerminalContrastCache {
    pub(super) fn resolve(&mut self, foreground: u32, background: u32, ratio: f32) -> u32 {
        let ratio = normalize_ratio(ratio);
        if ratio <= 1.0 {
            return foreground;
        }
        let key = (foreground, background, ratio.to_bits());
        if let Some(color) = self.colors.get(&key) {
            return *color;
        }
        let color = ensure_contrast(foreground, background, ratio);
        if self.colors.len() == CONTRAST_CACHE_CAP
            && let Some(oldest) = self.order.pop_front()
        {
            self.colors.remove(&oldest);
        }
        self.colors.insert(key, color);
        self.order.push_back(key);
        color
    }
}

/// 终端几何图形不应被正文对比度算法改色。
///
/// 制表框线与 Powerline 私有区符号依靠相邻色块拼合形状，颜色被单独推亮或压暗
/// 会破坏接缝；普通文字、空格与 ANSI 色块仍按目标比值修正。
pub(super) fn contrast_excluded(character: char) -> bool {
    character.is_whitespace()
        || ('\u{2500}'..='\u{259f}').contains(&character)
        || ('\u{e0a0}'..='\u{e0d7}').contains(&character)
}

pub(super) fn corrected_color(
    cache: &mut TerminalContrastCache,
    foreground: Hsla,
    background: Hsla,
    ratio: f32,
    character: char,
) -> Hsla {
    if contrast_excluded(character) {
        return foreground;
    }
    let color = cache.resolve(color_to_u32(foreground), color_to_u32(background), ratio);
    let mut corrected = Hsla::from(rgb(color));
    corrected.a = foreground.a;
    corrected
}

#[cfg(test)]
mod tests {
    use super::{
        CONTRAST_CACHE_CAP, TerminalContrastCache, contrast_excluded, contrast_ratio,
        ensure_contrast, normalize_ratio,
    };
    use gpui_kit::{Hsla, rgb};

    #[test]
    fn correction_reaches_target_or_best_available_endpoint() {
        for ratio in [1.0, 3.0, 4.5, 7.0, 21.0] {
            for background in [0x000000, 0xffffff, 0x112233, 0x777777, 0x41d255] {
                let corrected = ensure_contrast(0x28dcdc, background, ratio);
                let available =
                    contrast_ratio(0, background).max(contrast_ratio(0xffffff, background));
                assert!(contrast_ratio(corrected, background) >= ratio.min(available));
            }
        }
        assert_eq!(ensure_contrast(0x28dcdc, 0x112233, 1.0), 0x28dcdc);
    }

    #[test]
    fn cache_is_bounded_and_reuses_existing_entries() {
        let mut cache = TerminalContrastCache::default();
        let first = cache.resolve(0x123456, 0xffffff, 4.5);
        assert_eq!(cache.resolve(0x123456, 0xffffff, 4.5), first);
        for color in 1..=CONTRAST_CACHE_CAP as u32 {
            cache.resolve(color, 0x112233, 4.5);
        }
        assert_eq!(cache.colors.len(), CONTRAST_CACHE_CAP);
        assert_eq!(cache.order.len(), CONTRAST_CACHE_CAP);
    }

    #[test]
    fn invalid_ratios_disable_and_finite_ratios_clamp() {
        assert_eq!(normalize_ratio(f32::NAN), 1.0);
        assert_eq!(normalize_ratio(f32::INFINITY), 1.0);
        assert_eq!(normalize_ratio(100.0), 21.0);
        assert_eq!(normalize_ratio(-1.0), 1.0);
    }

    #[test]
    fn preserves_alpha_and_excludes_powerline_glyphs() {
        let mut cache = TerminalContrastCache::default();
        let foreground = Hsla::from(rgb(0x28dcdc)).alpha(0.7);
        let corrected =
            super::corrected_color(&mut cache, foreground, Hsla::from(rgb(0x41d255)), 7.0, 'A');
        assert!((corrected.a - 0.7).abs() < 1e-6);
        for ch in [' ', '─', '\u{e0b0}'] {
            assert!(contrast_excluded(ch));
            let same =
                super::corrected_color(&mut cache, foreground, Hsla::from(rgb(0x41d255)), 7.0, ch);
            assert_eq!(same, foreground);
        }
    }
}
