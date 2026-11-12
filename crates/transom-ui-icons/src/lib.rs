//! IntelliJ New UI 图标、文件类型图标、领域图标与项目自有图标。
//!
//! 设计原理（Why）：图标资产、分类选择与全彩 SVG 光栅化缓存属于独立
//! 关注点。集中到本 crate 后，`transom-ui` 只需要提供应用 `AssetSource` 并调用
//! `render_app_icon`，图标渲染不会反向依赖界面状态或业务模块。
//!
//! 权衡：本 crate 依赖 GPUI 的 SVG 光栅化器与主题解析器，因此不是
//! 纯数据包；但这样可以把“保留 IntelliJ 原始颜色”的复杂渲染细节完全封装起来。

mod custom_icons;
mod domain_icons;
mod file_type_icons;
mod ui_icons;

pub use custom_icons::CustomIcon;
pub use domain_icons::DomainIcon;
pub use file_type_icons::FileTypeIcon;
pub use ui_icons::UiIcon;

use gpui_kit::assets::IconNamed;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::component::{resolve_icon_path, set_icon_resolver};
use gpui_kit::*;
use parking_lot::Mutex;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// 应用层图标：New UI、文件类型、领域和项目自有图标各自分组。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppIcon {
    Ui(UiIcon),
    FileType(FileTypeIcon),
    Domain(DomainIcon),
    Custom(CustomIcon),
}

impl From<UiIcon> for AppIcon {
    fn from(icon: UiIcon) -> Self {
        Self::Ui(icon)
    }
}

impl From<FileTypeIcon> for AppIcon {
    fn from(icon: FileTypeIcon) -> Self {
        Self::FileType(icon)
    }
}

impl From<DomainIcon> for AppIcon {
    fn from(icon: DomainIcon) -> Self {
        Self::Domain(icon)
    }
}

impl From<CustomIcon> for AppIcon {
    fn from(icon: CustomIcon) -> Self {
        Self::Custom(icon)
    }
}

impl AppIcon {
    /// 按文件路径选择对应文件类型图标。
    pub fn for_file(path: &str) -> Self {
        FileTypeIcon::for_path(path).into()
    }
}

/// 可直接放入 GPUI 布局树的全彩应用图标。
///
/// 设计原理（Why）：`Icon` 会把 SVG 当成单色 alpha 蒙版，IntelliJ 带背景色的
/// 图标经它渲染会变成实心方块。该类型保留与 `Icon` 相近的链式尺寸接口，但最终
/// 走 [`render_app_icon`] 的全彩位图链路，供列表、标签、工具栏和按钮统一使用。
///
/// 权衡：颜色由资产自身决定，不再跟随 `text_color`；需要主题色驱动的旧单色
/// 图标不应使用该类型。
#[derive(Clone, Copy, IntoElement)]
pub struct AppIconElement {
    icon: AppIcon,
    size: AppIconSize,
    tint: Option<Hsla>,
    force_monochrome: bool,
}

/// 全彩图标的逻辑尺寸单位。
#[derive(Clone, Copy, Debug)]
pub enum AppIconSize {
    /// 固定逻辑像素尺寸。
    Pixels(Pixels),
    /// 相对根字号的 rem 尺寸。
    Rems(Rems),
}

impl From<Pixels> for AppIconSize {
    fn from(size: Pixels) -> Self {
        Self::Pixels(size)
    }
}

impl From<Rems> for AppIconSize {
    fn from(size: Rems) -> Self {
        Self::Rems(size)
    }
}

impl AppIconElement {
    /// 创建全彩图标，默认按 16px 逻辑尺寸绘制。
    pub fn new(icon: impl Into<AppIcon>) -> Self {
        Self {
            icon: icon.into(),
            size: Rems(1.0).into(),
            tint: None,
            force_monochrome: false,
        }
    }

    /// 使用紧凑尺寸，对应组件库的 `small()`。
    pub fn small(mut self) -> Self {
        self.size = Rems(1.0).into();
        self
    }

