//! 跨平台宿主资源采样：CPU、内存、磁盘、网络、进程与本地进程终止。
//!
//! 设计原理（Why）：Linux 可以直接读 `/proc`，但 macOS 与 Windows 没有等价
//! 文件；把差异交给 `sysinfo` 的系统 API，业务层只消费统一快照，避免继续
//! 维护 `sh`、`df`、`ps`、`kill` 等按平台分叉的命令字符串。
//!
//! 核心优势：同一套采样器覆盖三端，网络速率按累计计数器差值计算，磁盘与
//! 进程信息经过结构化类型进入上层，不再依赖本地化文本解析。
//!
//! 代价与局限：`sysinfo` 不提供用户态/系统态 CPU 拆分和 systemd 诊断；
//! 快照通过能力标志显式说明这些信息不可用，UI 不应把“缺少能力”显示成 0。

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, Networks, Pid, ProcessRefreshKind, ProcessesToUpdate,
    RefreshKind, Signal, System, UpdateKind, Users,
};

/// 单个逻辑核心的使用率。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CoreUsage {
    pub index: usize,
    pub percent: f32,
}

/// 单个挂载点的占用。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilesystemUsage {
    pub mount: String,
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// 单块非回环网卡的本轮速率。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InterfaceRate {
    pub name: String,
    pub rx_bytes_per_sec: u64,
    pub tx_bytes_per_sec: u64,
}

/// 一轮宿主资源快照；Linux 专属诊断字段不在此结构中伪造。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SystemSnapshot {
    pub cpu_percent: f32,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub uptime_secs: u64,
    pub os_name: String,
    pub load_avg: (f32, f32, f32),
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub cpu_cores: Vec<CoreUsage>,
    pub filesystems: Vec<FilesystemUsage>,
    pub interfaces: Vec<InterfaceRate>,
}

/// 进程采样的一行。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    pub cpu_percent: f32,
    pub mem_percent: f32,
    pub rss_bytes: u64,
    pub cmdline: String,
    pub args: Option<String>,
}

/// 网络累计计数器快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NetworkTotals {
    pub received_bytes: u64,
    pub transmitted_bytes: u64,
}

/// 宿主系统与网络采样器。
///
/// 网络采样可独立按秒调用；系统采样复用最近一次网卡速率，避免同一采集
/// 窗口重复刷新网络表。
pub struct HostSampler {
    system: System,
    disks: Disks,
    networks: Networks,
    previous_interfaces: HashMap<String, (u64, u64)>,
    previous_network_at: Option<Instant>,
    latest_interfaces: Vec<InterfaceRate>,
    disk_refresh_round: u64,
    os_name: String,
}

