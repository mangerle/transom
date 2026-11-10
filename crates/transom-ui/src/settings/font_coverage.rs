//! 字体族中文覆盖检测。
//!
//! 设计原理：字体名称无法可靠说明某个字族是否包含中文字形，直接按名称维护
//! 黑名单会在用户导入字体或系统字体更新后失真。这里向 GPUI 文本系统查询一组
//! 常用简体中文字符的真实 glyph 覆盖，结果按字族缓存，避免设置页每帧重复查询。
//! 代价是缓存以进程内字族名为键；字体文件原位替换或重新导入同名族时必须显式
//! 清空缓存。

use gpui_kit::{App, Font, FontFeatures, FontStyle, FontWeight, px};
use std::cell::RefCell;
use std::collections::HashMap;

/// 用于判断简体中文字体覆盖的探针。
///
/// 刻意包含“设”“置”“终”“端”等简体字形：只支持日文假名或繁体的字体不应被
/// 当作适合中文界面的字体。
const CHINESE_PROBE: &str = "\u{4e2d}\u{6587}\u{754c}\u{9762}\u{7ec8}\u{7aef}\u{8bbe}\u{7f6e}";

thread_local! {
    /// 字族名到中文覆盖状态的进程内缓存；所有访问都在 UI 线程。
    static CHINESE_SUPPORT_CACHE: RefCell<HashMap<String, ChineseFontSupport>> =
        RefCell::new(HashMap::new());
}

/// 字体族对简体中文字形的覆盖状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChineseFontSupport {
    /// 字体已安装，并且包含全部探针字形。
    Supported,
    /// 字体已安装，但至少缺少一个探针字形。
    MissingGlyphs,
    /// 配置中的字族当前未安装，无法检测；下拉列表已有“未安装”标记。
    NotInstalled,
}

/// 检测指定字族能否直接渲染简体中文。
pub(crate) fn chinese_font_support(cx: &App, family: &str) -> ChineseFontSupport {
    CHINESE_SUPPORT_CACHE.with(|cache| {
        if let Some(support) = cache.borrow().get(family).copied() {
            return support;
        }
        let support = probe_installed_font(cx, family);
        cache.borrow_mut().insert(family.to_owned(), support);
        support
    })
}

/// 清空字形缓存。
///
/// 每次打开设置页、导入字体后调用：系统字体可能在进程外更新，导入也可能用
/// 同名文件覆盖上一份字形数据。
pub(crate) fn clear_chinese_support_cache() {
    CHINESE_SUPPORT_CACHE.with(|cache| cache.borrow_mut().clear());
}

fn probe_installed_font(cx: &App, family: &str) -> ChineseFontSupport {
    let installed = cx.text_system().all_font_names();
    if !installed.iter().any(|name| name == family) {
        return ChineseFontSupport::NotInstalled;
    }
    let font = Font {
        family: family.into(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    };
    // 调用前已由 `all_font_names()` 证明字族存在；GPUI 的 `resolve_font`
    // 在这些平台上会解析已注册字族，因此这里不需要再做一次名称猜测。
    let font_id = cx.text_system().resolve_font(&font);
    let supported = glyphs_cover_chinese(|character| {
        cx.text_system()
            .typographic_bounds(font_id, px(14.0), character)
            .is_ok()
    });
    if supported {
        ChineseFontSupport::Supported
    } else {
        ChineseFontSupport::MissingGlyphs
    }
}

fn glyphs_cover_chinese(mut glyph_exists: impl FnMut(char) -> bool) -> bool {
    CHINESE_PROBE.chars().all(&mut glyph_exists)
}

#[cfg(test)]
mod tests {
    use super::glyphs_cover_chinese;

    #[test]
    fn chinese_probe_requires_every_glyph() {
        assert!(glyphs_cover_chinese(|_| true));
        assert!(!glyphs_cover_chinese(|character| character != '\u{8bbe}'));
    }
}
