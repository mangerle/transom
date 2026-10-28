//! 远程与本地系统资源统计数据解析器与状态机。

pub mod parser;

#[cfg(test)]
mod tests;

pub use parser::{
    CpuTimesSnapshot, StatsSamplerState, calculate_cpu_breakdown, calculate_cpu_percent,
    calculate_interface_rates, parse_combined_stats, parse_cpu_cores, parse_cpu_jiffies,
    parse_cpu_times, parse_disk, parse_failed_services, parse_filesystems, parse_loadavg,
    parse_meminfo, parse_meminfo_full, parse_net_dev_interfaces, parse_oom_alert, parse_os_name,
    parse_ports, parse_processes, parse_uptime,
};
