//! 国际化守卫：UI 源码不得再出现硬编码中文文案。

use crate::common::ui_source_files;

// ---------------------------------------------------------------------------
// 国际化守卫：UI 源码不得再出现硬编码中文文案
// ---------------------------------------------------------------------------

/// 不面向界面的调用：日志、断言、内部诊断。这些位置的字符串先整体剔除，
/// 避免用逐条白名单维护；剔除后仍出现中文，才视为界面文案漏网。
const SILENT_CALLS: &[&str] = &[
    "log::trace!(",
    "log::debug!(",
    "log::info!(",
    "log::warn!(",
    "log::error!(",
    "tracing::trace!(",
    "tracing::debug!(",
    "tracing::info!(",
    "tracing::warn!(",
    "tracing::error!(",
    "trace!(",
    "debug!(",
    "info!(",
    "warn!(",
    "error!(",
    "println!(",
    "eprintln!(",
    "panic!(",
    "debug_assert!(",
    "debug_assert_eq!(",
    "debug_assert_ne!(",
    "assert!(",
    "assert_eq!(",
    "assert_ne!(",
    "AppError::with_detail(",
    "io::Error::new(",
    "expect(",
    "#[error(",
];

/// 参与守卫扫描的界面源码：主界面与终端渲染层。

#[test]
fn ui_sources_have_no_hardcoded_chinese() {
    let mut offenders = Vec::new();
    for (path, content) in ui_source_files() {
        for (index, literal) in chinese_literals(&content) {
            if allowlisted_literal(&path, &literal) {
                continue;
            }
            offenders.push(format!("{path}:{index}: {literal}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "UI 源码仍有硬编码中文文案，请改用 rust_i18n::t!()：\n{}",
        offenders.join("\n")
    );
}

/// 有意保留的中文：内部比较标记、内部诊断与语言名，不参与界面语言切换。
///
/// 这几条都不是界面文案：`本地/远端` 是属性弹窗的来源判别值，`校验` 用于识别
/// 校验失败以强制重传，`连接已断开` 与 `权限` 参与错误分支判定（诊断详情只在
/// 中文界面展示），`上报 shell` 只拼进日志；语言名按自身语言书写。
fn allowlisted_literal(path: &str, literal: &str) -> bool {
    let literal = literal.trim();
    (literal == "本地"
        && (path.ends_with("sftp/properties.rs")
            || path.ends_with("sftp/modals.rs")
            || path.ends_with("sftp/modals/properties.rs")))
        || (literal == "远端" && path.ends_with("sftp/handlers.rs"))
        || (literal == "中文" && path.ends_with("settings/general_page.rs"))
        || (literal == "校验"
            && (path.ends_with("sftp/queue_actions/mod.rs")
                || path.ends_with("sftp/queue_actions/task_control.rs")))
        || (literal == "连接已断开" && path.ends_with("connection/mod.rs"))
        || (literal == "权限" && path.ends_with("editor/banner.rs"))
        || (literal == "，上报 shell: {shell}" && path.ends_with("shell_integration.rs"))
        || path.ends_with("container/hub/tags.rs")
        || path.ends_with("container/detail/image_history.rs")
        || path.ends_with("container/types/run_form/infer.rs")
        || path.ends_with("container/compose_templates.rs")
}

/// 剔除测试模块、注释与静默调用后，返回剩余的中文字面量（行号从 1 起）。
fn chinese_literals(src: &str) -> Vec<(usize, String)> {
    let production = blank_test_modules(src);
    let code = strip_silent_regions(&production);
    let mut out = Vec::new();
    for (index, line) in code.lines().enumerate() {
        for literal in string_literals(line) {
            if literal.chars().any(is_cjk) {
                out.push((index + 1, literal));
            }
        }
    }
    out
}

/// 把 `#[cfg(test)] mod tests;` 与内联测试模块替换为空格，保留行号。
///
/// 不能简单地从第一个 `#[cfg(test)]` 截断：不少模块把 `mod tests;` 声明在
/// 文件顶部，截断会把其后全部生产代码漏掉，硬编码文案因此逃过扫描。
fn blank_test_modules(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < bytes.len() {
        if src[i..].starts_with("#[cfg(test)]") && !inside_ident(bytes, i) {
            let mut j = i + "#[cfg(test)]".len();
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if src[j..].starts_with("mod ") {
                let mut k = j;
                while k < bytes.len() && bytes[k] != b'{' && bytes[k] != b';' {
                    k += 1;
                }
                let end = if k < bytes.len() && bytes[k] == b'{' {
                    skip_balanced(bytes, k)
                } else {
                    (k + 1).min(bytes.len())
                };
                blank_range(&src[i..end], &mut out);
                i = end;
                continue;
            }
        }
        let len = src[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&src[i..i + len]);
        i += len;
    }
    out
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

/// 把注释与静默调用替换为空格，保留换行以维持行号。
fn strip_silent_regions(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            let end = src[i..].find('\n').map_or(bytes.len(), |offset| i + offset);
            blank_range(&src[i..end], &mut out);
            i = end;
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let end = skip_block_comment(bytes, i);
            blank_range(&src[i..end], &mut out);
            i = end;
            continue;
        }
        if !inside_ident(bytes, i)
            && let Some(name) = SILENT_CALLS
                .iter()
                .find(|name| src[i..].starts_with(**name))
        {
            let open = i + name.len() - 1;
            let end = skip_balanced(bytes, open);
            blank_range(&src[i..end], &mut out);
            i = end;
            continue;
        }
        if bytes[i] == b'"' {
            let end = skip_string(bytes, i);
            out.push_str(&src[i..end]);
            i = end;
            continue;
        }
        if bytes[i] == b'\'' && looks_like_char(bytes, i) {
            let end = skip_char(bytes, i);
            out.push_str(&src[i..end]);
            i = end;
            continue;
        }
        let len = src[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&src[i..i + len]);
        i += len;
    }
    out
}

fn inside_ident(bytes: &[u8], index: usize) -> bool {
    index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_')
}

fn blank_range(range: &str, out: &mut String) {
    for c in range.chars() {
        out.push(if c == '\n' { '\n' } else { ' ' });
    }
}

fn skip_string(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn looks_like_char(bytes: &[u8], start: usize) -> bool {
    match bytes.get(start + 1) {
        Some(b'\\') => bytes.get(start + 3) == Some(&b'\''),
        Some(_) => bytes.get(start + 2) == Some(&b'\''),
        None => false,
    }
}

fn skip_char(bytes: &[u8], start: usize) -> usize {
    if bytes.get(start + 1) == Some(&b'\\') {
        (start + 4).min(bytes.len())
    } else {
        (start + 3).min(bytes.len())
    }
}

fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut depth = 1;
    let mut i = start + 2;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

/// 从 `(` 开始跳到配对的 `)` 之后；字符串、字符与注释里的括号不计入深度。
fn skip_balanced(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i = bytes[i..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(bytes.len(), |offset| i + offset);
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i = skip_block_comment(bytes, i);
            continue;
        }
        if bytes[i] == b'"' {
            i = skip_string(bytes, i);
            continue;
        }
        if bytes[i] == b'\'' && looks_like_char(bytes, i) {
            i = skip_char(bytes, i);
            continue;
        }
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// 单行内的双引号字符串字面量；转义引号不计为结束。
fn string_literals(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'"' {
            i += 1;
            continue;
        }
        let mut value = String::new();
        i += 1;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => {
                    if let Some(&next) = bytes.get(i + 1) {
                        value.push(next as char);
                    }
                    i += 2;
                }
                b'"' => {
                    i += 1;
                    break;
                }
                _ => {
                    let len = line[i..].chars().next().map_or(1, char::len_utf8);
                    value.push_str(&line[i..i + len]);
                    i += len;
                }
            }
        }
        out.push(value);
    }
    out
}

/// 页面层字号也必须使用 rem/token；`text_size(px(...))` 会绕过密度缩放。

#[test]
fn test_module_declaration_does_not_hide_production_literals() {
    let src = "#[cfg(test)]\nmod tests;\n\nfn message() -> &'static str {\n    \"未翻译文案\"\n}\n";
    let found = chinese_literals(src);
    assert!(
        found.iter().any(|(_, literal)| literal == "未翻译文案"),
        "mod tests; 之后的生产代码必须继续参与扫描"
    );
}

/// 内联测试模块里的中文（断言/测试数据）不参与扫描，但模块之后的生产代码
/// 仍要被扫描。
#[test]
fn inline_test_module_is_ignored_but_following_code_is_scanned() {
    let src = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn case() { let _ = \"测试中文\"; }\n}\n\nfn message() -> &'static str {\n    \"生产中文\"\n}\n";
    let found = chinese_literals(src);
    assert!(
        !found.iter().any(|(_, literal)| literal == "测试中文"),
        "内联测试模块内的中文不应参与扫描"
    );
    assert!(
        found.iter().any(|(_, literal)| literal == "生产中文"),
        "内联测试模块之后的生产代码必须继续参与扫描"
    );
}
