//! 品牌舷窗标志：把仓库根 `logo/transom-logo.svg` 内嵌进二进制并渲染成顶栏/关于页
//! 使用的标志图。
//!
//! 设计原理（Why）：
//! - 用矢量 SVG 而不是预栅格化的 PNG：顶栏（18px）与"关于"页（48px）尺寸不同，
//!   且密度档会等比缩放 rem，矢量一份即可覆盖全部组合，省掉按 density 备图与
//!   高 DPI 下重新出图的维护成本；GPUI 自带 usvg 光栅化器，`ImageFormat::Svg`
//!   走的是和图片解码同一条资产缓存管线；
//! - `include_bytes!` 直接指向仓库根 `logo/`：该目录是 README 横幅与打包图标的
//!   共同来源，复制一份进 crate 只会让两处标志随时间漂移；
//! - 代价与局限：SVG 按 scale 1.0 软件光栅化后常驻纹理缓存，因此只用于小面积
//!   品牌标志；界面内其余图标仍走 gpui-kit 的 Lucide 矢量资产，不吃这块缓存。

use gpui_kit::*;
use std::sync::{Arc, OnceLock};

/// 内嵌的舷窗标志源文件（512x512，品牌青渐变底）。
const LOGO_SVG: &[u8] = include_bytes!("../../../logo/transom-logo.svg");

/// 进程内共享的标志位图。`ImageSource` 以 `Image::id`（内容哈希）为缓存键，
/// 因此顶栏与关于页必须复用同一个 `Arc`，否则同一份标志会占两份纹理。
static LOGO_IMAGE: OnceLock<Arc<Image>> = OnceLock::new();

/// 构造一次并缓存内嵌标志。
fn logo_image() -> Arc<Image> {
    LOGO_IMAGE
        .get_or_init(|| Arc::new(Image::from_bytes(ImageFormat::Svg, LOGO_SVG.to_vec())))
        .clone()
}

/// 渲染品牌标志：`size` 为显示边长（rem，随字号密度等比缩放）。
///
/// 标志本身已带圆角，这里只做尺寸约束与裁剪，不再叠加 `rounded` 描边。
pub(crate) fn logo_mark(size: f32) -> AnyElement {
    div()
        .flex_none()
        .size(rems(size))
        .overflow_hidden()
        .child(
            img(ImageSource::Image(logo_image()))
                .w_full()
                .h_full()
                .object_fit(ObjectFit::Contain),
        )
        .into_any_element()
}
