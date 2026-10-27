//! 控制平面：命令与事件总线、连接与任务生命周期、停机收尾。
//!
//! 设计原理（Why）：
//! - 组合根与本模块之间的接口只有三条有界通道：命令 256、事件 1024、
//!   内部 64（容量与背压策略见 `Controller::start` 的注释）；
//! - 主循环本体只做三件事：收事件、委托到 [`commands`] 与 [`internal`]
//!   处理、判定是否停机。状态表在 [`state::ControllerState`]、命令处理在
//!   [`commands`]、接入与断开在 [`internal`]：单函数不再随功能膨胀，
//!   四类连接的接入路径也收敛为一条；
//! - 停机先投递命令并限时等待收尾完成（`Controller::shutdown`），收尾
//!   任务全部纳入 JoinSet 统一等待，任务生命周期全程可追踪。

use std::sync::Arc;
use std::sync::mpsc;
use tokio::runtime::Runtime;
use transom_core::{
    AppCommand, AppError, AppEvent, ConnectionId, ErrorCode, ForwardId, KnownHostsStore,
    SecretStore, SftpOperation, TabId, TerminalCommand, TerminalIo, TransferId,
};
use transom_net::{ForwardTask, SftpClient, SshSession, ZmodemConfig};
use transom_sync::CloudSyncService;

mod commands;
mod host_exec;
mod internal;
mod lifecycle;
mod pty;
mod samplers;
mod sftp_exec;
mod sftp_finish;
mod state;
mod sync_exec;

use state::{ControllerCtx, ControllerState};

pub struct Controller {
    _runtime: Runtime,
    command_tx: flume::Sender<AppCommand>,
    event_tx: flume::Sender<AppEvent>,
    event_rx: flume::Receiver<AppEvent>,
    /// 控制平面主循环的退出通知：`shutdown` 限时等待它，确保远端断开与
    /// 转发停止真正执行完，而不是随进程退出被粗暴取消。
    shutdown_rx: mpsc::Receiver<()>,
}

impl Controller {
    pub fn start(
        known_hosts: Arc<dyn KnownHostsStore>,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, AppError> {
        // 三条通道的容量与背压策略（全工程禁止无界通道）：
        // - 命令 256：用户命令低频，UI 侧 try_send 快速失败并提示；
        // - 事件 1024：可覆盖状态样本（流量/统计/进度）由生产者 try_send，
        //   通道满即丢弃、不占位；状态迁移事件（连接、转发、失败）用
        //   send_async 反压，UI 停滞时拖慢发送方而非内存无限积压；
        // - 内部 64：连接完成/断开通知，低频。
        let (command_tx, command_rx) = flume::bounded::<AppCommand>(256);
        let (event_tx, event_rx) = flume::bounded::<AppEvent>(1024);
        let (internal_tx, internal_rx) = flume::bounded::<Internal>(64);
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| AppError::with_detail(ErrorCode::Io, error.to_string()))?;

        // 云同步服务随控制器构建一次：HTTP 客户端连接池全生命周期复用。
        let cloud_sync = Arc::new(CloudSyncService::new(secrets)?);
        // 事件发送端留一份克隆给应用外壳：深链监听不在控制平面内，
        // 但唤起请求要走同一条事件总线。
        let outer_event_tx = event_tx.clone();
        let root = runtime.spawn(run_controller(ControlPlaneWiring {
            known_hosts,
            command_rx,
            internal_tx,
            internal_rx,
            event_tx,
            cloud_sync,
            shutdown_tx,
        }));
        // 根控制器是整个应用的控制平面：panic 后命令与事件全部停摆，
        // 必须可观察（日志上报）而不是静默失效。
        runtime.spawn(async move {
            match root.await {
                Ok(()) => log::info!("控制器主循环已正常退出，控制平面停机"),
                Err(error) if error.is_panic() => {
                    log::error!("控制器主循环异常崩溃，控制平面已失效: {error}")
                }
                Err(error) => log::error!("控制器主循环任务失败，控制平面已失效: {error}"),
            }
        });

        Ok(Self {
            _runtime: runtime,
            command_tx,
            event_tx: outer_event_tx,
            event_rx,
            shutdown_rx,
        })
    }

