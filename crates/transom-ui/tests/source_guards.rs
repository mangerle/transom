//! 源码结构守卫：把架构约定固化成对源码的静态检查。
//!
//! # 设计原理（Why）
//! 这些检查与主题无关，放在 `theme` 单元测试里会让主题模块承担全 crate 的
//! 架构守卫职责；移到集成测试后，主题测试只保留主题契约，源码扫描集中在一处。
//!
//! # 代价与局限
//! 文本扫描只能防住“写法恰好被匹配”的回归，不能替代类型约束。字段可见性
//! 与实体边界最终应由类型系统和模块私有性保证；本文件是迁移期的防线。

use std::path::{Path, PathBuf};

/// 简易目录递归（避免为此引入 walkdir 依赖）。
fn walkdir_like(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
    }
    out
}

/// 统一使用 `/` 作为源码路径分隔符，避免 Windows 的反斜杠使后缀白名单失效。
fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn source_guard_paths_use_forward_slashes() {
    assert_eq!(
        normalized_path(Path::new(r"C:\repo\transom-ui\src\settings\mod.rs")),
        "C:/repo/transom-ui/src/settings/mod.rs"
    );
}

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
fn ui_source_files() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for base in ["crates/transom-ui/src", "crates/transom-terminal/src"] {
        for entry in walkdir_like(&root.join(base)) {
            let path = normalized_path(&entry);
            if path.ends_with("tests.rs") {
                continue;
            }
            if entry.extension().and_then(|e| e.to_str()) == Some("rs")
                && let Ok(content) = std::fs::read_to_string(&entry)
            {
                out.push((path, content));
            }
        }
    }
    out
}

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
        || (literal == "远端" && path.ends_with("sftp_handlers.rs"))
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

