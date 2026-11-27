//! AI 输入区多模态附件：长文本自动转文档、本地文件流式限长读取与图片缩放编码。

use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::{ClipboardEntry, ClipboardItem};
use gpui_kit::*;
use transom_ai::PreparedImage;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 单次输入增量超过此字节数且总长度/行数达标时，自动折叠为文档附件。
pub(super) const AUTO_DOC_MIN_DELTA: usize = 600;
const AUTO_DOC_MIN_CHARS: usize = 1_200;
const AUTO_DOC_MIN_LINES: usize = 15;
const MAX_TEXT_ATTACHMENT_BYTES: u64 = 256 * 1024;
const MAX_IMAGE_RAW_BYTES: u64 = 20 * 1024 * 1024;

/// 输入框待发送的附件条目。
#[derive(Clone, Debug)]
pub(crate) enum ComposerAttachment {
    PastedDoc {
        id: String,
        title: String,
        content: String,
        lines: usize,
        chars: usize,
    },
    File {
        id: String,
        name: String,
        content: String,
        bytes: usize,
    },
    Image {
        id: String,
        name: String,
        media_type: String,
        base64: String,
        width: u32,
        height: u32,
    },
}

impl ComposerAttachment {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::PastedDoc { id, .. } | Self::File { id, .. } | Self::Image { id, .. } => id,
        }
    }

    fn chip_label(&self) -> String {
        match self {
            Self::PastedDoc {
                title,
                lines,
                chars,
                ..
            } => format!("{title} · {lines}L / {chars}c"),
            Self::File { name, bytes, .. } => format!("{name} · {} KB", (*bytes).div_ceil(1024)),
            Self::Image {
                name,
                width,
                height,
                ..
            } => format!("{name} · {width}x{height}"),
        }
    }

    fn icon(&self) -> AppIcon {
        match self {
            Self::PastedDoc { .. } | Self::File { .. } => AppIcon::Ui(UiIcon::LIST_FILES),
            Self::Image { .. } => AppIcon::Custom(CustomIcon::IMAGE),
        }
    }
}

/// 判断本次输入变化是否属于大段文本粘贴，满足条件时自动转为文档附件。
pub(super) fn should_convert_paste_to_doc(prev_len: usize, new_text: &str) -> bool {
    let delta = new_text.len().saturating_sub(prev_len);
    if delta < AUTO_DOC_MIN_DELTA {
        return false;
    }
    new_text.chars().count() >= AUTO_DOC_MIN_CHARS || new_text.lines().count() >= AUTO_DOC_MIN_LINES
}

fn is_image_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
            )
        })
        .unwrap_or(false)
}

/// 在后台线程池将剪贴板原始图片字节缩放编码为图片附件。
pub(super) fn load_attachment_from_image_bytes(
    bytes: &[u8],
    seq: usize,
) -> Result<ComposerAttachment, String> {
    let name = format!("image-{seq}.png");
    let prepared = PreparedImage::prepare(bytes).map_err(|err| format!("{name}: {err}"))?;
    Ok(ComposerAttachment::Image {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        media_type: prepared.media_type.clone(),
        base64: prepared.to_base64(),
        width: prepared.width,
        height: prepared.height,
    })
}

/// 在后台线程池安全读取本地文件或图片为附件（严格限制读取上限，防止 OOM）。
pub(super) fn load_attachment_from_path(path: &Path) -> Result<ComposerAttachment, String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("attachment")
        .to_owned();
    let id = uuid::Uuid::new_v4().to_string();
    let file = File::open(path).map_err(|err| format!("{name}: {err}"))?;

    if is_image_ext(path) {
        let mut buf = Vec::new();
        file.take(MAX_IMAGE_RAW_BYTES)
            .read_to_end(&mut buf)
            .map_err(|err| format!("{name}: {err}"))?;
        let prepared = PreparedImage::prepare(&buf).map_err(|err| format!("{name}: {err}"))?;
        return Ok(ComposerAttachment::Image {
            id,
            name,
            media_type: prepared.media_type.clone(),
            base64: prepared.to_base64(),
            width: prepared.width,
            height: prepared.height,
        });
    }

    let mut buf = Vec::new();
    file.take(MAX_TEXT_ATTACHMENT_BYTES)
        .read_to_end(&mut buf)
        .map_err(|err| format!("{name}: {err}"))?;
    let nul_count = buf.iter().filter(|&&b| b == 0).count();
    if nul_count > 4 {
        return Err(rust_i18n::t!("ai.attachment_binary_unsupported", name = name).to_string());
    }
    let content = String::from_utf8_lossy(&buf).into_owned();
    let bytes = buf.len();
    Ok(ComposerAttachment::File {
        id,
        name,
        content,
        bytes,
    })
}

/// 打包后的待发送多模态附件数据。
pub(super) struct PackedAttachments {
    pub(super) images: Vec<(String, String)>,
    pub(super) text: Option<(String, String)>,
}

/// 将所有待发送附件打包为 `images_base64` `(media_type, base64)` 与 `attachment_text` `(title, content)`。
pub(super) fn pack_attachments_for_request(
    attachments: &[ComposerAttachment],
) -> PackedAttachments {
    let mut images = Vec::new();
    let mut docs = Vec::new();
    let mut titles = Vec::new();
    for att in attachments {
        match att {
            ComposerAttachment::Image {
                media_type, base64, ..
            } => images.push((media_type.clone(), base64.clone())),
            ComposerAttachment::PastedDoc { title, content, .. } => {
                titles.push(title.clone());
                docs.push(format!("--- {title} ---\n{content}"));
            }
            ComposerAttachment::File { name, content, .. } => {
                titles.push(name.clone());
                docs.push(format!("--- {name} ---\n{content}"));
            }
        }
    }
    let text = (!docs.is_empty()).then(|| (titles.join(", "), docs.join("\n\n")));
    PackedAttachments { images, text }
}

