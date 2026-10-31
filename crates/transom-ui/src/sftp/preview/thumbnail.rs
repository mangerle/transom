//! 图片缩略图解码。

use super::*;

/// 解码图片并生成适合卡片展示的缩略图。
///
/// 设计原理（Why）：
/// - 实现初衷：SFTP 没有缩略图接口，远端只能传回完整文件；若把原图直接交给
///   渲染器解码，一张高分辨率照片可能解出数百 MB 的像素缓冲；
/// - 核心优势：先在解码前读取图片头校验像素数，再在后台线程解码并缩放到
///   卡片所需的 2 倍尺寸，UI 线程只接收这张小图；EXIF 方向在缩放前应用，
///   竖拍照片不会横着显示；
/// - 代价与局限：解码阶段仍会短暂持有原图像素缓冲（峰值由像素上限约束）；
///   动图只取首帧，缩略图不播放动画。
pub(crate) fn decode_preview_thumbnail(bytes: &[u8]) -> Result<Arc<RenderImage>, String> {
    decode_thumbnail(
        bytes,
        PREVIEW_IMAGE_MAX_PIXELS,
        PREVIEW_THUMBNAIL_MAX_WIDTH,
        PREVIEW_THUMBNAIL_MAX_HEIGHT,
    )
}

pub(crate) fn decode_thumbnail(
    bytes: &[u8],
    max_pixels: u64,
    max_width: u32,
    max_height: u32,
) -> Result<Arc<RenderImage>, String> {
    use image::ImageDecoder as _;

    let dimensions = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| {
            rust_i18n::t!("sftp.preview.unrecognized_format", error = error).to_string()
        })?
        .into_dimensions()
        .map_err(|error| {
            rust_i18n::t!("sftp.preview.read_dimensions_failed", error = error).to_string()
        })?;
    let pixels = u64::from(dimensions.0).saturating_mul(u64::from(dimensions.1));
    if pixels > max_pixels {
        return Err(rust_i18n::t!(
            "sftp.preview.too_many_pixels",
            width = dimensions.0,
            height = dimensions.1,
            pixels = pixels,
            max = max_pixels
        )
        .to_string());
    }

    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| {
            rust_i18n::t!("sftp.preview.unrecognized_format", error = error).to_string()
        })?;
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| rust_i18n::t!("sftp.preview.decoder_failed", error = error).to_string())?;
    // 方向信息缺失时按不旋转处理，宁可保持原始方向也不要让预览失败。
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = image::DynamicImage::from_decoder(decoder)
        .map_err(|error| rust_i18n::t!("sftp.preview.decode_failed", error = error).to_string())?;
    image.apply_orientation(orientation);

    let image = if image.width() <= max_width && image.height() <= max_height {
        image
    } else {
        image.thumbnail(max_width, max_height)
    };
    let mut rgba = image.into_rgba8();
    // gpui 的 RenderImage 约定 BGRA；与内置解码器保持一致的通道顺序。
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok(Arc::new(RenderImage::new([image::Frame::new(rgba)])))
}
