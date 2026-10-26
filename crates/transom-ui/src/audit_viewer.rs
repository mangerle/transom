//! 应用内置审计纯文本日志查看器。
//!
//! 设计说明（Why）：
//! - 会话审计原始输出流已按会话存放在 `config/transom/audit` 下；
//! - 本模块提供按会话快速检索、查看纯文本内容、搜索过滤与一键定位文件的应用内查看器，
//!   解决每次排查连接会话历史都必须跳到外部文本编辑器的问题；
//! - 读取限制最大 2 MiB 避免超大日志瞬时撑爆内存；
//! - 展示前自动剥离 ANSI 转义字符，使终端输出以规整干净的纯文本呈现。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::io::Read;
use std::path::{Path, PathBuf};
use transom_core::{Connection, Session, SessionId, TabId, paths};

/// 单次读取最大字节数（2 MiB），防止大文件撑爆 UI 线程。
const MAX_AUDIT_LOG_BYTES: usize = 2 * 1024 * 1024;

/// 审计日志条目元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuditLogEntry {
    pub path: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
    pub modified_display: String,
}

/// 审计日志查看器状态。
pub(crate) struct AuditViewer {
    pub target_title: String,
    pub session_name: Option<String>,
    pub host: Option<String>,
    pub entries: Vec<AuditLogEntry>,
    pub selected_index: usize,
    pub content: String,
    pub is_truncated: bool,
    pub search_input: Entity<InputState>,
    pub show_all: bool,
}