impl HostSampler {
    /// 创建采样器；首次 CPU 使用率需要下一轮刷新才有有效差值。
    pub fn new() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::new()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        Self {
            system,
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            previous_interfaces: HashMap::new(),
            previous_network_at: None,
            latest_interfaces: Vec::new(),
            disk_refresh_round: 0,
            os_name: os_name(),
        }
    }

    /// 刷新网络并返回机器级累计计数器与逐网卡速率。
    pub fn sample_network(&mut self) -> NetworkTotals {
        self.networks.refresh();
        let now = Instant::now();
        let elapsed = self
            .previous_network_at
            .map_or(0.0, |previous| now.duration_since(previous).as_secs_f64());
        let mut current = HashMap::with_capacity(self.networks.list().len());
        let mut interfaces = Vec::with_capacity(self.networks.list().len());
        let mut totals = NetworkTotals::default();
        for (name, data) in self.networks.list() {
            if is_loopback_interface(data) {
                continue;
            }
            let received = data.total_received();
            let transmitted = data.total_transmitted();
            totals.received_bytes = totals.received_bytes.saturating_add(received);
            totals.transmitted_bytes = totals.transmitted_bytes.saturating_add(transmitted);
            let (rx_rate, tx_rate) = self.previous_interfaces.get(name).map_or((0, 0), |last| {
                (
                    rate(last.0, received, elapsed),
                    rate(last.1, transmitted, elapsed),
                )
            });
            interfaces.push(InterfaceRate {
                name: name.clone(),
                rx_bytes_per_sec: rx_rate,
                tx_bytes_per_sec: tx_rate,
            });
            current.insert(name.clone(), (received, transmitted));
        }
        interfaces.sort_by(|left, right| left.name.cmp(&right.name));
        self.previous_interfaces = current;
        self.previous_network_at = Some(now);
        self.latest_interfaces.clone_from(&interfaces);
        totals
    }

    /// 刷新 CPU、内存与磁盘，返回统一系统快照。
    pub fn sample_system(&mut self) -> SystemSnapshot {
        self.system.refresh_cpu_all();
        self.system.refresh_memory();
        self.disk_refresh_round = self.disk_refresh_round.wrapping_add(1);
        if self.disk_refresh_round.is_multiple_of(10) {
            self.disks.refresh_list();
        } else {
            self.disks.refresh();
        }

        let mut filesystems = Vec::with_capacity(self.disks.list().len());
        for disk in self.disks.list() {
            let total = disk.total_space();
            if total == 0 {
                continue;
            }
            filesystems.push(FilesystemUsage {
                mount: disk.mount_point().display().to_string(),
                used_bytes: total.saturating_sub(disk.available_space()),
                total_bytes: total,
            });
        }
        filesystems.sort_by(|left, right| left.mount.cmp(&right.mount));
        let root = system_disk(&filesystems);
        let load = System::load_average();
        let cpu_cores = self
            .system
            .cpus()
            .iter()
            .enumerate()
            .map(|(index, cpu)| CoreUsage {
                index,
                percent: cpu.cpu_usage(),
            })
            .collect();
        SystemSnapshot {
            cpu_percent: self.system.global_cpu_usage(),
            mem_used_bytes: self.system.used_memory(),
            mem_total_bytes: self.system.total_memory(),
            disk_used_bytes: root.map_or(0, |disk| disk.used_bytes),
            disk_total_bytes: root.map_or(0, |disk| disk.total_bytes),
            uptime_secs: System::uptime(),
            os_name: self.os_name.clone(),
            load_avg: (load.one as f32, load.five as f32, load.fifteen as f32),
            swap_used_bytes: self.system.used_swap(),
            swap_total_bytes: self.system.total_swap(),
            cpu_cores,
            filesystems,
            interfaces: self.latest_interfaces.clone(),
        }
    }
}

impl Default for HostSampler {
    fn default() -> Self {
        Self::new()
    }
}

/// 持久进程采样器：保留上一轮进程表，才能计算 CPU 使用率差值。
pub struct ProcessSampler {
    system: System,
    users: Users,
    refresh_round: u64,
}

impl ProcessSampler {
    pub fn new() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::new().with_memory(MemoryRefreshKind::everything()),
        );
        Self {
            system,
            users: Users::new_with_refreshed_list(),
            refresh_round: 0,
        }
    }

    /// 刷新全部进程并返回结构化快照。
    pub fn sample(&mut self) -> Vec<ProcessInfo> {
        self.refresh_round = self.refresh_round.wrapping_add(1);
        if self.refresh_round.is_multiple_of(30) {
            self.users.refresh_list();
        }
        self.system
            .refresh_processes_specifics(ProcessesToUpdate::All, process_refresh_kind());
        self.system.refresh_memory();
        let total_memory = self.system.total_memory();
        let mut processes = Vec::with_capacity(self.system.processes().len());
        for (pid, process) in self.system.processes() {
            let command = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            let args = (!command.is_empty()).then_some(command.clone());
            processes.push(ProcessInfo {
                pid: pid.as_u32(),
                ppid: process.parent().map_or(0, Pid::as_u32),
                user: process
                    .user_id()
                    .and_then(|id| self.users.get_user_by_id(id))
                    .map_or_else(|| "-".to_string(), |user| user.name().to_string()),
                cpu_percent: process.cpu_usage(),
                mem_percent: if total_memory == 0 {
                    0.0
                } else {
                    process.memory() as f32 / total_memory as f32 * 100.0
                },
                rss_bytes: process.memory(),
                cmdline: process.name().to_string_lossy().to_string(),
                args,
            });
        }
        processes.sort_unstable_by_key(|process| process.pid);
        processes
    }

    /// 终止本地进程；`force` 在 Unix 上选择 SIGKILL，否则优先 SIGTERM。
    ///
    /// Windows 没有可供 `sysinfo` 表达的可移植软终止信号，统一走系统终止 API。
    pub fn kill(&mut self, pid: u32, force: bool) -> Result<(), String> {
        let pid = Pid::from_u32(pid);
        self.system
            .refresh_processes(ProcessesToUpdate::Some(&[pid]));
        let process = self
            .system
            .process(pid)
            .ok_or_else(|| format!("本地进程不存在或已经退出，PID: {}", pid.as_u32()))?;
        let killed = if force {
            process.kill()
        } else {
            process
                .kill_with(Signal::Term)
                .unwrap_or_else(|| process.kill())
        };
        if killed {
            Ok(())
        } else {
            Err(format!("操作系统拒绝终止本地进程，PID: {}", pid.as_u32()))
        }
    }
}