/// 在输入框上方渲染待发送附件卡片预览条。
pub(super) fn render_attachment_chips(
    attachments: &[ComposerAttachment],
    p: &Palette,
    on_remove: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let mut row = h_flex()
        .w_full()
        .px(rems(space::SM))
        .pt(rems(space::XS))
        .gap(rems(space::XS))
        .flex_wrap();
    for (idx, att) in attachments.iter().enumerate() {
        let id = att.id().to_owned();
        let remove = on_remove.clone();
        let remove_label = rust_i18n::t!("ai.attachment_remove").to_string();
        let chip = h_flex()
            .px(rems(space::XS))
            .py(rems(space::XXS))
            .gap(rems(space::XS))
            .items_center()
            .rounded(p.radius.md)
            .bg(p.secondary)
            .border_1()
            .border_color(p.divider(p.border))
            .child(
                div()
                    .p(px(4.0))
                    .rounded(p.radius.sm)
                    .bg(p.surface.floating)
                    .text_color(p.primary)
                    .child(AppIconElement::new(att.icon()).small()),
            )
            .child(
                v_flex().gap(px(1.0)).child(
                    div()
                        .max_w(px(140.0))
                        .truncate()
                        .text_size(rems(typography::CAPTION))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.foreground)
                        .child(att.chip_label()),
                ),
            )
            .child(
                Button::new(("ai-att-rm", idx))
                    .ghost()
                    .accessibility_label(remove_label)
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).small())
                    .on_click(move |_, w, cx| remove(id.clone(), w, cx)),
            );
        row = row.child(chip);
    }
    row.into_any_element()
}

impl AiAssistantView {
    /// 处理粘贴事件：若剪贴板包含图片或文件则异步转为附件并拦截，纯文本放行。
    pub(super) fn handle_paste_item(
        &mut self,
        item: &ClipboardItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut image_payloads: Vec<Vec<u8>> = Vec::new();
        let mut path_payloads: Vec<PathBuf> = Vec::new();
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(img) => image_payloads.push(img.bytes.clone()),
                ClipboardEntry::ExternalPaths(ext) => {
                    path_payloads.extend(ext.paths().iter().cloned());
                }
                ClipboardEntry::String(_) => {}
            }
        }
        if image_payloads.is_empty() && path_payloads.is_empty() {
            return false;
        }
        let seq_base = self.state.pending_attachments.len() + 1;
        let bg = cx.background_executor().clone();
        self._attachment_task = Some(cx.spawn_in(window, async move |this, cx| {
            let loaded = bg
                .spawn(async move {
                    let mut out = Vec::new();
                    for (i, bytes) in image_payloads.into_iter().enumerate() {
                        out.push(load_attachment_from_image_bytes(&bytes, seq_base + i));
                    }
                    for path in path_payloads {
                        out.push(load_attachment_from_path(&path));
                    }
                    out
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                for res in loaded {
                    match res {
                        Ok(att) => this.state.pending_attachments.push(att),
                        Err(err) => this.state.status_error = Some(err),
                    }
                }
                cx.notify();
            });
        }));
        true
    }

    /// 处理拖入输入区的文件与图片。
    pub(super) fn handle_dropped_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let bg = cx.background_executor().clone();
        self._attachment_task = Some(cx.spawn_in(window, async move |this, cx| {
            let loaded = bg
                .spawn(async move {
                    paths
                        .into_iter()
                        .map(|p| load_attachment_from_path(&p))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                for res in loaded {
                    match res {
                        Ok(att) => this.state.pending_attachments.push(att),
                        Err(err) => this.state.status_error = Some(err),
                    }
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn remove_attachment(&mut self, id: &str, cx: &mut Context<Self>) {
        self.state.pending_attachments.retain(|a| a.id() != id);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{ComposerAttachment, pack_attachments_for_request, should_convert_paste_to_doc};

    #[test]
    fn converts_large_paste_to_doc_and_ignores_normal_typing() {
        assert!(!should_convert_paste_to_doc(0, "ls -la"));
        let long_single_line = "a".repeat(1300);
        assert!(should_convert_paste_to_doc(0, &long_single_line));
        let multi_line = (0..20)
            .map(|i| format!("line-{i}: some log output padding text here"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(should_convert_paste_to_doc(0, &multi_line));
        // 若只是在已有长文后敲入 1 个字符，不重复触发转换
        assert!(!should_convert_paste_to_doc(
            long_single_line.len(),
            &format!("{long_single_line}b")
        ));
    }

    #[test]
    fn packs_images_and_text_attachments_separately() {
        let items = vec![
            ComposerAttachment::PastedDoc {
                id: "1".to_owned(),
                title: "粘贴文档 #1".to_owned(),
                content: "hello".to_owned(),
                lines: 1,
                chars: 5,
            },
            ComposerAttachment::Image {
                id: "2".to_owned(),
                name: "err.png".to_owned(),
                media_type: "image/png".to_owned(),
                base64: "AAAA".to_owned(),
                width: 100,
                height: 100,
            },
        ];
        let packed = pack_attachments_for_request(&items);
        assert_eq!(
            packed.images,
            vec![("image/png".to_owned(), "AAAA".to_owned())]
        );
        let (title, body) = packed.text.expect("应包含文本附件");
        assert!(title.contains("粘贴文档 #1"));
        assert!(body.contains("粘贴文档 #1"));
        assert!(body.contains("hello"));
    }
}
