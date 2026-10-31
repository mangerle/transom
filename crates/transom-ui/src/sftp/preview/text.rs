//! 文本预览行生成与本地文件头读取。

use super::*;

/// 从文件头部字节生成预览行；返回实际是否还有未展示内容。
pub(crate) fn text_preview_lines(bytes: &[u8], truncated: bool) -> (Vec<String>, bool) {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = Vec::with_capacity(PREVIEW_TEXT_LINES);
    let mut more = truncated;
    for line in text.lines() {
        if lines.len() >= PREVIEW_TEXT_LINES {
            more = true;
            break;
        }
        lines.push(truncate_preview_line(line));
    }
    (lines, more)
}

fn truncate_preview_line(line: &str) -> String {
    let line = line.trim_end_matches('\r');
    let mut characters = line.chars();
    let mut out = String::with_capacity(line.len().min(PREVIEW_LINE_MAX_CHARS * 4));
    for _ in 0..PREVIEW_LINE_MAX_CHARS {
        match characters.next() {
            Some('\t') => out.push_str("    "),
            Some(character) if character.is_control() => out.push(' '),
            Some(character) => out.push(character),
            None => return out,
        }
    }
    if characters.next().is_some() {
        out.push('…');
    }
    out
}

/// 读取本地文本文件的头部；多读 1 字节判断是否发生截断。
pub(crate) fn read_local_text_head(
    path: &Path,
    max_bytes: u64,
) -> std::io::Result<(Vec<u8>, bool)> {
    use std::io::Read;

    let file = std::fs::File::open(path)?;
    let capacity = usize::try_from(max_bytes).unwrap_or(0);
    let mut buffer = Vec::with_capacity(capacity);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut buffer)?;
    let truncated = u64::try_from(buffer.len()).unwrap_or(u64::MAX) > max_bytes;
    if truncated {
        buffer.truncate(capacity);
    }
    Ok((buffer, truncated))
}