    /// 使用更紧凑尺寸，对应组件库的 `xsmall()`。
    pub fn xsmall(mut self) -> Self {
        self.size = Rems(0.875).into();
        self
    }

    /// 设置逻辑像素尺寸。
    pub fn with_size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = size.into().into();
        self
    }

    /// 设置 rem 尺寸，随根字号与界面密度缩放。
    pub fn with_rems(mut self, size: impl Into<Rems>) -> Self {
        self.size = size.into().into();
        self
    }

    /// 与 `Styled::size` 对齐的尺寸设置入口。
    pub fn size(mut self, size: impl Into<AppIconSize>) -> Self {
        self.size = size.into();
        self
    }

    /// 与 `Styled::size_3` 对齐的 12px 尺寸。
    pub fn size_3(self) -> Self {
        self.with_rems(rems(0.75))
    }

    /// 与 `Styled::size_4` 对齐的 16px 尺寸。
    pub fn size_4(self) -> Self {
        self.with_rems(rems(1.0))
    }

    /// 图标本身已经固定宽度，保留该调用以兼容原链式写法。
    pub fn flex_none(self) -> Self {
        self
    }

    /// 为自有单色图标设置状态色；文件类型与 IntelliJ 全彩资产仍保留原始颜色。
    ///
    /// 设计原理（Why）：探测、连接测试与容器引擎等自有轮廓图标需要由主题和状态着色，
    /// 而文件类型等全彩资产不能被统一染色。只对 `CustomIcon` 走 GPUI 的
    /// 单色 SVG 链路，可以同时满足两类资产的颜色契约。
    pub fn text_color(mut self, color: Hsla) -> Self {
        self.tint = Some(color);
        self
    }

    /// 强制把任意图标按单色蒙版渲染。
    ///
    /// 设计原理（Why）：选中页签需要所有图标统一跟随强调色，而 IntelliJ 领域图标
    /// 默认保留资产原色。该入口只供明确需要单色状态的组件使用，避免全局改变多色
    /// 文件类型和品牌图标的展示契约。
    ///
    /// 代价与局限：多色资产会丢失内部颜色层次，因此普通列表与工具栏不得默认调用。
    pub fn monochrome(mut self, color: Hsla) -> Self {
        self.tint = Some(color);
        self.force_monochrome = true;
        self
    }
}

impl RenderOnce for AppIconElement {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let size = match self.size {
            AppIconSize::Pixels(size) => size,
            AppIconSize::Rems(size) => size.to_pixels(window.rem_size()),
        };
        if (self.force_monochrome || matches!(self.icon, AppIcon::Custom(_)))
            && let Some(tint) = self.tint
        {
            return Icon::new(self.icon)
                .text_color(tint)
                .w(size)
                .h(size)
                .into_any_element();
        }
        render_app_icon(self.icon, size.into()).into_any_element()
    }
}

impl IconNamed for AppIcon {
    fn path(self) -> SharedString {
        match self {
            Self::Ui(icon) => icon.light_path(),
            Self::FileType(icon) => icon.light_path(),
            Self::Domain(icon) => icon.light_path(),
            Self::Custom(icon) => icon.light_path(),
        }
    }
}

/// 读取本 crate 嵌入的图标资产。
pub fn asset_data(path: &str) -> Option<Cow<'static, [u8]>> {
    file_type_icons::asset_data(path)
        .or_else(|| ui_icons::asset_data(path))
        .or_else(|| domain_icons::asset_data(path))
        .or_else(|| custom_icons::asset_data(path))
}

/// 列出本 crate 嵌入的全部图标路径。
pub fn asset_paths() -> impl Iterator<Item = SharedString> {
    file_type_icons::asset_paths()
        .chain(ui_icons::asset_paths())
        .chain(domain_icons::asset_paths())
        .chain(custom_icons::asset_paths())
}

