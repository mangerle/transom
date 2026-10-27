//! 实例套接字生命周期管理与请求监听转发。

use super::types::{DIR_LINE_PREFIX, LaunchRequest, request_lines};
use anyhow::{Context as _, anyhow};
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::net::UnixListener;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use transom_core::{AppEvent, DeepLinkRequest, is_ssh_url, paths};

/// 单条深链请求的字节上限：地址串远小于此，超限即视为异常输入截断丢弃，
/// 避免对端用长报文撑大读取侧内存。
pub(crate) const MAX_REQUEST_BYTES: u64 = 4096;

/// 等待一次请求写入的超时：本地对端进程假死时不能永久占住接收循环。
pub(crate) const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// 首个实例的监听端点；套接字文件的清理由监听任务持有的守卫负责。
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
        // 删除失败不影响退出：残留文件会在下次启动时按"对端无响应即陈旧"清理。
        let _ = std::fs::remove_file(&self.path);
    }
}

/// 决定本进程的角色：绑定套接字成功即成为主实例；失败说明已有实例在跑，
/// 把请求转交后返回 `None`，调用方应当直接退出。
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
            // 上个实例崩溃留下的陈旧套接字：清理后重新绑定，本进程改当主实例。
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

/// 绑定套接字并把权限收紧到仅属主可读写。
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

/// 探测套接字对端是否真有进程在监听：连接失败即认为文件是陈旧残留。
pub(crate) fn probe_alive(socket_path: &Path) -> bool {
    StdUnixStream::connect(socket_path).is_ok()
}

/// 把请求逐行写入已运行实例的套接字；没有任何请求时发一个空行，语义是"仅唤起"。
fn forward(socket_path: &Path, request: &LaunchRequest) -> io::Result<()> {
    let stream = StdUnixStream::connect(socket_path)?;
    stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
    let mut writer = &stream;
    let lines = request_lines(request);
    for line in &lines {
        std::io::Write::write_all(&mut writer, line.as_bytes())?;
        std::io::Write::write_all(&mut writer, b"\n")?;
    }
    if lines.is_empty() {
        std::io::Write::write_all(&mut writer, b"\n")?;
    }
    std::io::Write::flush(&mut writer)
}

/// 接收循环：逐个处理连接，一条连接上的所有请求读完为止。
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

/// 读完一条连接上的所有请求并逐条投递。
pub(crate) async fn accept_stream<R: AsyncRead + Unpin>(
    stream: &mut R,
    event_tx: &flume::Sender<AppEvent>,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream.take(MAX_REQUEST_BYTES));
    loop {
        let mut line = String::new();
        let read = tokio::time::timeout(REQUEST_TIMEOUT, reader.read_line(&mut line))
            .await
            .map_err(|_| anyhow!("等待深链请求超时，已丢弃在途请求"))?
            .context("读取深链请求失败")?;
        if read == 0 {
            return Ok(());
        }
        let request = line.trim_end_matches(['\n', '\r']).trim();
        accept_request(request, event_tx)?;
    }
}

/// 解析并投递单条请求。
fn accept_request(request: &str, event_tx: &flume::Sender<AppEvent>) -> anyhow::Result<()> {
    let request = parse_request_line(request)?;
    log::info!("收到外部唤起请求: {request:?}");
    if event_tx.try_send(AppEvent::DeepLink { request }).is_err() {
        log::warn!("深链唤起未能投递到界面事件通道（队列已满），本次请求已丢弃");
    }
    Ok(())
}

/// 把一行请求文本洗成 [`DeepLinkRequest`]：协议只认三种形态，其余一律拒绝。
fn parse_request_line(request: &str) -> anyhow::Result<DeepLinkRequest> {
    if request.is_empty() {
        return Ok(DeepLinkRequest::Activate);
    }
    if is_ssh_url(request) {
        return Ok(DeepLinkRequest::Ssh {
            url: request.to_string(),
        });
    }
    if let Some(path) = request.strip_prefix(DIR_LINE_PREFIX) {
        let candidate = Path::new(path);
        if candidate.is_absolute() && candidate.is_dir() {
            return Ok(DeepLinkRequest::Dir {
                path: candidate.to_path_buf(),
            });
        }
        return Err(anyhow!(
            "目录请求不是现存绝对路径，已拒绝，原始输入: {request}"
        ));
    }
    Err(anyhow!("深链请求协议形态未知，已拒绝，原始输入: {request}"))
}