impl AppShell {
    /// 打开审计日志查看器。
    pub(crate) fn open_audit_viewer(
        &mut self,
        session: Option<&Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (session_name, host, target_title) = match session {
            Some(s) => {
                let h = match &s.connection {
                    Connection::Ssh(ssh) => ssh.host.clone(),
                    Connection::Telnet(telnet) => telnet.host.clone(),
                    Connection::Serial(serial) => serial.port.clone(),
                    Connection::Local(_) => "local".to_string(),
                    _ => "session".to_string(),
                };
                (
                    Some(s.name.clone()),
                    Some(h),
                    rust_i18n::t!("audit_viewer.for_session", name = s.name.clone()).to_string(),
                )
            }
            None => (None, None, rust_i18n::t!("audit_viewer.title").to_string()),
        };

        let audit_dir = paths::audit_dir();
        let entries = find_session_audit_logs(
            &audit_dir,
            session_name.as_deref(),
            host.as_deref(),
            session_name.is_none(),
        );

        let (content, is_truncated) = if let Some(first) = entries.first() {
            read_log_content(&first.path, MAX_AUDIT_LOG_BYTES)
        } else {
            (String::new(), false)
        };

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("audit_viewer.search_placeholder").to_string())
        });

        self.audit_viewer = Some(AuditViewer {
            target_title,
            session_name,
            host,
            entries,
            selected_index: 0,
            content,
            is_truncated,
            search_input,
            show_all: session.is_none(),
        });
        cx.notify();
    }

    /// 打开指定标签页的审计日志查看器。
    pub(crate) fn open_audit_viewer_for_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .map(|tab| tab.session.clone());
        if let Some(s) = session {
            self.open_audit_viewer(Some(&s), window, cx);
        } else {
            self.open_audit_viewer(None, window, cx);
        }
    }

    /// 打开指定会话的审计日志查看器。
    pub(crate) fn open_audit_viewer_for_session(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned();
        if let Some(s) = session {
            self.open_audit_viewer(Some(&s), window, cx);
        } else {
            self.open_audit_viewer(None, window, cx);
        }
    }

    /// 切换查看的审计日志文件。
    pub(crate) fn select_audit_log(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(viewer) = self.audit_viewer.as_mut() else {
            return;
        };
        if index >= viewer.entries.len() {
            return;
        }
        viewer.selected_index = index;
        let path = viewer.entries[index].path.clone();
        let (content, is_truncated) = read_log_content(&path, MAX_AUDIT_LOG_BYTES);
        viewer.content = content;
        viewer.is_truncated = is_truncated;
        cx.notify();
    }

    /// 切换“仅当前会话”与“全部会话”范围。
    pub(crate) fn toggle_audit_viewer_scope(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.audit_viewer.as_mut() else {
            return;
        };
        viewer.show_all = !viewer.show_all;
        let audit_dir = paths::audit_dir();
        viewer.entries = find_session_audit_logs(
            &audit_dir,
            viewer.session_name.as_deref(),
            viewer.host.as_deref(),
            viewer.show_all,
        );
        viewer.selected_index = 0;
        let (content, is_truncated) = if let Some(first) = viewer.entries.first() {
            read_log_content(&first.path, MAX_AUDIT_LOG_BYTES)
        } else {
            (String::new(), false)
        };
        viewer.content = content;
        viewer.is_truncated = is_truncated;
        cx.notify();
    }

    /// 复制当前审计日志内容到剪贴板。
    pub(crate) fn copy_audit_viewer_content(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.audit_viewer.as_ref() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(viewer.content.clone()));
        self.status = Some(rust_i18n::t!("audit_viewer.copied").to_string());
        cx.notify();
    }

    /// 渲染审计日志查看器模态弹窗。
    pub(crate) fn render_audit_viewer(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(viewer) = self.audit_viewer.as_ref() else {
            return div().into_any_element();
        };

        let title = viewer.target_title.clone();
        let selected_file = viewer.entries.get(viewer.selected_index);
        let selected_path = selected_file.map(|e| e.path.clone());
        let current_search = viewer.search_input.read(cx).value().trim().to_string();

        let scope_button_text = if viewer.show_all {
            rust_i18n::t!("audit_viewer.current_session").to_string()
        } else {
            rust_i18n::t!("audit_viewer.all_logs").to_string()
        };

        // 顶栏操作区
        let mut toolbar = h_flex().gap_2().items_center();

        if viewer.session_name.is_some() {
            toolbar = toolbar.child(
                Button::new("toggle-audit-scope")
                    .outline()
                    .label(scope_button_text)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_audit_viewer_scope(cx);
                    })),
            );
        }

        if let Some(path) = selected_path.clone() {
            toolbar = toolbar.child(
                Button::new("reveal-audit-file")
                    .outline()
                    .icon(IconName::FolderOpen)
                    .label(rust_i18n::t!("audit_viewer.reveal").to_string())
                    .on_click(move |_, _, _| {
                        crate::audit::reveal_in_file_manager(path.clone());
                    }),
            );
        }

        toolbar = toolbar.child(
            Button::new("copy-audit-content")
                .outline()
                .icon(IconName::Copy)
                .label(rust_i18n::t!("audit_viewer.copy_content").to_string())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.copy_audit_viewer_content(cx);
                })),
        );

        // 左侧文件列表
        let mut file_list = v_flex().id("audit-file-list").gap_1().overflow_y_scroll();
        if viewer.entries.is_empty() {
            file_list = file_list.child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .items_center()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("audit_viewer.no_logs").to_string())
                    .child(
                        div()
                            .text_center()
                            .child(rust_i18n::t!("audit_viewer.no_logs_hint").to_string()),
                    )
                    .child(
                        Button::new("open-settings-from-audit")
                            .outline()
                            .label(rust_i18n::t!("audit_viewer.open_settings").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.audit_viewer = None;
                                this.open_settings_modal(window, cx);
                            })),
                    ),
            );
        } else {
            for (idx, entry) in viewer.entries.iter().enumerate() {
                let is_selected = idx == viewer.selected_index;
                let bg_color = if is_selected {
                    p.primary.opacity(0.15)
                } else {
                    gpui::transparent_black()
                };
                let border_color = if is_selected {
                    p.primary
                } else {
                    gpui::transparent_black()
                };

                file_list = file_list.child(
                    v_flex()
                        .id(ElementId::Name(format!("audit-log-item-{idx}").into()))
                        .p_2()
                        .rounded(p.radius.sm)
                        .bg(bg_color)
                        .border_1()
                        .border_color(border_color)
                        .cursor_pointer()
                        .hover(|style| style.bg(p.muted.opacity(0.4)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_audit_log(idx, cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.foreground)
                                .truncate()
                                .child(entry.file_name.clone()),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(entry.modified_display.clone())
                                .child(format_file_size(entry.size_bytes)),
                        ),
                );
            }
        }

        // 右侧内容查看区
        let mut filtered_lines = Vec::new();
        for line in viewer.content.lines() {
            if current_search.is_empty()
                || line
                    .to_ascii_lowercase()
                    .contains(&current_search.to_ascii_lowercase())
            {
                filtered_lines.push(line);
            }
        }

        let content_text = if filtered_lines.is_empty() && !viewer.content.is_empty() {
            "(未找到匹配搜索的内容)".to_string()
        } else {
            filtered_lines.join("\n")
        };

        let right_pane = v_flex()
            .flex_1()
            .gap_2()
            .size_full()
            .overflow_hidden()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().child(viewer.search_input.clone()))
                    .child(if viewer.is_truncated {
                        div()
                            .text_xs()
                            .text_color(p.warning)
                            .child(rust_i18n::t!("audit_viewer.truncated_hint").to_string())
                    } else {
                        div()
                    }),
            )
            .child(
                div()
                    .id("audit-content-scroll")
                    .flex_1()
                    .size_full()
                    .p_3()
                    .rounded(p.radius.md)
                    .bg(p.muted.opacity(0.3))
                    .border_1()
                    .border_color(p.border)
                    .overflow_y_scroll()
                    .font_family(p.mono_font_family.clone())
                    .text_size(p.mono_font_size)
                    .line_height(relative(1.4))
                    .text_color(p.foreground)
                    .child(content_text),
            );

        let body = h_flex()
            .gap_3()
            .h(px(520.0))
            .child(
                div()
                    .w(px(260.0))
                    .h_full()
                    .border_r_1()
                    .border_color(p.border)
                    .pr_2()
                    .child(file_list),
            )
            .child(right_pane);

        let footer = h_flex()
            .justify_between()
            .items_center()
            .child(toolbar)
            .child(
                Button::new("close-audit-viewer")
                    .ghost()
                    .label(rust_i18n::t!("audit_viewer.close").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.audit_viewer = None;
                        cx.notify();
                    })),
            );

        let dialog = Self::dialog_frame(
            "audit-viewer-modal",
            title,
            selected_file.map(|f| f.file_name.clone()),
            body,
            Some(footer),
            px(900.0),
            p,
        );
        Self::overlay(dialog, p)
    }
}