/// 安装主题感知的 IntelliJ 图标解析器。
///
/// 只处理本图标包内的浅色资产；深色主题下优先替换为对应的 `_dark.svg`，
/// 没有深色版本时继续使用上游浅色资产。
pub fn install_icon_resolver(cx: &mut App) {
    set_icon_resolver(cx, |path, cx| {
        if cx.theme().is_dark() {
            file_type_icons::dark_variant(path)
                .or_else(|| ui_icons::dark_variant(path))
                .or_else(|| domain_icons::dark_variant(path))
                .or_else(|| custom_icons::dark_variant(path))
        } else {
            None
        }
    });
}

/// 全彩 SVG 的解析与位图缓存。
struct ColoredIconCache {
    parsed: HashMap<SharedString, ParsedSvg>,
    rasterized: HashMap<(SharedString, u32), Arc<RenderImage>>,
}

static COLORED_ICON_CACHE: OnceLock<Mutex<ColoredIconCache>> = OnceLock::new();

fn colored_icon_cache() -> &'static Mutex<ColoredIconCache> {
    COLORED_ICON_CACHE.get_or_init(|| {
        Mutex::new(ColoredIconCache {
            parsed: HashMap::with_capacity(96),
            rasterized: HashMap::with_capacity(192),
        })
    })
}

/// 渲染一个应用图标，并保留 IntelliJ 资产的原始颜色。
///
/// 设计原理（Why）：GPUI 的 `svg()` 会把 SVG 光栅化为 alpha 蒙版后统一着色，
/// IntelliJ 的多色文件图标与双色节点图标因此会变成黑块。这里直接光栅化为全彩
/// 位图，并按“资产路径 + 设备像素”缓存。
pub fn render_app_icon(icon: AppIcon, logical_size: f32) -> AnyElement {
    let source = ImageSource::Custom(Arc::new(move |window, cx| {
        rasterize_colored_icon(icon, logical_size, window, cx).map(Ok)
    }));
    div()
        .flex_none()
        .size(px(logical_size))
        .child(img(source).size_full().object_fit(ObjectFit::Contain))
        .into_any_element()
}

fn rasterize_colored_icon(
    icon: AppIcon,
    logical_size: f32,
    window: &Window,
    cx: &App,
) -> Option<Arc<RenderImage>> {
    let path = resolve_icon_path(icon.path(), cx);
    let device_px = (logical_size * window.scale_factor()).ceil().max(1.0) as u32;
    let key = (path.clone(), device_px);
    let renderer = cx.svg_renderer();
    let mut cache = colored_icon_cache().lock();
    if let Some(image) = cache.rasterized.get(&key) {
        return Some(image.clone());
    }

    if !cache.parsed.contains_key(&path) {
        let data = asset_data(&path)?;
        let parsed = match renderer.parse_svg(&data) {
            Ok(parsed) => parsed,
            Err(error) => {
                log::warn!("解析应用图标失败，资产: {path}，原因: {error}");
                return None;
            }
        };
        cache.parsed.insert(path.clone(), parsed);
    }
    let parsed = cache.parsed.get(&path)?;
    let pixels = DevicePixels(device_px as i32);
    let image = match renderer.render_parsed(parsed, SvgSize::Size(size(pixels, pixels))) {
        Ok(image) => image,
        Err(error) => {
            log::warn!("光栅化应用图标失败，资产: {path}，原因: {error}");
            return None;
        }
    };
    cache.rasterized.insert(key, image.clone());
    Some(image)
}

#[cfg(test)]
mod tests {
    use super::{asset_data, asset_paths};

    #[test]
    fn embedded_assets_cover_all_icon_categories() {
        assert!(asset_data("icons/file-types/rust.svg").is_some());
        assert!(asset_data("icons/ui/search.svg").is_some());
        assert!(asset_data("icons/domains/ssh.svg").is_some());
        assert!(asset_data("icons/custom/network.svg").is_some());
        assert!(asset_data("icons/custom/docker.svg").is_some());
        assert!(asset_paths().any(|path| path.as_ref() == "icons/ui/settings.svg"));
    }
}
