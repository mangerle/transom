//! 随应用分发的内置字体。
//!
//! 设计原理：终端默认字体不能依赖用户系统预装，否则同一份安装包在不同机器上
//! 会呈现不同字形；同时运行期下载字体又会破坏离线可用性与启动确定性。因此把
//! 终端需要的四个基础字形直接编入二进制，在窗口创建前注册到 GPUI 文本系统。
//! 这样 DEB、AppImage、NSIS 与 macOS 包共享同一字体版本，代价是安装包增加约
//! 1.1 MiB，并需要随字体文件一同分发 OFL 许可证。

use gpui_kit::{App, Global, SharedString};
use std::borrow::Cow;

/// 内置终端等宽字体的族名。
///
/// 该名称必须与实际 TTF 的 family 字段完全一致；主题在未显式指定等宽字体时
/// 使用它，终端渲染器则在会话未指定字体时跟随主题。
pub(crate) const TERMINAL_MONO_FONT_FAMILY: &str = "JetBrains Mono";

/// JetBrains Mono 2.304 的常规、粗体、斜体与粗斜体字形。
const FONT_DATA: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-Bold.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-Italic.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-BoldItalic.ttf"),
];

/// 内置字体注册结果。
///
/// 字体解析发生在启动阶段，失败时继续启动比直接崩溃更符合桌面应用的可恢复性
/// 要求；记录该状态后，主题会保留系统等宽字体，而不是引用一个未注册的族名。
#[derive(Clone, Copy, Debug)]
struct BundledFontState {
    mono_font_available: bool,
}

impl Global for BundledFontState {}

/// 将内置字体注册到 GPUI 文本系统。
///
/// 必须在首次布局文字前调用。失败时记录中文错误并进入系统字体回退路径，避免
/// 后续主题引用不存在的字族导致 GPUI 在排版时崩溃。
pub(crate) fn register(cx: &mut App) {
    let fonts = FONT_DATA.into_iter().map(Cow::Borrowed).collect::<Vec<_>>();
    let mono_font_available = match cx.text_system().add_fonts(fonts) {
        Ok(()) => {
            log::info!("内置终端字体已注册，字体: {TERMINAL_MONO_FONT_FAMILY}，版本: 2.304");
            true
        }
        Err(error) => {
            log::error!(
                "注册内置终端字体失败，终端将回退到系统等宽字体，字体: {TERMINAL_MONO_FONT_FAMILY}，原因: {error}"
            );
            false
        }
    };
    cx.set_global(BundledFontState {
        mono_font_available,
    });
}

/// 返回可用的内置等宽字族。
///
/// 只在字体注册成功时返回名称；调用方应把 `None` 解释为继续沿用主题或平台的
/// 系统等宽字体，而不是再次尝试加载字体。
pub(crate) fn default_mono_font_family(cx: &App) -> Option<SharedString> {
    cx.try_global::<BundledFontState>()
        .filter(|state| state.mono_font_available)
        .map(|_| SharedString::from(TERMINAL_MONO_FONT_FAMILY))
}

#[cfg(test)]
mod tests {
    use super::{FONT_DATA, TERMINAL_MONO_FONT_FAMILY, default_mono_font_family, register};
    use gpui_kit::TestAppContext;

    #[test]
    fn embedded_fonts_are_valid_true_type_files() {
        for bytes in FONT_DATA {
            assert!(bytes.starts_with(&[0x00, 0x01, 0x00, 0x00]));
        }
    }

    #[test]
    fn bundled_font_registers_expected_family() {
        let app = TestAppContext::single();
        app.update(|cx| {
            register(cx);
            assert_eq!(
                default_mono_font_family(cx).as_deref(),
                Some(TERMINAL_MONO_FONT_FAMILY),
                "内置字体注册成功后应发布默认等宽字族"
            );
        });
    }
}