/// 组件层必须走设计令牌：字号、间距、控件尺寸与弹窗宽度不能在这里重新写
/// `text_xs()` / `px(...)`，否则页面迁移到组件层后仍会随密度切换失调。
#[test]
fn widgets_use_design_tokens() {
    let mut offenders = Vec::new();
    for (path, content) in ui_source_files() {
        if !path.contains("/widgets/") {
            continue;
        }
        for (index, line) in content.lines().enumerate() {
            let magic_px = line
                .match_indices("px(")
                .any(|(index, _)| !line[index + 3..].starts_with("rems("));
            if line.contains("text_xs(")
                || line.contains("text_sm(")
                || magic_px
                || line.contains("gap_1(")
                || line.contains("gap_2(")
                || line.contains("gap_3(")
                || line.contains("gap_4(")
            {
                offenders.push(format!("{path}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "widgets 组件必须使用 theme::tokens 中的字号/间距/控件令牌：\n{}",
        offenders.join("\n")
    );
}

/// 菜单项必须走 widgets 构造器：`PopupMenuItem::new(...)` 紧随 `.icon(...)`
/// 是迁移前的写法，散落回页面会让图标顺序与禁用语义各自漂移。
///
/// 例外只保留两处真实差异：带真实 `Action` 的条目（动作自带键位展示）与下拉
/// 里的 `.checked(...)` 选中项，它们不属于同一语义。
#[test]
fn menu_items_use_widget_constructor() {
    let mut offenders = Vec::new();
    for (path, content) in ui_source_files() {
        // 只守卫 transom-ui：transom-terminal 不能反向依赖 UI 组件层。
        if path.contains("/widgets/") || !path.contains("/transom-ui/src/") {
            continue;
        }
        let lines: Vec<&str> = content.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains("PopupMenuItem::new(") && !line.contains("PopupMenuItem::new()") {
                continue;
            }
            let window = lines
                .iter()
                .skip(index)
                .take(3)
                .any(|next| next.trim_start().starts_with(".icon("));
            if window {
                offenders.push(format!("{}:{}: {}", path, index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "菜单项请使用 crate::widgets::menu_item / MenuItem：\n{}",
        offenders.join("\n")
    );
}

/// 外壳只能持有状态容器一个字段：`AppShell` 再平铺回字段就意味着功能模块又能
/// 通过兄弟路径直接读写全局状态，封装会重新退化。
#[test]
fn app_shell_keeps_single_state_field() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/mod.rs");
    let content = std::fs::read_to_string(&path).expect("读取 app/mod.rs 失败");
    let start = content
        .find("pub(crate) struct AppShell {")
        .expect("未找到 AppShell 定义");
    // Windows 检出可能使用 CRLF；只匹配换行后的闭合花括号，避免把行尾格式
    // 当作结构错误。
    let end = content[start..].find("\n}").expect("AppShell 定义未闭合") + start;
    let body = &content[start..end];
    let fields: Vec<&str> = body
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//") && line.ends_with(','))
        .collect();
    assert_eq!(
        fields,
        vec!["state: AppState,"],
        "AppShell 只允许持有 state 一个字段，新增状态请放进 AppState 或 feature 结构体"
    );
}

/// 监控面板的渲染与面板状态必须归属 `MonitorView` 实体：`monitor/` 下只允许
/// `ops.rs` 保留 `impl AppShell`（采样调度、任务生命周期与跨域动作回交外壳）。
/// 一旦渲染方法重新散落到外壳，说明实体化被回退了。
#[test]
fn monitor_rendering_belongs_to_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/monitor");
    let mut offenders = Vec::new();
    for (path, content) in walkdir_like(&root)
        .into_iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            Some((normalized_path(&path), content))
        })
    {
        if path.ends_with("ops.rs") {
            continue;
        }
        if content.contains("impl AppShell") {
            offenders.push(path);
        }
    }
    assert!(
        offenders.is_empty(),
        "监控渲染必须以 impl MonitorView 实现，不得挂在 AppShell 上：\n{}",
        offenders.join("\n")
    );
}

/// SFTP 视图不得再实现 `Deref`/`DerefMut`：字段访问必须显式写
/// `view.state.xxx`。迁移期的透明访问会让"状态归属"在调用点消失，也给
/// 后续同名字段埋下静默遮蔽。
#[test]
fn sftp_view_has_no_deref() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sftp/entity/mod.rs");
    let content = std::fs::read_to_string(&path).expect("读取 sftp/entity/mod.rs 失败");
    assert!(
        !content.contains("impl std::ops::Deref"),
        "SftpPanelView 不得再实现 Deref/DerefMut，请显式访问 view.state"
    );
}

/// 设置弹窗的页面渲染、保存与取消必须归 `SettingsView`：`settings/` 下只允许
/// `handler.rs`（跨域事件消费）与 `mod.rs`（通用控件与外观下发）保留
/// `impl AppShell`。页面一旦重新挂回外壳，设置实体化就被回退了。
#[test]
fn settings_rendering_belongs_to_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/settings");
    let mut offenders = Vec::new();
    for (path, content) in walkdir_like(&root)
        .into_iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            Some((normalized_path(&path), content))
        })
    {
        if path.ends_with("handler.rs") || path.ends_with("settings/mod.rs") {
            continue;
        }
        for pattern in ["impl AppShell", "settings_modal(", "settings_modal_mut("] {
            if content.contains(pattern) {
                offenders.push(format!("{path}: {pattern}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "设置页面必须挂在 SettingsView 上，跨域动作走 SettingsEvent：\n{}",
        offenders.join("\n")
    );
}

/// 编辑面板也不得再有 `with_shell` / `read_shell` 同步回交通道。
#[test]
fn editor_panel_has_no_sync_callback_channel() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/editor");
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        if content.contains("with_shell(") || content.contains("read_shell(") {
            offenders.push(normalized_path(&entry));
        }
    }
    assert!(
        offenders.is_empty(),
        "编辑面板不得再使用 with_shell / read_shell 同步回交外壳：\n{}",
        offenders.join("\n")
    );
}

/// 所有面板实体都不得持有外壳句柄。
///
/// 数据由外壳注入、意图用事件上报，面板不再有同步回到外壳的路径；
/// 这条结构约束同时消灭"双向调用导致双重租借"的可能。
#[test]
fn panel_entities_have_no_parent_entity_handle() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for base in ["monitor", "editor", "forward", "sftp"] {
        for entry in walkdir_like(&root.join(base)) {
            if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&entry) else {
                continue;
            };
            // 只在结构体字段声明里检查：注释与文档提及不算违规。
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") {
                    continue;
                }
                if trimmed.contains("WeakEntity<AppShell>") {
                    offenders.push(format!("{}: {}", entry.display(), trimmed));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "面板实体不得持有 WeakEntity<AppShell>：\n{}",
        offenders.join("\n")
    );
}

