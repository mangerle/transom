//! 窗口外观：整扇窗口的透明度与毛玻璃。
//!
//! 这两项都是"整扇窗口"的属性，因此集中在一处：主题按透明度折算表面色，
//! 窗口本身按模式切换背景处理。两者共用同一份全局值，各处渲染不必各自去读
//! 设置，也就不会出现"侧栏半透明、标签栏不透明"的分裂。

use gpui_kit::*;
use transom_core::{WindowBackgroundMode, WindowSettings};

/// 切到半透明或毛玻璃时的默认透明度（百分比）。
///
/// 模式本身不改变界面颜色，若透明度仍是 0，用户会看到"模式切了但界面没变"。
pub(crate) const DEFAULT_WINDOW_TRANSPARENCY: u8 = 30;

/// 窗口外观的当前值；由设置改动时写入全局，渲染各处只读。
#[derive(Clone)]
pub(crate) struct WindowAppearance {
    /// 应用表面色的不透明度系数：1.0 为完全不透明。
    pub(crate) surface_alpha: f32,
}

impl Global for WindowAppearance {}

impl WindowAppearance {
    /// 按设置算出当前外观值。
    pub(crate) fn from_settings(settings: &WindowSettings) -> Self {
        let transparency = settings.window_transparency.min(100);
        Self {
            surface_alpha: 1.0 - f32::from(transparency) / 100.0,
        }
    }
}

/// 把当前窗口外观写入全局；主题与窗口据此渲染。
pub(crate) fn apply_window_appearance(settings: &WindowSettings, cx: &mut App) {
    cx.set_global(WindowAppearance::from_settings(settings));
}

/// 当前的窗口外观；未设置过时按完全不透明处理。
pub(crate) fn window_appearance(cx: &App) -> Option<WindowAppearance> {
    cx.try_global::<WindowAppearance>().cloned()
}

/// 把设置里的背景模式映射到窗口管理器的背景处理。
///
/// 窗口背景只有两档：透出桌面（透明）与模糊桌面（毛玻璃）。透明度滑块决定
/// 界面表面淡到多透，背景模式决定透出来的是桌面本身还是它的模糊版本。
pub(crate) fn background_appearance(settings: &WindowSettings) -> WindowBackgroundAppearance {
    match settings.window_background {
        WindowBackgroundMode::Blurred => WindowBackgroundAppearance::Blurred,
        // 不透明与透明都按透明处理：界面表面的不透明度由透明度滑块负责，
        // 窗口层面必须让桌面透进来，否则滑块调什么都不出效果。
        WindowBackgroundMode::Opaque | WindowBackgroundMode::Transparent => {
            WindowBackgroundAppearance::Transparent
        }
    }
}
