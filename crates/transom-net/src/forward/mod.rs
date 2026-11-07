//! 本地/远程/动态（SOCKS5）端口转发。
//!
//! 设计原理（Why）：
//! - 每条转发任务持有取消信号与连接集合，停止时统一中止在途连接；
//! - accept 并发受信号量上限约束，满员时让内核 backlog 反压，避免 fd 打满；
//! - 动态转发把 SOCKS5 握手与 UDP 关联放在 socks5.rs，主文件只保留任务生命周期
//!   与 TCP/远程转发的编排。

mod socks5;

use socks5::{relay_local_connection, start_dynamic};

use crate::client::SshRemote;
use futures::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

use tokio::sync::{Semaphore, watch};
use tokio::task::{JoinHandle, JoinSet};
use transom_core::{
    AppError, BindAddress, ErrorCode, ForwardKind, ForwardRule, SocksAuth, TargetAddress,
};

/// accept 失败后的退避窗口：EMFILE 等瞬时错误下避免 100% CPU 忙等。
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(500);

/// 单条本地/动态转发的最大并发连接数。
///
/// 满员时暂停 accept，让内核 backlog 形成反压；无上限 spawn 会把文件描述符
/// 与任务表打满，EMFILE 后还会退化成 accept 失败空转。
const MAX_FORWARD_CONNECTIONS: usize = 64;

struct RemoteForwardInfo {
    remote: SshRemote,
    host: String,
    port: u16,
}

pub struct ForwardTask {
    pub bound_port: Option<u16>,
    stop: watch::Sender<bool>,
    join: Option<JoinHandle<()>>,
    remote_forward: Option<RemoteForwardInfo>,
}

impl ForwardTask {
    pub async fn stop(mut self) {
        let _ = self.stop.send(true);
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
        if let Some(info) = self.remote_forward.take() {
            info.remote.unregister_remote_target(info.port);
            let _ = info
                .remote
                .cancel_tcpip_forward(&info.host, info.port)
                .await;
        }
    }
}

impl Drop for ForwardTask {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(info) = self.remote_forward.take() {
            log::warn!(
                "端口转发任务未经 stop 被丢弃，远端监听端口 {} 可能保留到会话断开",
                info.port
            );
            info.remote.unregister_remote_target(info.port);
        }
    }
}

pub async fn start_forward(
    remote: &SshRemote,
    rule: &ForwardRule,
) -> Result<ForwardTask, AppError> {
    match &rule.kind {
        ForwardKind::Local { bind, target } => start_local(remote, bind, target).await,
        ForwardKind::Dynamic { bind, socks } => {
            if !matches!(socks.auth, SocksAuth::None) {
                return Err(AppError::with_detail(
                    ErrorCode::ForwardFailed,
                    "动态转发暂不支持 SOCKS 认证",
                ));
            }
            start_dynamic(remote, bind).await
        }
        ForwardKind::Remote { bind, target } => start_remote(remote, bind, target).await,
        _ => Err(AppError::with_detail(
            ErrorCode::ForwardFailed,
            "不支持的转发模式",
        )),
    }
}

async fn start_local(
    remote: &SshRemote,
    bind: &BindAddress,
    target: &TargetAddress,
) -> Result<ForwardTask, AppError> {
    let listener = bind_listener(bind).await?;
    let bound_port = listener.local_addr().ok().map(|address| address.port());
    let (stop, stop_rx) = watch::channel(false);
    let remote = remote.clone();
    let target_host = target.host.clone();
    let target_port = target.port;

    let join = tokio::spawn(serve_listener(
        listener,
        stop_rx,
        Arc::new(move |tcp| {
            let remote = remote.clone();
            let host = target_host.clone();
            Box::pin(async move {
                relay_local_connection(remote, tcp, host, target_port).await;
            })
        }),
    ));

    Ok(ForwardTask {
        bound_port,
        stop,
        join: Some(join),
        remote_forward: None,
    })
}

/// 本地/动态转发的监听骨架：并发准入、accept 退避与停止回收。
///
/// 没有这层收敛时，两个入口各自复制循环，且都没有并发上限与错误退避：
/// 被本机或外部进程刷连接时会耗尽 FD，accept 失败后还会立即重试空转。
pub(super) async fn serve_listener(
    listener: TcpListener,
    mut stop_rx: watch::Receiver<bool>,
    handler: Arc<dyn Fn(TcpStream) -> BoxFuture<'static, ()> + Send + Sync>,
) {
    let mut connections = JoinSet::new();
    let slots = Arc::new(Semaphore::new(MAX_FORWARD_CONNECTIONS));
    loop {
        let permit = match Arc::clone(&slots).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                // 满员：停止 accept，等一条连接结束或停止信号。内核 backlog
                // 负责在 accept 暂停期间挡住新连接，形成天然反压。
                tokio::select! {
                    _ = stop_rx.changed() => break,
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
                continue;
            }
        };
        let accepted = tokio::select! {
            _ = stop_rx.changed() => break,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok((tcp, _)) => {
                let future = handler(tcp);
                connections.spawn(async move {
                    let _permit = permit;
                    future.await;
                });
            }
            Err(error) => {
                drop(permit);
                log::warn!("转发监听接受连接失败，将退避后重试，原因: {error}");
                tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
            }
        }
        while connections.try_join_next().is_some() {}
    }
    // 停止转发必须同时断开在途连接：abort 取消 copy_bidirectional 后，
    // 本地套接字与 SSH 通道随 future 一起释放，客户端立即看到连接关闭。
    abort_connections(&mut connections).await;
}