/// 外壳处理面板事件时不得回读该面板。
///
/// 事件派发阶段面板仍被租借，处理函数里再 `read`/`update` 会触发
/// `cannot read/update ... while it is already being updated`。各面板的处理函数
/// 统一命名为 `on_*_panel_event`，这里扫描它们的函数体。
#[test]
fn panel_event_handlers_do_not_read_back() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for base in ["monitor", "editor", "forward", "sftp"] {
        for entry in walkdir_like(&root.join(base)) {
            if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&entry) else {
                continue;
            };
            let Some(start) = content.find("fn on_") else {
                continue;
            };
            let name_start = start + "fn ".len();
            let Some(name_end) = content[name_start..].find('(') else {
                continue;
            };
            let name = &content[name_start..name_start + name_end];
            if !name.ends_with("panel_event") {
                continue;
            }
            let Some(body_start) = content[name_start..].find('{') else {
                continue;
            };
            let body = &content[name_start + body_start..];
            let mut depth = 0usize;
            let mut end = 0usize;
            for (offset, ch) in body.char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = offset;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let body = &body[..end];
            for pattern in ["sftp_view(", "sftp_state(", "settings_view(", ".read(cx)"] {
                if body.contains(pattern) {
                    offenders.push(format!("{name}: {pattern} @ {}", entry.display()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "面板事件处理函数不得回读面板：\n{}",
        offenders.join("\n")
    );
}

/// Git 工作区必须由实体持有：标签内容存 `Entity<GitWorkspace>`，旧的可变
/// 引用访问器不得复活。实体化后动作与状态同处，状态生命周期随标签页。
#[test]
fn git_workspace_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let types = std::fs::read_to_string(root.join("src/app/types.rs")).expect("读取 tags 类型失败");
    assert!(
        types.contains("Git(Entity<crate::git::GitWorkspace>)"),
        "TabContent::Git 必须持有 Entity<GitWorkspace>"
    );
    let state = std::fs::read_to_string(root.join("src/app/state.rs")).expect("读取 AppState 失败");
    for forbidden in [
        "fn git_state_of_tab(",
        "fn git_state_of_tab_mut(",
        "fn git_state_of(",
        "fn git_state_of_mut(",
    ] {
        assert!(
            !state.contains(forbidden) && !types.contains(forbidden),
            "旧的 Git 引用访问器不得复活：{forbidden}"
        );
    }
    let entity =
        std::fs::read_to_string(root.join("src/git/entity.rs")).expect("读取 Git 实体失败");
    assert!(
        entity.contains("pub(crate) struct GitWorkspace"),
        "GitWorkspace 实体必须存在于 git/entity.rs"
    );
}

/// 容器工作区必须由实体持有：标签内容存 `Entity<ContainerWorkspace>`，旧的
/// 可变引用访问器不得复活。实体化后状态生命周期随标签页。
#[test]
fn container_workspace_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let types = std::fs::read_to_string(root.join("src/app/types.rs")).expect("读取标签类型失败");
    assert!(
        types.contains("Containers(Entity<crate::container::ContainerWorkspace>)"),
        "TabContent::Containers 必须持有 Entity<ContainerWorkspace>"
    );
    let container = std::fs::read_to_string(root.join("src/container/mod.rs"))
        .expect("读取 container/mod.rs 失败");
    assert!(
        !container.contains("fn container_workspace_mut("),
        "旧的 container_workspace_mut 不得复活"
    );
    let entity =
        std::fs::read_to_string(root.join("src/container/entity.rs")).expect("读取容器实体失败");
    assert!(
        entity.contains("pub(crate) struct ContainerWorkspace"),
        "ContainerWorkspace 实体必须存在于 container/entity.rs"
    );
}

/// 已知主机弹窗必须由实体持有，旧的 `AppState` 刷新入口不得复活。
#[test]
fn known_hosts_modal_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("known_hosts: Entity<KnownHostsView>"),
        "ModalHub 的 known_hosts 必须持有 Entity<KnownHostsView>"
    );
    let view = std::fs::read_to_string(root.join("src/keys/known_hosts_view.rs"))
        .expect("读取已知主机实体失败");
    assert!(
        view.contains("pub(crate) struct KnownHostsView"),
        "KnownHostsView 实体必须存在于 keys/known_hosts_view.rs"
    );
    let shell = std::fs::read_to_string(root.join("src/known_hosts_modal.rs"))
        .expect("读取已知主机视图失败");
    assert!(
        !shell.contains("fn refresh_known_hosts(")
            && !shell.contains("fn render_known_hosts_body("),
        "已知主机的旧外壳入口不得复活"
    );
}

