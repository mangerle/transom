pub(crate) mod components;
mod entity;
mod layout_ops;
pub(crate) use layout_ops::{
    apply_monitor_drawer_width, dock_monitor_to_drawer, maximize_monitor_to_tab,
};
mod ops;
pub(crate) use ops::{MonitorOps, MonitorTerminalPlan};
mod palette;
pub(crate) use palette::MonitorPalette;
mod panel_event;
mod sections;
mod types;
mod view;

pub(crate) use entity::MonitorView;
pub(crate) use panel_event::MonitorPanelEvent;
pub(crate) use types::{MonitorSectionCollapse, SampleIntervalMode, SparklineChartType};
pub(crate) use view::MonitorTabParams;
