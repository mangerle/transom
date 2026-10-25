//! 品牌舷窗标志：内嵌仓库根 `logo/transom-logo.svg`，按目标窗口的设备像素光栅化
//! 后渲染成顶栏与"关于"页使用的标志图。
//!
//! 设计原理（Why）：
//! - 为什么不能用 `ImageSource::Image(svg)`：gpui 的 `Image::to_image_data` 对
//!   `ImageFormat::Svg` 写死了 `render_single_frame(bytes, 1.0)`，即按 SVG 的固有
//!   尺寸出图。本标志固有尺寸是 512×512，纹理因此恒为 512 见方，而顶栏只有 18px
//!   逻辑宽（约 27~36 设备像素），关于页也只有 48px；GPUI 的图像链路不带 mipmap
//!   （全仓无 min_filter/mipmap 设置），十几倍的缩小全靠双线性重采样，结果是欠采样：
//!   细线（环带描边 30/512 在 18px 处只剩 1px）碎成颗粒、边缘发毛，窗口移动时还会闪。
//! - 为什么不用 `svg()` 元素（Lucide 图标那条链路）：gpui 的 SVG 精灵是先出
//!   alpha 蒙版再用元素色填充的，多色的舷窗（青渐变底 + 琥珀色光标）会被压成
//!   单色剪影，品牌色直接丢掉。所以只能自己出全彩位图。
//! - 现在的做法：SVG 解析一次常驻缓存，出图边长按 rem 逻辑尺寸 × `window.rem_size()`
//!   × `window.scale_factor()` 算出目标设备像素再交给光栅化器。注意
//!   `SvgRenderer::render_parsed` 对 `SvgSize::ScaleFactor` 内部还会再乘一次
//!   `SMOOTH_SVG_SCALE_FACTOR`（当前为 2）并把 `RenderImage.scale_factor` 标成同一个值，
//!   于是纹理实际是目标尺寸的约 2 倍超采样，绘制时只做约 2:1 的降采样——相对原来
//!   512（含 gpui 的 ×2 超采样即 1024）压到 36 设备像素的二十多倍欠采样，细线不再碎裂。
//!   顺带一提，`ImageSource::Render` 是同步就绪的，因此也消掉了异步资产链路在启动
//!   首帧画不出标志的那一闪。
//! - 代价与局限：位图与窗口 DPI 绑定，跨屏拖动改变 `scale_factor` 后要多缓存一档
//!   （键是目标设备像素边长，实际只有 1x/1.25x/1.5x/2x 几档；顶栏 18px@2x 出 36→72px
//!   位图、关于页 48px@2x 出 96→192px 位图，全 RGBA 加起来不到 200KB）；档位随
//!   `HashMap` 累积，不做淘汰——显示器组合是有限的，为这点内存加淘汰逻辑不值得。
//!   解析或光栅化失败时退回内嵌 SVG 的异步资产链路：那条链路就是会糊的老路径，
//!   但顶栏不该因为一次渲染失败而没有品牌标志。

use gpui_kit::*;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// 内嵌的舷窗标志源文件（512x512，品牌青渐变底）。
const LOGO_SVG: &[u8] = include_bytes!("../../../logo/transom-logo.svg");

/// SVG 自身声明的固有边长（px），必须与 `logo/transom-logo.svg` 的 width/height
/// 一致：栅格缩放比是按它算的，改了 SVG 尺寸不改这里，标志会整体缩放错误。
const LOGO_INTRINSIC_PX: f32 = 512.0;

/// 解析结果与各显示尺寸的栅格缓存。
///
/// `ParsedSvg` 已经解析过字体并把文本转成路径，重复解析是纯浪费，因此只留一份；
/// 位图按显示边长分档，同一档内复用同一个 `Arc<RenderImage>`，避免每帧重画。
struct LogoCache {
    parsed: Option<ParsedSvg>,
    rasterized: HashMap<u32, Arc<RenderImage>>,
}

static LOGO_CACHE: OnceLock<Mutex<LogoCache>> = OnceLock::new();

/// 备用链路用的内嵌位图源（只在光栅化失败时使用，按固有尺寸出图）。
static LOGO_FALLBACK: OnceLock<Arc<Image>> = OnceLock::new();

fn cache() -> &'static Mutex<LogoCache> {
    LOGO_CACHE.get_or_init(|| {
        Mutex::new(LogoCache {
            parsed: None,
            rasterized: HashMap::with_capacity(4),
        })
    })
}

/// 按设备像素边长取栅格；未缓存则解析并绘制一次。
///
/// 返回 `None` 表示 SVG 解析或光栅化失败（原因已记 warn），调用方退回备用链路。
fn rasterize(device_px: u32, cx: &App) -> Option<Arc<RenderImage>> {
    let renderer = cx.svg_renderer();
    let mut cache = cache().lock();
    if let Some(image) = cache.rasterized.get(&device_px) {
        return Some(image.clone());
    }

    let parsed = match &mut cache.parsed {
        Some(parsed) => parsed,
        None => match renderer.parse_svg(LOGO_SVG) {
            Ok(parsed) => cache.parsed.insert(parsed),
            Err(error) => {
                log::warn!("解析品牌标志失败，改用内嵌 SVG 资产链路，原因: {error}");
                return None;
            }
        },
    };
    // 比值按"目标设备像素 / SVG 固有像素"给；`render_parsed` 会再乘一次内部
    // 超采样系数（2），得到约 2 倍于目标的多采样板图，绘制时降采样回盒子大小。
    let scale = device_px as f32 / LOGO_INTRINSIC_PX;
    match renderer.render_parsed(parsed, SvgSize::ScaleFactor(scale)) {
        Ok(image) => {
            cache.rasterized.insert(device_px, image.clone());
            Some(image)
        }
        Err(error) => {
            log::warn!("光栅化品牌标志失败，改用内嵌 SVG 资产链路，原因: {error}");
            None
        }
    }
}

/// 备用链路的内嵌标志：走 gpui 的异步图像资产，尺寸固定为 SVG 固有尺寸。
fn fallback_source() -> ImageSource {
    ImageSource::Image(
        LOGO_FALLBACK
            .get_or_init(|| Arc::new(Image::from_bytes(ImageFormat::Svg, LOGO_SVG.to_vec())))
            .clone(),
    )
}

/// 渲染品牌标志：`size_rems` 为逻辑边长（rem，随字号密度档等比缩放）。
///
/// 需要 `window` 只为取两个度量：`rem_size()` 把 rem 换算成逻辑像素，
/// `scale_factor()` 换算成设备像素——出图尺寸必须落在设备像素上才不会糊。
/// 标志自带圆角，这里只做尺寸约束与裁剪，不再叠加 `rounded` 描边。
pub(crate) fn logo_mark(size_rems: f32, window: &Window, cx: &App) -> AnyElement {
    let logical_px = size_rems * window.rem_size().as_f32();
    // 向上取整：宁可留半像素余量，也不要因为截断而比显示盒小一圈（那又是一次缩放采样）。
    let device_px = (logical_px * window.scale_factor()).ceil().max(1.0) as u32;
    let source = rasterize(device_px, cx).map_or_else(fallback_source, ImageSource::Render);

    div()
        .flex_none()
        .size(rems(size_rems))
        .overflow_hidden()
        .child(img(source).w_full().h_full().object_fit(ObjectFit::Contain))
        .into_any_element()
}