/// 备份弹窗必须由实体持有：状态、在途任务与渲染都归 `BackupPromptView`，
/// 旧的外壳提交入口与 `TaskHub` 任务槽不得复活。外壳只消费事件做目录合并。
#[test]
fn backup_prompt_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("backup_prompt: Entity<BackupPromptView>"),
        "ModalHub 的 backup_prompt 必须持有 Entity<BackupPromptView>"
    );
    let task_hub =
        std::fs::read_to_string(root.join("src/app/task_hub.rs")).expect("读取 task_hub.rs 失败");
    assert!(
        !task_hub.contains("backup: Option<Task<()>>"),
        "备份任务句柄必须归实体，TaskHub 不得保留 backup 槽位"
    );
    let view = std::fs::read_to_string(root.join("src/backup/view.rs")).expect("读取备份实体失败");
    assert!(
        view.contains("pub(crate) struct BackupPromptView")
            && view.contains("task: Option<Task<()>>"),
        "BackupPromptView 必须在 backup/view.rs 持有状态与任务句柄"
    );
    let backup_mod =
        std::fs::read_to_string(root.join("src/backup/mod.rs")).expect("读取 backup/mod.rs 失败");
    assert!(
        !backup_mod.contains("impl AppShell"),
        "备份弹窗的旧外壳方法不得复活，跨域收尾走 BackupPromptEvent"
    );
    let shell = std::fs::read_to_string(root.join("src/app/backup_prompt.rs"))
        .expect("读取备份外壳事件处理失败");
    assert!(
        shell.contains("fn on_backup_event(")
            && !shell.contains("fn submit_backup(")
            && !shell.contains("fn render_backup_prompt_body("),
        "外壳只允许保留打开入口与事件消费，不得重新接管提交或渲染"
    );
}

/// 密钥向导必须由实体持有：状态、在途生成任务与渲染都归 `KeyWizardView`，
/// 旧的外壳动作与 `TaskHub` 任务槽不得复活。外壳只消费事件并注入部署目标。
#[test]
fn key_wizard_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("key_wizard: Entity<KeyWizardView>"),
        "ModalHub 的 key_wizard 必须持有 Entity<KeyWizardView>"
    );
    let task_hub =
        std::fs::read_to_string(root.join("src/app/task_hub.rs")).expect("读取 task_hub.rs 失败");
    assert!(
        !task_hub.contains("keygen: Option<Task<()>>"),
        "密钥生成任务句柄必须归实体，TaskHub 不得保留 keygen 槽位"
    );
    let entity =
        std::fs::read_to_string(root.join("src/keys/entity.rs")).expect("读取密钥实体失败");
    assert!(
        entity.contains("pub(crate) struct KeyWizardView")
            && entity.contains("task: Option<Task<()>>"),
        "KeyWizardView 必须在 keys/entity.rs 持有状态与任务句柄"
    );
    let keys_mod =
        std::fs::read_to_string(root.join("src/keys/mod.rs")).expect("读取 keys/mod.rs 失败");
    assert!(
        !keys_mod.contains("impl AppShell"),
        "密钥向导的旧外壳方法不得复活，跨域提示走 KeyWizardEvent"
    );
    assert!(
        !root.join("src/keys/view.rs").exists(),
        "旧渲染文件 keys/view.rs 已迁入 keys/render.rs，不得复活"
    );
}

