//! 偏好设置中的"日志"分组：运行日志级别、日志目录入口与历史备份清理。
//!
//! 设计原理（Why）：
//! - 实现初衷：桌面应用的用户没法像服务端那样随手 `journalctl -f`，出问题只能靠
//!   重启前的一段输出。把级别、目录与清理做到界面里，报障时"调 debug → 复现 →
//!   点开发目录"三步就能拿到现场。
//! - 级别立即生效：log 门面的门槛是一个全局原子量（`log::set_max_level`），不必
//!   重启也不必重装 logger；这与启动时的取值路径一致（见 `transom_ui::run`）。
//! - 代价与局限：`debug`/`trace` 会把协议与命令细节写进本地文件，其中可能包含
//!   远端主机名、路径与命令行；审计内容（屏幕原始回显）不在这条链路上，另有开关。
//!   清理只动已轮转的备份，正在写入的主文件不删——否则会留下一个指向已 unlink
//!   文件的句柄，之后的写入全部进不了磁盘还看不出原因。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::*;
use transom_core::{LogLevel, paths};

/// 清理历史备份时扫描的份数上界。
///
/// 应用侧的保留份数是编译期常量，这里刻意扫得更宽：用户手工改过配置或旧版本
/// 留下更多份文件时，清理按钮仍能把它们一并收掉，而不是留下几个"删不掉的孤儿"。
const CLEANUP_SCAN_LIMIT: usize = 12;

/// 级别对应的文案键。
///
/// 用 `match` 而不是拼 `format!("settings.log_level.{}`）：文案键必须是编译期
/// 已知的静态串，运行时拼出来的键在语言文件缺项时只会静默回退，查不到漏翻。
fn level_label_key(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "settings.log_level_error",
        LogLevel::Warn => "settings.log_level_warn",
        LogLevel::Info => "settings.log_level_info",
        LogLevel::Debug => "settings.log_level_debug",
        LogLevel::Trace => "settings.log_level_trace",
    }
}

impl AppShell {
    /// 日志组：级别选择、目录展示与打开、历史备份清理。
    pub(crate) fn render_settings_logs(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = self.settings.logs.level;
        let log_dir = paths::log_dir();
        let dir_label = log_dir.display().to_string();
        let open_dir = cx.listener(move |this, _, _, cx| {
            crate::audit::reveal_in_file_manager(log_dir.clone());
            this.status = Some(rust_i18n::t!("settings.log_revealed").to_string());
            cx.notify();
        });
        let cleanup = cx.listener(move |this, _, _, cx| {
            let removed = this.prune_rotated_logs();
            this.status =
                Some(rust_i18n::t!("settings.log_cleanup_done", count = removed).to_string());
            cx.notify();
        });

        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.log_level",
                Some("settings.log_level_desc"),
                h_flex()
                    .gap_1()
                    .children(LogLevel::ALL.into_iter().map(|level| {
                        Button::new(format!("log-level-{}", level.as_str()))
                            .xsmall()
                            .label(rust_i18n::t!(level_label_key(level)).to_string())
                            .selected(current == level)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.settings.logs.level = level;
                                // 门槛就是一个全局原子量，改完下一条日志即刻生效。
                                log::set_max_level(level.to_filter());
                                cx.notify();
                            }))
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.log_dir",
                Some("settings.log_dir_desc"),
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .max_w(rems(24.0))
                            .truncate()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(dir_label),
                    )
                    .child(
                        Button::new("log-dir-reveal")
                            .small()
                            .label(rust_i18n::t!("settings.log_dir_open").to_string())
                            .on_click(open_dir),
                    )
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.log_cleanup",
                Some("settings.log_cleanup_desc"),
                Button::new("log-cleanup")
                    .small()
                    .label(rust_i18n::t!("settings.log_cleanup_action").to_string())
                    .on_click(cleanup)
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 删除目录里已轮转的日志备份，返回成功删除的份数。
    ///
    /// 单个文件删不掉（被别的程序占用、权限被改）只记一条警告并继续：清理是
    /// 尽力而为的维护动作，不该因为一个文件失败就把按钮整体报错，让用户以为
    /// 完全没生效。
    fn prune_rotated_logs(&self) -> usize {
        let log_path = paths::log_file_path();
        let mut removed = 0usize;
        for index in 1..=CLEANUP_SCAN_LIMIT {
            let candidate = paths::rotated_backup_path(&log_path, index);
            if !candidate.exists() {
                continue;
            }
            match std::fs::remove_file(&candidate) {
                Ok(()) => removed += 1,
                Err(error) => log::warn!(
                    "清理历史日志失败，已跳过该份，路径: {}，原因: {error}",
                    candidate.display()
                ),
            }
        }
        log::info!(
            "历史日志清理完成，删除份数: {removed}，目标目录: {}",
            log_path.display()
        );
        removed
    }
}
