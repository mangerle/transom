//! 容器文件路径与权限辅助函数。

/// tar 头里的权限位转成常见的 `rwxr-xr-x` 形式，只保留读写执行九位。
pub(crate) fn mode_label(mode: u32) -> String {
    const BITS: [(char, u32); 9] = [
        ('r', 0o400),
        ('w', 0o200),
        ('x', 0o100),
        ('r', 0o040),
        ('w', 0o020),
        ('x', 0o010),
        ('r', 0o004),
        ('w', 0o002),
        ('x', 0o001),
    ];
    let mut text = String::with_capacity(9);
    for (glyph, bit) in BITS {
        text.push(if mode & bit != 0 { glyph } else { '-' });
    }
    text
}

pub(crate) fn parent_path(path: &str) -> String {
    if path == "/" || path.is_empty() {
        return "/".to_string();
    }
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => path[..index].to_string(),
    }
}