/// 会话表单必须由实体持有：打开状态、输入控件、保存/删除动作与渲染都归
/// `SessionFormView`，外壳只保留打开入口、目录/钥匙串落盘与串口数据注入。
#[test]
fn session_form_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("form: Entity<SessionFormView>"),
        "ModalHub 的会话表单必须持有 Entity<SessionFormView>"
    );
    let entity = std::fs::read_to_string(root.join("src/session_form/entity.rs"))
        .expect("读取会话表单实体失败");
    assert!(
        entity.contains("pub(crate) struct SessionFormView"),
        "SessionFormView 实体必须存在于 session_form/entity.rs"
    );
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src/session_form")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        if content.contains("impl AppShell {") {
            offenders.push(normalized_path(&entry));
        }
    }
    assert!(
        offenders.is_empty(),
        "会话表单的旧外壳渲染/动作不得复活，跨域数据走 SessionFormEvent：\n{}",
        offenders.join("\n")
    );
    let ops = std::fs::read_to_string(root.join("src/workspace/tabs/session_ops.rs"))
        .expect("读取会话表单外壳事件处理失败");
    for forbidden in [
        "fn save_session_form(",
        "fn delete_session_form(",
        "fn update_session_form(",
    ] {
        assert!(
            !ops.contains(forbidden),
            "会话表单保存/删除必须走 SessionFormEvent：{forbidden}"
        );
    }
    let view = std::fs::read_to_string(root.join("src/session_form/view.rs"))
        .expect("读取会话表单视图失败");
    assert!(
        !view.contains("render_session_form_body"),
        "旧的外壳渲染入口不得复活，弹窗渲染归 SessionFormView::render_body"
    );
}

/// 容器引擎连接表单必须由实体持有：打开状态、输入控件、保存/测试/删除动作
/// 与渲染都归 `ContainerFormView`，外壳只保留打开入口、分组快照注入与落目录。
#[test]
fn container_form_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("container_form: Entity<ContainerFormView>"),
        "ModalHub 的容器连接表单必须持有 Entity<ContainerFormView>"
    );
    let entity = std::fs::read_to_string(root.join("src/container/form/entity.rs"))
        .expect("读取容器连接表单实体失败");
    assert!(
        entity.contains("pub(crate) struct ContainerFormView"),
        "ContainerFormView 实体必须存在于 container/form/entity.rs"
    );
    let dialogs = std::fs::read_to_string(root.join("src/container/types/dialogs.rs"))
        .expect("读取容器界面状态失败");
    assert!(
        !dialogs.contains("form: Option<ContainerFormState>"),
        "ContainerUiState 不得重新持有连接表单状态"
    );
    assert!(
        !root.join("src/container/form/engine_actions.rs").exists(),
        "旧的外壳动作文件 engine_actions.rs 不得复活"
    );
    let mut offenders = Vec::new();
    for file in [
        "src/container/form/appearance.rs",
        "src/container/form/connection.rs",
        "src/container/form/render.rs",
        "src/container/form/entity.rs",
    ] {
        let path = root.join(file);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if content.contains("impl AppShell {") || content.contains("Context<AppShell>") {
            offenders.push(file.to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "容器连接表单的旧外壳渲染/动作不得复活，跨域数据走 ContainerFormEvent：\n{}",
        offenders.join("\n")
    );
    let connection_events =
        std::fs::read_to_string(root.join("src/container/events/connection.rs"))
            .expect("读取容器连接事件失败");
    assert!(
        connection_events.contains("set_test_result"),
        "测试连接结果必须按 request_id 回写 ContainerFormView"
    );
}

/// 搁置项必须走控制平面：UI 不得直接构造 git 进程或读写搁置项存储。
#[test]
fn shelf_operations_use_command_bus() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/git/shelf.rs");
    let shelf = std::fs::read_to_string(&path).expect("读取 git/shelf.rs 失败");
    for forbidden in ["transom_git::Repo", "transom_store::"] {
        assert!(
            !shelf.contains(forbidden),
            "Shelf 操作不得在 UI 直接使用 {forbidden}，请改走 GitCommand"
        );
    }
    for command in [
        "GitCommand::LoadShelves",
        "GitCommand::Shelve",
        "GitCommand::ApplyShelf",
        "GitCommand::DropShelf",
    ] {
        assert!(shelf.contains(command), "Shelf 入口必须投递 {command}");
    }
}

