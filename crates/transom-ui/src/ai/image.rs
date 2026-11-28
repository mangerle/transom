//! 多模态对话图片预处理：尺寸缩放、格式规整与 SHA-256 去重。
//!
//! 设计原理（Why）：
//! - 剪贴板截图可能高达 4K 未压缩（几十 MB），而视觉模型普遍在 1568px 左右先缩图；
//! - 在后台线程统一缩放并优先编码为 PNG（超过阈值时退化为 JPEG），同时节省网络带宽与磁盘体积。

use std::io::Cursor;

use base64::Engine as _;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView as _, ImageFormat};
use sha2::{Digest, Sha256};

use transom_ai_contracts::AiError;

/// 模型侧视觉输入的长边最大像素。
pub const MAX_IMAGE_EDGE: u32 = 1568;

/// 单张图片编码后允许的最大字节数（3 MiB）。
pub const MAX_IMAGE_BYTES: usize = 3 * 1024 * 1024;

/// 规整后的对话图片。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedImage {
    pub sha256: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl PreparedImage {
    /// 编码为模型接口需要的 Base64 字符串。
    pub fn to_base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(&self.bytes)
    }

    /// 根据原始字节解码、缩放并规整为 PNG 或 JPEG。
    ///
    /// 注意：编解码属于 CPU 密集操作，应在阻塞线程池调用。
    pub fn prepare(raw_bytes: &[u8]) -> Result<Self, AiError> {
        let img =
            image::load_from_memory(raw_bytes).map_err(|err| AiError::ImageProcessFailed {
                reason: err.to_string(),
            })?;
        let (w, h) = img.dimensions();
        let resized = if w > MAX_IMAGE_EDGE || h > MAX_IMAGE_EDGE {
            img.resize(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE, FilterType::Lanczos3)
        } else {
            img
        };
        let (out_w, out_h) = resized.dimensions();

        let (bytes, media_type) = encode_compact(&resized)?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        Ok(Self {
            sha256,
            media_type,
            bytes,
            width: out_w,
            height: out_h,
        })
    }
}

fn encode_compact(img: &DynamicImage) -> Result<(Vec<u8>, String), AiError> {
    let mut png_buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut png_buf), ImageFormat::Png)
        .map_err(|err| AiError::ImageProcessFailed {
            reason: err.to_string(),
        })?;
    if png_buf.len() <= MAX_IMAGE_BYTES {
        return Ok((png_buf, "image/png".to_owned()));
    }

    let mut jpg_buf = Vec::new();
    img.to_rgb8()
        .write_to(&mut Cursor::new(&mut jpg_buf), ImageFormat::Jpeg)
        .map_err(|err| AiError::ImageProcessFailed {
            reason: err.to_string(),
        })?;
    if jpg_buf.len() > MAX_IMAGE_BYTES {
        return Err(AiError::ImageProcessFailed {
            reason: format!("图片压缩后仍超过上限（{} 字节）", jpg_buf.len()),
        });
    }
    Ok((jpg_buf, "image/jpeg".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::PreparedImage;
    use image::{ImageFormat, RgbImage};
    use std::io::Cursor;

    #[test]
    fn prepare_encodes_small_image_as_png() {
        let img = RgbImage::new(64, 64);
        let mut raw = Vec::new();
        img.write_to(&mut Cursor::new(&mut raw), ImageFormat::Png)
            .expect("构造测试 PNG 失败");
        let prepared = PreparedImage::prepare(&raw).expect("规整测试图片失败");
        assert_eq!(prepared.media_type, "image/png");
        assert_eq!(prepared.width, 64);
        assert!(!prepared.sha256.is_empty());
    }
}
