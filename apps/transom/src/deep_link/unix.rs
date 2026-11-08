//! Unix 单实例传输：私有域套接字。
//!
//! Linux 与 macOS 共用 Unix domain socket；macOS 的 LaunchServices URL 事件由
//! GPUI 回调进入同一事件通道，命令行请求则由启动参数解析后走本模块转发。

use super::protocol::{accept_stream, write_request};
use super::types::LaunchRequest;
use anyhow::{Context as _, anyhow};
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream};
use std::path::{Path, PathBuf};
use tokio::net::UnixListener;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use transom_core::AppEvent;
use transom_platform::paths;

/// 首个实例的监听端点；套接字文件清理由监听任务持有的守卫负责。
pub struct PrimaryInstance {
    listener: StdUnixListener,
    socket: SocketGuard,
}

/// 套接字文件守护：RAII 删除，监听任务结束时不留陈旧文件。
pub(crate) struct SocketGuard {
    pub(crate) path: PathBuf,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// 决定本进程的角色：绑定套接字成功即成为主实例；失败说明已有实例在跑，
/// 把请求转交后返回 `None`。
pub fn acquire_primary(request: &LaunchRequest) -> anyhow::Result<Option<PrimaryInstance>> {
    acquire_primary_at(&paths::instance_socket_path(), request)
}

/// [`acquire_primary`] 的可注入路径版本。
pub(crate) fn acquire_primary_at(
    socket_path: &Path,
    request: &LaunchRequest,
) -> anyhow::Result<Option<PrimaryInstance>> {
    match bind_listener(socket_path) {
        Ok(primary) => Ok(Some(primary)),
        Err(bind_error) if bind_error.kind() == io::ErrorKind::AddrInUse => {
            if probe_alive(socket_path) {
                forward(socket_path, request).with_context(|| {
                    format!("向已运行实例转交唤起请求失败，套接字: {socket_path:?}")
                })?;
                return Ok(None);
            }
            std::fs::remove_file(socket_path)
                .with_context(|| format!("清理陈旧实例套接字失败，路径: {socket_path:?}"))?;
            let primary = bind_listener(socket_path)
                .with_context(|| format!("清理陈旧套接字后重新绑定失败，路径: {socket_path:?}"))?;
            Ok(Some(primary))
        }
        Err(error) => Err(anyhow!(
            "绑定实例套接字失败，路径: {socket_path:?}，原因: {error}"
        )),
    }
}

impl PrimaryInstance {
    /// 把监听端点接入控制平面运行时开始接收深链请求。
    pub fn serve(self, runtime: &Runtime, event_tx: flume::Sender<AppEvent>) -> JoinHandle<()> {
        let Self { listener, socket } = self;
        let path = socket.path.clone();
        let _guard = runtime.enter();
        let join = match UnixListener::from_std(listener) {
            Ok(async_listener) => runtime.spawn(run_loop(async_listener, event_tx, socket)),
            Err(error) => {
                log::error!(
                    "实例套接字切换到异步监听失败，深链唤起不可用，路径: {path:?}，原因: {error}"
                );
                drop(socket);
                return runtime.spawn(async {});
            }
        };
        runtime.spawn(async move {
            match join.await {
                Ok(()) => log::info!("深链监听任务已退出"),
                Err(error) if error.is_panic() => {
                    log::error!("深链监听任务异常崩溃，外部唤起将不再响应: {error}")
                }
                Err(error) => log::warn!("深链监听任务被取消: {error}"),
            }
        })
    }
}

fn bind_listener(socket_path: &Path) -> io::Result<PrimaryInstance> {
    let parent = socket_path.parent().unwrap_or(Path::new("."));
    if !parent.as_os_str().is_empty() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let listener = StdUnixListener::bind(socket_path)?;
    listener.set_nonblocking(true)?;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;
    Ok(PrimaryInstance {
        listener,
        socket: SocketGuard {
            path: socket_path.to_path_buf(),
        },
    })
}

/// 探测套接字对端是否真有进程在监听。
pub(crate) fn probe_alive(socket_path: &Path) -> bool {
    StdUnixStream::connect(socket_path).is_ok()
}

fn forward(socket_path: &Path, request: &LaunchRequest) -> io::Result<()> {
    let stream = StdUnixStream::connect(socket_path)?;
    stream.set_write_timeout(Some(super::protocol::REQUEST_TIMEOUT))?;
    let mut writer = &stream;
    write_request(&mut writer, request)
}

async fn run_loop(listener: UnixListener, event_tx: flume::Sender<AppEvent>, socket: SocketGuard) {
    loop {
        match listener.accept().await {
            Ok((mut stream, _)) => {
                if let Err(error) = accept_stream(&mut stream, &event_tx).await {
                    log::warn!("深链请求处理失败，原因: {error}");
                }
            }
            Err(error) => {
                log::error!(
                    "实例套接字接受连接失败，路径: {:?}，原因: {error}",
                    socket.path
                );
            }
        }
    }
}
