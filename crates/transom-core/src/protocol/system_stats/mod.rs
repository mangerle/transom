//! 远程与本地系统资源统计数据解析器与模型。

mod parser;
mod types;

#[cfg(test)]
mod tests;

pub use parser::{
    StatsSamplerState, calculate_cpu_percent, calculate_interface_rates, parse_combined_stats,
    parse_cpu_cores, parse_cpu_jiffies, parse_disk, parse_filesystems, parse_meminfo,
    parse_net_dev_interfaces, parse_os_name, parse_processes, parse_uptime,
};
pub use types::{CoreUsage, FilesystemUsage, InterfaceRate, ProcessInfo};
