//! 远程与本地系统资源统计数据模型。

use serde::{Deserialize, Serialize};

/// 单个逻辑核心的使用率。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct CoreUsage {
    pub index: usize,
    pub percent: f32,
}

/// 单个挂载点的文件系统占用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilesystemUsage {
    pub mount: String,
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// 单块网卡的本轮上下行速率（字节/秒）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InterfaceRate {
    pub name: String,
    pub rx_bytes_per_sec: u64,
    pub tx_bytes_per_sec: u64,
}

/// 远端进程快照的一行。
///
/// `cmdline` 保存 `ps ... comm` 给出的进程名；完整命令行需要额外读取
/// `/proc/<pid>/cmdline`，每 3 秒对 50 个进程各开一次 exec 会放大远端开销，
/// 因此面板只展示短名，右键"复制命令"拼出 `pid + comm` 的可辨识文本。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    pub cpu_percent: f32,
    pub mem_percent: f32,
    pub rss_bytes: u64,
    pub cmdline: String,
}
