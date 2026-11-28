//! AI 输入框附件与大段粘贴转文档单元测试。

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
            thumb: None,
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
