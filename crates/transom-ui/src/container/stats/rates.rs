//! 速率换算、趋势窗口与通用卡片辅助。

use super::*;

pub(crate) fn stat_card(
    id: &'static str,
    icon: IconName,
    title: String,
    summary: AnyElement,
    content: AnyElement,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .id(id)
        .flex_1()
        .min_w_0()
        .min_h(px(160.0))
        .rounded(p.radius.md)
        .bg(p.muted.opacity(0.12))
        .border_1()
        .border_color(p.divider(p.border))
        .overflow_hidden()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .justify_between()
                .px_2p5()
                .py_2()
                .border_color(p.divider(p.border))
                .divider_edge(DividerEdge::Bottom, p.divider_width)
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .min_w_0()
                        .child(Icon::new(icon).xsmall().text_color(p.primary))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(title),
                        ),
                )
                .child(summary),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_2p5()
                .p_2p5()
                .child(content),
        )
        .into_any_element()
}

pub(crate) fn summary_text(value: String, color: Hsla) -> AnyElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(value)
        .into_any_element()
}

/// 相邻采样的增量按秒折算：主机监控的速率口径是 `/s`，容器这里保持一致。
pub(crate) fn per_second(
    history: &[ContainerStats],
    pick: impl Fn(&ContainerStats) -> u64,
    seconds: f32,
) -> Vec<u64> {
    history
        .windows(2)
        .map(|pair| {
            let delta = pick(&pair[1]).saturating_sub(pick(&pair[0])) as f64;
            (delta / seconds as f64) as u64
        })
        .collect()
}

pub(crate) fn latest_interface_rates(
    history: &[ContainerStats],
    seconds: f32,
) -> Vec<(String, u64, u64)> {
    let Some(last) = history.last() else {
        return Vec::new();
    };
    let previous = history.len().checked_sub(2).map(|index| &history[index]);
    last.networks
        .iter()
        .map(|interface| {
            let (rx, tx) = previous
                .and_then(|sample| {
                    sample
                        .networks
                        .iter()
                        .find(|item| item.name == interface.name)
                })
                .map(|item| {
                    (
                        interface.rx_bytes.saturating_sub(item.rx_bytes),
                        interface.tx_bytes.saturating_sub(item.tx_bytes),
                    )
                })
                .unwrap_or((0, 0));
            (
                interface.name.clone(),
                (rx as f64 / seconds as f64) as u64,
                (tx as f64 / seconds as f64) as u64,
            )
        })
        .collect()
}

pub(crate) fn latest_device_rates(
    history: &[ContainerStats],
    seconds: f32,
) -> Vec<(String, u64, u64)> {
    let Some(last) = history.last() else {
        return Vec::new();
    };
    let previous = history.len().checked_sub(2).map(|index| &history[index]);
    last.block_devices
        .iter()
        .map(|device| {
            let (read, write) = previous
                .and_then(|sample| {
                    sample
                        .block_devices
                        .iter()
                        .find(|item| item.device == device.device)
                })
                .map(|item| {
                    (
                        device.read_bytes.saturating_sub(item.read_bytes),
                        device.write_bytes.saturating_sub(item.write_bytes),
                    )
                })
                .unwrap_or((0, 0));
            (
                device.device.clone(),
                (read as f64 / seconds as f64) as u64,
                (write as f64 / seconds as f64) as u64,
            )
        })
        .collect()
}
