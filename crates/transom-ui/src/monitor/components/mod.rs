//! 监控面板通用 UI 组件库。

pub(crate) mod card;
pub(crate) mod sparkline;

pub(crate) use card::CollapsibleCard;
pub(crate) use sparkline::{render_network_dual_sparkline, render_percent_sparkline};