/// 扫描审计目录并按会话与过滤条件返回条目。
pub(crate) fn find_session_audit_logs(
    audit_dir: &Path,
    session_name: Option<&str>,
    host: Option<&str>,
    show_all: bool,
) -> Vec<AuditLogEntry> {
    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(audit_dir) else {
        return entries;
    };

    let prefix = if !show_all && let (Some(name), Some(host)) = (session_name, host) {
        let label = paths::sanitize_audit_label(name);
        let host = paths::sanitize_audit_label(host);
        Some(format!("{label}-{host}"))
    } else {
        None
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("log") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();

        if let Some(prefix) = &prefix
            && !file_name.starts_with(prefix)
        {
            continue;
        }

        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let size_bytes = meta.len();
        let modified_display = meta
            .modified()
            .ok()
            .map(|time| {
                let dt: chrono::DateTime<chrono::Local> = time.into();
                dt.format("%Y-%m-%d %H:%M:%S").to_string()
            })
            .unwrap_or_else(|| "-".to_string());

        entries.push(AuditLogEntry {
            path,
            file_name,
            size_bytes,
            modified_display,
        });
    }

    entries.sort_by(|a, b| b.file_name.cmp(&a.file_name));
    entries
}

/// 从文件流式读取纯文本并剥离 ANSI 转义字符。
pub(crate) fn read_log_content(path: &Path, max_bytes: usize) -> (String, bool) {
    let Ok(mut file) = std::fs::File::open(path) else {
        return ("(无法打开日志文件)".to_string(), false);
    };
    let mut buffer = Vec::new();
    let mut take = (&mut file).take((max_bytes + 1) as u64);
    if take.read_to_end(&mut buffer).is_err() {
        return ("(读取日志文件出错)".to_string(), false);
    }
    let is_truncated = buffer.len() > max_bytes;
    if is_truncated {
        buffer.truncate(max_bytes);
    }
    let raw = String::from_utf8_lossy(&buffer);
    (strip_ansi_escapes(&raw), is_truncated)
}

/// 轻量级剥离终端 ANSI 转义字符序列并保留规整纯文本。
pub(crate) fn strip_ansi_escapes(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() || c == 'm' || c == 'K' || c == 'H' || c == 'J' {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                chars.next();
                for c in chars.by_ref() {
                    if c == '\x07' || c == '\x1b' {
                        break;
                    }
                }
            }
        } else if ch == '\r' {
            // 忽略回车符，保留换行
            if chars.peek() != Some(&'\n') {
                output.push('\n');
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    // 不用 `use super::*`：本模块 glob 导入了 gpui，而 gpui 里有同名属性宏
    // `test`（gpui::test），glob 进来会把标准 `#[test]` 劫持成它，导致宏递归。
    use super::{format_file_size, strip_ansi_escapes};

    #[test]
    fn test_strip_ansi_escapes() {
        let input = "\x1b[31mRed Text\x1b[0m and \x1b[1;32mGreen\x1b[0m\r\nNext Line";
        let cleaned = strip_ansi_escapes(input);
        assert_eq!(cleaned, "Red Text and Green\nNext Line");
    }

    #[test]
    fn test_format_file_size() {
        assert_eq!(format_file_size(500), "500 B");
        assert_eq!(format_file_size(2048), "2.0 KiB");
        assert_eq!(format_file_size(5 * 1024 * 1024), "5.0 MiB");
    }
}