impl Default for ProcessSampler {
    fn default() -> Self {
        Self::new()
    }
}

/// 当前宿主平均负载；供容器本机引擎展示宿主负载。
pub fn load_average() -> (f32, f32, f32) {
    let load = System::load_average();
    (load.one as f32, load.five as f32, load.fifteen as f32)
}

/// 当前操作系统的可读名称；平台 API 不提供时退回 Rust 的 OS 标识。
pub fn os_name() -> String {
    System::long_os_version()
        .or_else(System::name)
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}

fn process_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::new()
        .with_cpu()
        .with_memory()
        .with_user(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
}

fn is_loopback_interface(data: &sysinfo::NetworkData) -> bool {
    let networks = data.ip_networks();
    !networks.is_empty() && networks.iter().all(|network| network.addr.is_loopback())
}

fn rate(previous: u64, current: u64, elapsed_secs: f64) -> u64 {
    if elapsed_secs <= 0.0 {
        return 0;
    }
    let delta = current.saturating_sub(previous) as f64;
    let value = delta / elapsed_secs;
    if value.is_finite() && value > 0.0 {
        value.round().min(u64::MAX as f64) as u64
    } else {
        0
    }
}

fn system_disk(filesystems: &[FilesystemUsage]) -> Option<&FilesystemUsage> {
    let current = std::env::current_exe()
        .ok()
        .or_else(|| std::env::current_dir().ok());
    if let Some(current) = current {
        let mut best: Option<&FilesystemUsage> = None;
        for filesystem in filesystems {
            let mount = Path::new(&filesystem.mount);
            if current.starts_with(mount)
                && best.is_none_or(|candidate| mount.as_os_str().len() > candidate.mount.len())
            {
                best = Some(filesystem);
            }
        }
        if best.is_some() {
            return best;
        }
    }
    filesystems
        .iter()
        .find(|filesystem| filesystem.mount == "/")
        .or_else(|| filesystems.first())
}

#[cfg(test)]
mod tests {
    use super::{HostSampler, ProcessSampler};

    #[test]
    fn host_snapshot_has_cores_memory_and_system_disk() {
        if !sysinfo::IS_SUPPORTED_SYSTEM {
            return;
        }
        let mut sampler = HostSampler::new();
        sampler.sample_network();
        let snapshot = sampler.sample_system();
        assert!(!snapshot.cpu_cores.is_empty());
        assert!(snapshot.mem_total_bytes > 0);
        assert!(snapshot.disk_total_bytes > 0);
        assert!(!snapshot.os_name.is_empty());
    }

    #[test]
    fn process_snapshot_contains_current_process() {
        if !sysinfo::IS_SUPPORTED_SYSTEM {
            return;
        }
        let mut sampler = ProcessSampler::new();
        let processes = sampler.sample();
        assert!(
            processes
                .iter()
                .any(|process| process.pid == std::process::id())
        );
    }
}