async fn start_remote(
    remote: &SshRemote,
    bind: &BindAddress,
    target: &TargetAddress,
) -> Result<ForwardTask, AppError> {
    let actual_port = remote.tcpip_forward(&bind.host, bind.port).await?;
    let actual_port = actual_port.min(u16::MAX as u32) as u16;
    remote.register_remote_target(actual_port, target.host.clone(), target.port);
    let (stop, _stop_rx) = watch::channel(false);

    Ok(ForwardTask {
        bound_port: Some(actual_port),
        stop,
        join: None,
        remote_forward: Some(RemoteForwardInfo {
            remote: remote.clone(),
            host: bind.host.clone(),
            port: actual_port,
        }),
    })
}

pub(super) async fn bind_listener(bind: &BindAddress) -> Result<TcpListener, AppError> {
    TcpListener::bind((bind.host.as_str(), bind.port))
        .await
        .map_err(forward_error)
}

pub(super) fn forward_error(error: std::io::Error) -> AppError {
    AppError::with_detail(ErrorCode::ForwardFailed, error.to_string())
}

/// 中止并等待全部在途连接退出。
///
/// abort 会取消 `copy_bidirectional`/协商 future，套接字与 SSH 通道随
/// future 一起释放；`join_next` 再逐个回收，保证返回时任务集合已清空。
async fn abort_connections(connections: &mut JoinSet<()>) {
    connections.abort_all();
    while connections.join_next().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn serve_listener_caps_concurrent_connections() {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("绑定本地端口");
        let address = listener.local_addr().expect("读取监听地址");
        let (stop, stop_rx) = watch::channel(false);
        let (release_tx, release_rx) = watch::channel(false);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let handler_active = Arc::clone(&active);
        let handler_peak = Arc::clone(&peak);

        let join = tokio::spawn(serve_listener(
            listener,
            stop_rx,
            Arc::new(move |_tcp| {
                let active = Arc::clone(&handler_active);
                let peak = Arc::clone(&handler_peak);
                let mut release_rx = release_rx.clone();
                Box::pin(async move {
                    let current = active.fetch_add(1, Ordering::Relaxed) + 1;
                    peak.fetch_max(current, Ordering::Relaxed);
                    let _ = release_rx.wait_for(|released| *released).await;
                    active.fetch_sub(1, Ordering::Relaxed);
                })
            }),
        ));

        let mut clients = Vec::new();
        for _ in 0..(MAX_FORWARD_CONNECTIONS + 8) {
            if let Ok(client) = TcpStream::connect(address).await {
                clients.push(client);
            }
        }
        for _ in 0..200 {
            if peak.load(Ordering::Relaxed) >= MAX_FORWARD_CONNECTIONS {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        assert_eq!(
            peak.load(Ordering::Relaxed),
            MAX_FORWARD_CONNECTIONS,
            "并发连接应恰好吃满上限"
        );
        assert!(
            peak.load(Ordering::Relaxed) <= MAX_FORWARD_CONNECTIONS,
            "并发连接不得超过上限"
        );

        let _ = stop.send(true);
        let _ = join.await;
        let _ = release_tx.send(true);
        drop(clients);
    }

    #[tokio::test]
    async fn abort_connections_cancels_in_flight_tasks() {
        let cancelled = Arc::new(AtomicUsize::new(0));
        let mut connections = JoinSet::new();
        for _ in 0..3 {
            let cancelled = cancelled.clone();
            connections.spawn(async move {
                struct Guard(Arc<AtomicUsize>);
                impl Drop for Guard {
                    fn drop(&mut self) {
                        self.0.fetch_add(1, Ordering::Relaxed);
                    }
                }
                let _guard = Guard(cancelled);
                std::future::pending::<()>().await;
            });
        }
        tokio::task::yield_now().await;
        abort_connections(&mut connections).await;
        assert_eq!(
            cancelled.load(Ordering::Relaxed),
            3,
            "停止转发必须取消全部在途连接"
        );
        assert!(connections.is_empty(), "回收后任务集合必须为空");
    }
}
