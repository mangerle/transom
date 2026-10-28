//! 磁盘存储与挂载点文件系统监控板块。

use crate::app::AppShell;
use crate::monitor::components::CollapsibleCard;
use crate::monitor::types::{format_size, usage_bar, usage_color};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_core::SystemStats;

impl AppShell {
    /// 渲染磁盘与文件系统监控卡片。
    pub(crate) fn render_disk_section_card(
        &self,
        stats: &SystemStats,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let collapsed = self.monitor.collapse.disk;

        // 折叠态紧凑摘要（以根目录为核心代表）
        let root_fs = stats.filesystems.iter().find(|fs| fs.mount == "/");
        let (root_used, root_total, root_pct) = match root_fs {
            Some(fs) if fs.total_bytes > 0 => {
                let pct = (fs.used_bytes as f32 / fs.total_bytes as f32) * 100.0;
                (format_size(fs.used_bytes), format_size(fs.total_bytes), pct)
            }
            _ => (
                format_size(stats.disk_used_bytes),
                format_size(stats.disk_total_bytes),
                0.0,
            ),
        };

        let summary = h_flex()
            .gap_2()
            .text_xs()
            .child(
                div()
                    .text_color(p.muted_foreground)
                    .child(format!("{root_used}/{root_total}")),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(usage_color(root_pct, p))
                    .child(format!("{root_pct:.0}%")),
            );

        // 展开态详细挂载点列表
        let mut content = v_flex().gap_2();

        if stats.filesystems.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("monitor.no_data").to_string()),
            );
        }

        for filesystem in stats.filesystems.iter().take(8) {
            let fraction = if filesystem.total_bytes == 0 {
                0.0
            } else {
                filesystem.used_bytes as f32 / filesystem.total_bytes as f32
            };
            let color = usage_color(fraction * 100.0, p);
            content = content.child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .text_xs()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.foreground)
                                    .child(filesystem.mount.clone()),
                            )
                            .child(div().text_color(p.muted_foreground).child(format!(
                                "{} / {} ({:.0}%)",
                                format_size(filesystem.used_bytes),
                                format_size(filesystem.total_bytes),
                                fraction * 100.0
                            ))),
                    )
                    .child(usage_bar(fraction, color, p)),
            );
        }

        CollapsibleCard::new(
            "card-disk",
            IconName::HardDrive,
            rust_i18n::t!("monitor.disk").to_string(),
        )
        .collapsed(collapsed)
        .summary(summary)
        .content(content)
        .render(
            p,
            cx.listener(|this, _, _, cx| {
                this.toggle_section_collapse("disk", cx);
            }),
        )
    }
}
