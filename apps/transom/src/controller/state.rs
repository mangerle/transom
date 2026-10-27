//! 控制平面的状态表与共享上下文。
//!
//! 设计原理（Why）：控制循环此前把十二张任务/连接表平铺在函数栈上，四个
//! 连接接入分支各自把整组表逐个重列一遍——新增一张表要改五处，接入路径
//! 长期存在四份复制。收敛为 [`ControllerState`] 后，新增状态只改本文件与
//! 相关处理函数，接入路径只剩一条。
//!
//! [`ControllerCtx`] 收纳随控制器一次性建立的共享句柄；处理函数只读它们，
//! 不再从主循环逐个透传或克隆。

use super::Internal;
use super::lifecycle::ConnectingTab;
use super::samplers::LocalTrafficTabs;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::JoinSet;
use transom_core::{AppEvent, ConnectionId, ForwardId, KnownHostsStore, TabId, TransferId};
use transom_net::{ForwardTask, SftpClient, SshRemote, ZmodemControl};
use transom_sync::CloudSyncService;

/// 控制平面的全部可变状态表。
pub(super) struct ControllerState {
    /// 当前连接表：标签 -> 生效的连接 id。
    pub(super) current_connections: HashMap<TabId, ConnectionId>,
    /// 远端句柄表：标签 -> (连接 id, SSH 远端)。
    pub(super) remotes: HashMap<TabId, (ConnectionId, SshRemote)>,
    /// SFTP 客户端缓存：按标签复用已打开的客户端。
    pub(super) sftp_clients: HashMap<TabId, SftpClient>,
    /// 在途 SFTP 操作任务：取消或关机时中止。
    pub(super) sftp_tasks: HashMap<(TabId, TransferId), tokio::task::JoinHandle<()>>,
    /// 端口转发表：标签 -> 规则 -> 转发任务。
    pub(super) forwards: HashMap<TabId, HashMap<ForwardId, ForwardTask>>,
    /// 网络流量采样任务：按标签跟踪，断开/关闭时中止。
    pub(super) traffic_tasks: HashMap<TabId, tokio::task::JoinHandle<()>>,
    /// 公钥部署任务：按标签跟踪。
    pub(super) key_deploy_tasks: HashMap<TabId, tokio::task::JoinHandle<()>>,
    /// 远端系统统计采样任务：按标签跟踪，断开/关闭时中止，杜绝孤儿任务。
    pub(super) stats_tasks: HashMap<TabId, tokio::task::JoinHandle<()>>,
    /// 云同步任务：全局同一时刻只允许一个在途，进行中时拒绝新命令。
    pub(super) sync_task: Option<tokio::task::JoinHandle<()>>,
    /// 收尾任务集合（断开远端、转发优雅停止）：集中纳管 JoinHandle，
    /// 停机时统一限时等待，任务不再脱缰。
    pub(super) cleanup_set: JoinSet<()>,
    /// 待连接意图表：发起连接即登记，完成事件必须匹配当前意图才接入；
    /// 同标签的新连接会顶掉并取消旧建连任务，迟到的旧结果被拒收。
    pub(super) connecting: HashMap<TabId, ConnectingTab>,
    /// 本地终端共享一份系统统计采集：/proc 读取与 df 子进程按机器计一次，
    /// 采样器在首个本地连接时拉起、标签清空后退出。
    pub(super) local_stats_tabs: Arc<std::sync::Mutex<Vec<(TabId, ConnectionId)>>>,
    pub(super) local_stats_task: Option<tokio::task::JoinHandle<()>>,
    /// 本地网络流量同样是机器级属性：共享采样按机器计一份读取，逐标签广播。
    pub(super) local_traffic_tabs: LocalTrafficTabs,
    pub(super) local_traffic_task: Option<tokio::task::JoinHandle<()>>,
    /// ZMODEM：每标签一个事件转发任务与一个控制句柄；连接接入时建立，
    /// 断开/关标签/重连时随 stop_zmodem 回收。
    pub(super) zmodem_tasks: HashMap<TabId, tokio::task::JoinHandle<()>>,
    pub(super) zmodem_controls: HashMap<TabId, ZmodemControl>,
    /// 主机动作（拨测/唤醒）批次任务：停机时统一等待。
    pub(super) host_action_tasks: JoinSet<()>,
    /// 进程面板的一次性采样/终止任务：与主机动作同样纳入 JoinSet，
    /// 停机时统一等待，避免远端 exec 还在途时进程直接退出。
    pub(super) monitor_set: JoinSet<()>,
}

impl ControllerState {
    /// 空状态：全部表为空，收尾与批次集合就位。
    pub(super) fn new() -> Self {
        Self {
            current_connections: HashMap::new(),
            remotes: HashMap::new(),
            sftp_clients: HashMap::new(),
            sftp_tasks: HashMap::new(),
            forwards: HashMap::new(),
            traffic_tasks: HashMap::new(),
            key_deploy_tasks: HashMap::new(),
            stats_tasks: HashMap::new(),
            sync_task: None,
            cleanup_set: JoinSet::new(),
            connecting: HashMap::new(),
            local_stats_tabs: Arc::new(std::sync::Mutex::new(Vec::new())),
            local_stats_task: None,
            local_traffic_tabs: Arc::new(std::sync::Mutex::new(Vec::new())),
            local_traffic_task: None,
            zmodem_tasks: HashMap::new(),
            zmodem_controls: HashMap::new(),
            host_action_tasks: JoinSet::new(),
            monitor_set: JoinSet::new(),
        }
    }
}

/// 控制平面随控制器一次性建立的共享句柄。
pub(super) struct ControllerCtx {
    pub(super) known_hosts: Arc<dyn KnownHostsStore>,
    pub(super) event_tx: flume::Sender<AppEvent>,
    pub(super) internal_tx: flume::Sender<Internal>,
    pub(super) cloud_sync: Arc<CloudSyncService>,
    /// 主机动作的跨批次并发闸门：用户连点两次也不会翻倍占用在途连接数。
    pub(super) action_permits: Arc<tokio::sync::Semaphore>,
}
