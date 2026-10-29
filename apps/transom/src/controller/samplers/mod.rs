//! 状态栏采样器：网络速率与系统统计（远端/本地共享两套）。

mod common;
mod process;
mod stats;
mod traffic;

#[cfg(test)]
mod tests;

pub(crate) use process::{ProcessTarget, run_process_count, run_process_kill, run_process_sample};
pub(crate) use stats::{run_local_stats_sampler, spawn_remote_stats_sampler};
pub(crate) use traffic::{LocalTrafficTabs, run_local_traffic_sampler, spawn_traffic_sampler};