    /// 优雅停机：驱动控制平面完成收尾，再限时关闭运行时。
    ///
    /// 设计原理（Why）：
    /// - UI 退出时若直接丢弃控制器，运行时会把控制平面任务与全部连接粗暴
    ///   取消：远端收不到 DISCONNECT、转发任务无人停止，"优雅停机"分支
    ///   形同不存在。这里先投递 `Shutdown` 命令并等待主循环回报收尾完成
    ///   （断开远端、停转发、限时等待收尾任务），再关运行时；
    /// - `Runtime` 的默认析构会无限等待阻塞线程池，个别卡死的系统调用
    ///   （串口/文件 IO）能把退出拖成假死，因此运行时用 2 秒上限关闭；
    /// - 等待上限 5 秒：控制平面自身的收尾预算是 3 秒（`shutdown_all`），
    ///   留 2 秒余量；超时只告警不阻塞，进程退出由内核回收残余线程。
    pub fn shutdown(self) {
        let Self {
            _runtime,
            command_tx,
            shutdown_rx,
            ..
        } = self;
        match command_tx.send_timeout(
            TerminalCommand::Shutdown.into(),
            std::time::Duration::from_millis(500),
        ) {
            Ok(()) => {}
            Err(flume::SendTimeoutError::Timeout(_)) => {
                log::warn!("命令队列已满，停机命令未能投递，控制平面将随通道关闭退出");
            }
            Err(flume::SendTimeoutError::Disconnected(_)) => {
                log::debug!("控制平面已停止接收命令，跳过停机命令投递");
            }
        }
        // 关闭命令通道兜底：即使停机命令未投递成功，主循环也会因
        // RecvError 走停机分支。
        drop(command_tx);
        match shutdown_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(()) => log::info!("控制平面已完成优雅停机，开始关闭运行时"),
            Err(_) => log::warn!("等待控制平面停机超过 5 秒，直接关闭运行时"),
        }
        _runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    }

    pub fn command_sender(&self) -> flume::Sender<AppCommand> {
        self.command_tx.clone()
    }

    pub fn event_receiver(&self) -> flume::Receiver<AppEvent> {
        self.event_rx.clone()
    }

    /// 事件总线发送端：供应用外壳（深链监听）投递与连接无关的界面事件。
    pub fn event_sender(&self) -> flume::Sender<AppEvent> {
        self.event_tx.clone()
    }

    /// 控制平面运行时句柄：应用外壳的辅助常驻任务（深链监听）挂到这里，
    /// 与控制器共享线程池并随控制器一起停机，不再单独开一个运行时。
    pub fn runtime(&self) -> &Runtime {
        &self._runtime
    }
}

/// 内部完成事件：建连任务、SFTP/转发建立、断开通知回流主循环。
///
/// 四种连接完成曾各占一个变体，接入逻辑因此复制四份；收敛为
/// [`ReadyConnection`] 承载的单一变体后，接入路径只剩一条。
enum Internal {
    Connected {
        tab_id: TabId,
        connection_id: ConnectionId,
        kind: ReadyConnection,
        /// ZMODEM 监督层配置：随连接命令携带，控制器不回读设置文件。
        zmodem: ZmodemConfig,
    },
    Disconnected {
        tab_id: TabId,
        connection_id: ConnectionId,
    },
    /// SFTP 客户端在任务内完成初始化后的提交：主循环校验连接归属后
    /// 才写入客户端缓存并派发操作，分发循环本身不为打开客户端等待。
    SftpReady {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        op: SftpOperation,
        result: Result<SftpClient, AppError>,
    },
    /// 端口转发在任务内建立后的提交：主循环校验连接归属后登记任务
    /// 并向界面回报状态。
    ForwardReady {
        tab_id: TabId,
        connection_id: ConnectionId,
        rule_id: ForwardId,
        result: Result<ForwardTask, AppError>,
    },
}

