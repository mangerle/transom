//! 状态栏采样器：网络速率与系统统计（远端/本地共享两套）。

mod common;
mod local_host;
mod process;
mod stats;
mod traffic;

#[cfg(test)]
mod tests;

pub(crate) use local_host::{LocalHostTabs, run_local_host_sampler};
pub(crate) use process::{ProcessTarget, run_process_count, run_process_kill, run_process_sample};
pub(crate) use stats::spawn_remote_stats_sampler;
pub(crate) use traffic::spawn_traffic_sampler;
