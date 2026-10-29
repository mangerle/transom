//! 桌面通知策略：什么时候值得打断用户，以及通知文案怎么拼。
//!
//! 判定刻意做成纯函数（[`should_notify`]），这样"该不该弹"能在没有窗口环境的情况下测；
//! 只有真正发通知的那一步才依赖 gpui 与 notify-rust。

use crate::app::AppShell;
use crate::sftp::send_desktop_notification;
use gpui_kit::*;
use transom_core::TabId;

/// 阈值秒数上限：超过 1 小时的阈值等价于"永远不通知"，直接拒绝写入设置。
const NOTIFY_THRESHOLD_MAX_SECS: u64 = 3600;

impl AppShell {
    /// shell 集成报告一条命令跑完。
    pub(crate) fn on_command_finished(
        &mut self,
        tab_id: TabId,
        exit_code: Option<i32>,
        duration_ms: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = &self.settings.window;
        let pane_active = self.workspace.active_tab == Some(tab_id);
        if !should_notify(
            settings.desktop_notifications,
            duration_ms,
            settings.notify_threshold_secs,
            window.is_window_active(),
            pane_active,
        ) {
            return;
        }
        let pane_name = self
            .workspace
            .tab(tab_id)
            .map(|tab| tab.display_name())
            .unwrap_or_default();
        let exit_text = match exit_code {
            Some(0) => rust_i18n::t!("notify.exit_code_zero").to_string(),
            Some(code) => rust_i18n::t!("notify.exit_code", code = code).to_string(),
            // 只有 A/B 标记的 shell 集成拿不到退出码，此时宁可不写也不写个假的 0。
            None => rust_i18n::t!("notify.exit_code_unknown").to_string(),
        };
        let duration_text = format_command_duration(duration_ms);
        send_desktop_notification(
            rust_i18n::t!("notify.command_done_title").as_ref(),
            rust_i18n::t!(
                "notify.command_done_body",
                pane = pane_name,
                duration = duration_text,
                exit = exit_text
            )
            .as_ref(),
        );
        self.status = Some(rust_i18n::t!("notify.command_done_status").to_string());
        cx.notify();
    }

    /// 终端程序通过 OSC 777 显式请求的通知：程序自己决定了"值得看一眼"，
    /// 因此不加耗时阈值，只受总开关约束。
    pub(crate) fn on_desktop_notification(
        &mut self,
        title: &str,
        body: &str,
        cx: &mut Context<Self>,
    ) {
        if !self.settings.window.desktop_notifications {
            return;
        }
        let summary = if title.is_empty() {
            rust_i18n::t!("notify.default_title").to_string()
        } else {
            format!("Transom · {title}")
        };
        send_desktop_notification(&summary, body);
        self.status = Some(rust_i18n::t!("notify.desktop_sent").to_string());
        cx.notify();
    }
}

/// 是否应该弹通知。
///
/// 门槛设计成"总开关 + 耗时阈值 + 确实不在看这一屏"三重，是因为通知一旦误弹，
/// 用户会直接把这个功能整体关掉；漏弹只是少一次提醒。
fn should_notify(
    enabled: bool,
    duration_ms: u64,
    threshold_secs: u64,
    window_active: bool,
    pane_active: bool,
) -> bool {
    if !enabled {
        return false;
    }
    let threshold_ms = threshold_secs.saturating_mul(1_000);
    if duration_ms < threshold_ms {
        return false;
    }
    // 系统没聚焦这个窗口，或者完成的命令在后台标签里：用户确实看不见它跑完。
    !window_active || !pane_active
}

/// 把毫秒数说成人话：秒级保留一位小数，分钟以上只报整数，避免"1 分 03.7 秒"这种噪音。
fn format_command_duration(duration_ms: u64) -> String {
    let secs = duration_ms as f64 / 1000.0;
    if duration_ms < 60_000 {
        return format!("{:.1} {}", secs, rust_i18n::t!("notify.unit_second"));
    }
    let total_secs = duration_ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        return rust_i18n::t!(
            "notify.duration_hms",
            hours = hours,
            minutes = minutes,
            seconds = seconds
        )
        .to_string();
    }
    rust_i18n::t!("notify.duration_ms", minutes = minutes, seconds = seconds).to_string()
}

/// 校验阈值输入：越界或非法都返回 `None`，由调用方保留旧值。
///
/// 下界取 1 秒：0 意味着每条命令都弹一次，那不是"长任务提醒"而是噪声发生器。
pub(crate) fn parse_notify_threshold(raw: &str) -> Option<u64> {
    let parsed = raw.trim().parse::<u64>().ok()?;
    (1..=NOTIFY_THRESHOLD_MAX_SECS)
        .contains(&parsed)
        .then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::{format_command_duration, parse_notify_threshold, should_notify};

    #[test]
    fn formats_duration_by_magnitude() {
        let seconds = |text: &str| format!("{text} {}", rust_i18n::t!("notify.unit_second"));
        assert_eq!(format_command_duration(0), seconds("0.0"));
        assert_eq!(format_command_duration(400), seconds("0.4"));
        assert_eq!(format_command_duration(12_400), seconds("12.4"));
        assert_eq!(format_command_duration(59_999), seconds("60.0"));
        assert_eq!(
            format_command_duration(65_000),
            rust_i18n::t!("notify.duration_ms", minutes = 1, seconds = 5).to_string()
        );
        assert_eq!(
            format_command_duration(3_725_000),
            rust_i18n::t!("notify.duration_hms", hours = 1, minutes = 2, seconds = 5).to_string()
        );
    }

    #[test]
    fn gates_on_threshold_and_attention() {
        // 没到阈值不弹，即使正在忙别的事。
        assert!(!should_notify(true, 9_999, 10, false, false));
        assert!(should_notify(true, 3_000, 3, false, false));
        assert!(should_notify(true, 10_000, 10, false, false));
        // 总开关关掉后一律不弹。
        assert!(!should_notify(false, 999_999, 10, false, false));
        // 窗口正被系统聚焦、且就是当前标签：用户看得见，不打扰。
        assert!(!should_notify(true, 30_000, 10, true, true));
        // 窗口聚焦但命令在后台标签：仍然要提醒。
        assert!(should_notify(true, 30_000, 10, true, false));
        // 窗口失焦时即便标签是当前标签也要提醒（用户已经切去别的程序）。
        assert!(should_notify(true, 30_000, 10, false, true));
    }

    /// 阈值不能设成荒谬值：0 秒等价于每条命令都弹，超过一小时等价于永不弹。
    #[test]
    fn parses_threshold_with_bounds() {
        assert_eq!(parse_notify_threshold(" 15 "), Some(15));
        assert_eq!(
            parse_notify_threshold("0"),
            None,
            "0 秒阈值等价于每条命令都弹，拒绝"
        );
        assert_eq!(parse_notify_threshold("3600"), Some(3600));
        assert_eq!(parse_notify_threshold("3601"), None);
        assert_eq!(parse_notify_threshold("abc"), None);
        assert_eq!(parse_notify_threshold("-1"), None);
        assert_eq!(parse_notify_threshold(""), None);
    }
}