/// Git 进程级配置只能由控制平面写入：界面不得再直接调用 set_git_*。
#[test]
fn git_config_is_controller_owned() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for pattern in [
            "transom_git::set_git_program",
            "transom_git::set_git_author",
        ] {
            if content.contains(pattern) {
                offenders.push(format!("{}: {pattern}", entry.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "界面不得直接写 Git 全局配置，请投递 GitCommand::Configure：\n{}",
        offenders.join("\n")
    );
}

/// 测试模块声明不能把其后的生产代码一起隐藏：`mod tests;` 是外部测试模块，
/// 只应屏蔽这一行，后面的硬编码中文仍必须被扫描到。
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

/// 审计查看器必须由实体持有：状态、扫描与渲染不得退回 ModalHub 字段。
#[test]
fn audit_viewer_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("audit_viewer: Entity<AuditViewerView>"),
        "ModalHub 的 audit_viewer 必须持有 Entity<AuditViewerView>"
    );
    let entity =
        std::fs::read_to_string(root.join("src/audit_viewer/entity.rs")).expect("读取审计实体失败");
    assert!(
        entity.contains("pub(crate) struct AuditViewerView"),
        "AuditViewerView 必须存在于 audit_viewer/entity.rs"
    );
    assert!(
        !root.join("src/audit_viewer/ops.rs").exists()
            && !root.join("src/audit_viewer/view.rs").exists(),
        "审计查看器旧的 ops.rs/view.rs 应已删除"
    );
}

/// 回放选择弹窗必须由实体持有；录制启停与播放器打开仍可留在外壳。
#[test]
fn replay_modal_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("replay: Entity<ReplayModalView>"),
        "ModalHub 的 replay 必须持有 Entity<ReplayModalView>"
    );
    let entity =
        std::fs::read_to_string(root.join("src/record/entity.rs")).expect("读取回放实体失败");
    assert!(
        entity.contains("pub(crate) struct ReplayModalView"),
        "ReplayModalView 必须存在于 record/entity.rs"
    );
}

/// 已下沉的容器渲染切片不得重新挂回 `impl AppShell`：这些文件只允许
/// 自由渲染函数或实体方法，动作通过实体句柄/事件完成；代码行也不得再出现
/// `Context<AppShell>`（文档注释里的历史说明不计）。
#[test]
fn migrated_container_render_slices_do_not_return_to_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = [
        "src/container/pull_drawer.rs",
        "src/container/view/header.rs",
        "src/container/list/render.rs",
        "src/container/list/filters.rs",
        "src/container/detail/overview.rs",
        "src/container/detail/overview_cards/mod.rs",
        "src/container/detail/logs.rs",
        "src/container/detail/network.rs",
        "src/container/detail/volume.rs",
        "src/container/detail/image.rs",
        "src/container/detail/image_history.rs",
        "src/container/detail/raw_inspect.rs",
        "src/container/detail.rs",
        "src/container/detail/header.rs",
        "src/container/detail/tab_strip.rs",
        "src/container/detail/header_container.rs",
        "src/container/detail/changes.rs",
        "src/container/detail/confirm.rs",
        "src/container/detail/compose.rs",
        "src/container/detail/compose_topology.rs",
        "src/container/detail/header_actions.rs",
        "src/container/detail/terminal.rs",
        "src/container/detail/file_preview.rs",
        "src/container/detail/files/mod.rs",
        "src/container/detail/files/row.rs",
        "src/container/detail/files/toolbar.rs",
        "src/container/actions/changes.rs",
        "src/container/logs_actions.rs",
        "src/container/stats.rs",
        "src/container/stats/toolbar.rs",
        "src/container/stats/ports.rs",
        "src/container/stats/processes.rs",
        "src/container/stats_actions.rs",
        "src/container/files_actions/browse.rs",
        "src/container/files_actions/edit.rs",
        "src/container/files_actions/mutate.rs",
        "src/container/files_actions/prompt.rs",
        "src/container/files_actions/transfer.rs",
    ];
    let mut offenders = Vec::new();
    for file in files {
        let path = root.join(file);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let uses_shell_context = content.lines().any(|line| {
            let line = line.trim_start();
            !line.starts_with("//") && line.contains("Context<AppShell>")
        });
        if content.contains("impl AppShell") || uses_shell_context {
            offenders.push(file.to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "已下沉的容器渲染文件不得再挂回外壳：\n{}",
        offenders.join("\n")
    );
}