/// 建连任务的完成形态：SSH 会话，或"IO + 退出通知"的本地/串口/Telnet。
enum ReadyConnection {
    Ssh(SshSession),
    Local {
        io: TerminalIo,
        join: tokio::sync::oneshot::Receiver<()>,
    },
    Serial {
        io: TerminalIo,
        join: tokio::sync::oneshot::Receiver<()>,
    },
    Telnet {
        io: TerminalIo,
        join: tokio::sync::oneshot::Receiver<()>,
    },
}

impl ReadyConnection {
    /// 日志用的来源名。
    fn label(&self) -> &'static str {
        match self {
            Self::Ssh(_) => "SSH",
            Self::Local { .. } => "本地终端",
            Self::Serial { .. } => "串口",
            Self::Telnet { .. } => "Telnet",
        }
    }
}

enum Incoming {
    Command(Result<AppCommand, flume::RecvError>),
    Internal(Result<Internal, flume::RecvError>),
}

/// 控制平面主循环的输入装配：句柄一次性建立、随控制器存活。
///
/// 参数超过五个后收敛为结构体：平铺的八个参数让调用点难以核对顺序，
/// 新增接线时也容易漏项。
struct ControlPlaneWiring {
    known_hosts: Arc<dyn KnownHostsStore>,
    command_rx: flume::Receiver<AppCommand>,
    internal_tx: flume::Sender<Internal>,
    internal_rx: flume::Receiver<Internal>,
    event_tx: flume::Sender<AppEvent>,
    cloud_sync: Arc<CloudSyncService>,
    /// 主循环退出（含停机收尾完成）通知；`Controller::shutdown` 限时等待它。
    shutdown_tx: mpsc::Sender<()>,
}

async fn run_controller(wiring: ControlPlaneWiring) {
    let ControlPlaneWiring {
        known_hosts,
        command_rx,
        internal_tx,
        internal_rx,
        event_tx,
        cloud_sync,
        shutdown_tx,
    } = wiring;
    let ctx = ControllerCtx {
        known_hosts,
        event_tx,
        internal_tx,
        cloud_sync,
        action_permits: Arc::new(tokio::sync::Semaphore::new(
            host_exec::MAX_CONCURRENT_ACTIONS,
        )),
    };
    let mut state = ControllerState::new();

    loop {
        // 批次集合是否有在途任务：select 的分支条件不能与 join_next 的
        // 可变借用同字段并存，先取布尔快照。
        let has_host_actions = !state.host_action_tasks.is_empty();
        let has_monitors = !state.monitor_set.is_empty();
        let incoming = tokio::select! {
            command = command_rx.recv_async() => Incoming::Command(command),
            message = internal_rx.recv_async() => Incoming::Internal(message),
            // 批次任务收自己：异常退出要可见，否则界面会停在"进行中"。
            joined = state.host_action_tasks.join_next(), if has_host_actions => {
                // 批次任务内部已为每条动作补齐结果，这里只记录批次级异常。
                if let Some(Err(error)) = joined {
                    log::error!("主机动作批次任务异常退出，本批结果可能不完整: {error}");
                }
                continue;
            }
            joined = state.monitor_set.join_next(), if has_monitors => {
                if let Some(Err(error)) = joined {
                    log::error!("进程面板任务异常退出: {error}");
                }
                continue;
            }
        };

        let control = match incoming {
            Incoming::Command(Ok(command)) => {
                commands::handle_command(&mut state, &ctx, command).await
            }
            // 停机命令或通道关闭（UI 退出/通道断连）：同一路径做限时收尾。
            Incoming::Command(Err(_)) | Incoming::Internal(Err(_)) => {
                commands::LoopControl::Shutdown
            }
            Incoming::Internal(Ok(internal)) => {
                internal::handle_internal(&mut state, &ctx, internal).await;
                commands::LoopControl::Continue
            }
        };

        if matches!(control, commands::LoopControl::Shutdown) {
            commands::shutdown(&mut state, &shutdown_tx).await;
            break;
        }
    }
}
