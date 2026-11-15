pub(crate) mod components;
mod entity;
mod layout_ops;
pub(crate) use layout_ops::{
    apply_monitor_drawer_width, dock_monitor_to_drawer, maximize_monitor_to_tab,
};
mod ops;
pub(crate) use ops::{
    after_monitor_collapse, ensure_monitor_task, monitor_count_sampled, monitor_failed,
    monitor_killed, monitor_sampled, on_monitor_panel_event, restart_monitor_task,
    restart_systemd_service, sample_processes, send_kill_process, send_terminal_input,
    set_monitor_interval_mode, toggle_monitor, trace_process_in_terminal,
};
mod panel_event;
mod sections;
mod types;
mod view;

pub(crate) use entity::MonitorView;
pub(crate) use panel_event::MonitorPanelEvent;
pub(crate) use types::usage_color;
pub(crate) use types::{MonitorSectionCollapse, SampleIntervalMode, SparklineChartType};
pub(crate) use view::MonitorTabParams;
