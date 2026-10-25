//! 桌面深链（`ssh://`）与单实例唤起：Unix 域套接字实现的进程间转发。
//!
//! 设计原理（Why）：
//! - gpui 的 Linux 后端只把 `open_urls` 回调存下来、没有任何调用点（X11 与
//!   Wayland 都不产生该事件），`App::activate` 在 Linux 上也是空实现，因此
//!   深链与单实例必须在应用侧自建，不能等平台层；
//! - "绑定即锁"：对固定路径的套接字 `bind()` 成功即说明自己是首个实例，失败
//!   说明已有实例在跑，于是不再启动界面，只把地址写过去后退出。相比 pid 文件
//!   加 flock 的方案，少了额外依赖，也不必处理"进程已死但 pid 被复用"的误判；
//! - 转发请求复用控制器已有的事件通道（有界 1024），界面侧不必新增一条泵。
//!
//! 代价与局限（Trade-offs）：
//! - 清理陈旧套接字（上次崩溃残留）与重新绑定之间存在理论竞态：两个进程同时
//!   冷启动且套接字已失效时，后者可能删掉前者刚创建的套接字文件。该窗口只在
//!   "崩溃后立刻双开"时出现，后果是一个实例收不到后续深链、重启即恢复，因此
//!   接受这点复杂度换取零新增依赖；
//! - 唤起只做到"打开新标签"与 `App::activate`（Linux 为空操作），无法把已
//!   最小化的窗口提到前台——gpui 未暴露 raise 接口，绕过它需要 X11/Wayland
//!   专有代码，收益不抵成本。

use anyhow::{Context as _, anyhow};
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::net::UnixListener;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use transom_core::{AppEvent, is_ssh_url, paths};

/// 单条深链请求的字节上限：地址串远小于此，超限即视为异常输入截断丢弃，
/// 避免对端用长报文撑大读取侧内存。
const MAX_REQUEST_BYTES: u64 = 4096;

/// 等待一次请求写入的超时：本地对端进程假死时不能永久占住接收循环。
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// 首个实例的监听端点；套接字文件的清理由监听任务持有的守卫负责。
pub struct PrimaryInstance {
    listener: StdUnixListener,
    socket: SocketGuard,
}

/// 套接字文件守护：RAII 删除，监听任务结束时不留陈旧文件。
struct SocketGuard {
    path: PathBuf,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        // 删除失败不影响退出：残留文件会在下次启动时按"对端无响应即陈旧"清理。
        let _ = std::fs::remove_file(&self.path);
    }
}

/// 从命令行参数中挑出 `ssh://` 地址（跳过程序名，忽略启动器附带的其他参数）。
pub fn urls_from_args<I: IntoIterator<Item = String>>(args: I) -> Vec<String> {
    args.into_iter()
        .skip(1)
        .map(|arg| arg.trim().to_string())
        .filter(|arg| is_ssh_url(arg))
        .collect()
}

/// 决定本进程的角色：绑定套接字成功即成为主实例；失败说明已有实例在跑，
/// 把地址转交后返回 `None`，调用方应当直接退出。
///
/// 转发是同步阻塞写：此刻还没有 tokio 运行时，且写完即退出进程，不值得为
/// 一次性的启动动作引入异步。
pub fn acquire_primary(urls: &[String]) -> anyhow::Result<Option<PrimaryInstance>> {
    acquire_primary_at(&paths::instance_socket_path(), urls)
}

/// [`acquire_primary`] 的可注入路径版本。
///
/// 生产入口只有一条路径（`XDG_RUNTIME_DIR` 下的固定端点），但"绑定即锁 +
/// 陈旧清理 + 跨进程转发"三段逻辑必须在真实文件系统套接字上被验证，
/// 因此把路径作为参数暴露——与 `parse_ssh_config` / `parse_ssh_config_with_home`
/// 同一手法。
fn acquire_primary_at(
    socket_path: &Path,
    urls: &[String],
) -> anyhow::Result<Option<PrimaryInstance>> {
    match bind_listener(socket_path) {
        Ok(primary) => Ok(Some(primary)),
        Err(bind_error) if bind_error.kind() == io::ErrorKind::AddrInUse => {
            if probe_alive(socket_path) {
                forward(socket_path, urls).with_context(|| {
                    format!("向已运行实例转交深链地址失败，套接字: {socket_path:?}")
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
    ///
    /// 返回的 JoinHandle 由调用方持有（禁止脱缰任务）：任务 panic 时监督任务
    /// 记录日志，深链入口静默失效至少留下可检索的痕迹。
    pub fn serve(self, runtime: &Runtime, event_tx: flume::Sender<AppEvent>) -> JoinHandle<()> {
        let Self { listener, socket } = self;
        let path = socket.path.clone();
        // 调用点（main 的组装阶段）不在运行时上下文里，而 `from_std` 注册驱动
        // 事件源时要求当前 reactor 存在，因此先进入运行时上下文再转换。
        let _guard = runtime.enter();
        let join = match UnixListener::from_std(listener) {
            Ok(async_listener) => runtime.spawn(run_loop(async_listener, event_tx, socket)),
            Err(error) => {
                tracing::error!(
                    "实例套接字切换到异步监听失败，深链唤起不可用，路径: {path:?}，原因: {error}"
                );
                // 监听建不起来也要让守卫析构删除文件，并返回一个已完成句柄，
                // 保持调用方"持有一个可追踪任务"的契约不变。
                drop(socket);
                return runtime.spawn(async {});
            }
        };
        runtime.spawn(async move {
            match join.await {
                Ok(()) => tracing::info!("深链监听任务已退出"),
                Err(error) if error.is_panic() => {
                    tracing::error!("深链监听任务异常崩溃，外部唤起将不再响应: {error}")
                }
                Err(error) => tracing::warn!("深链监听任务被取消: {error}"),
            }
        })
    }
}

/// 绑定套接字并把权限收紧到仅属主可读写。
///
/// 目录 0700、套接字 0600：深链会让应用直接发起连接，必须排除同机其他用户
/// 伪造请求。XDG_RUNTIME_DIR 本身已是用户私有目录，这里再收紧一层，防的是
/// 环境变量缺失回落到配置目录时的共享权限。
fn bind_listener(socket_path: &Path) -> io::Result<PrimaryInstance> {
    let parent = socket_path.parent().unwrap_or(Path::new("."));
    if !parent.as_os_str().is_empty() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let listener = StdUnixListener::bind(socket_path)?;
    // 必须先切非阻塞再交给 tokio：`UnixListener::from_std` 检测到阻塞型 fd 会
    // 直接 panic（tokio 的硬约束），而这里绑定时机早于运行时。
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
///
/// 这次连接会让主实例的接收循环多走一轮，但对端连上就断开、没有写入任何字节，
/// `accept_one` 把空报文判为非请求，因此不会产生多余的唤起事件。
fn probe_alive(socket_path: &Path) -> bool {
    StdUnixStream::connect(socket_path).is_ok()
}

/// 把地址逐行写入已运行实例的套接字；没有地址时发一个空行，语义是"仅唤起"。
fn forward(socket_path: &Path, urls: &[String]) -> io::Result<()> {
    let stream = StdUnixStream::connect(socket_path)?;
    // 对端接收循环停滞时不无限等待：写超时后报错，由调用方决定后续处理。
    stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
    let mut writer = &stream;
    for url in urls {
        std::io::Write::write_all(&mut writer, url.as_bytes())?;
        std::io::Write::write_all(&mut writer, b"\n")?;
    }
    if urls.is_empty() {
        std::io::Write::write_all(&mut writer, b"\n")?;
    }
    std::io::Write::flush(&mut writer)
}

/// 接收循环：逐个处理连接，读到一行即投递事件。
///
/// 串行而非派生任务：深链由用户点击触发，频率极低，串行天然限制并发任务数；
/// 单次读取另有超时兜底，对端假死不会拖住整个循环。
async fn run_loop(listener: UnixListener, event_tx: flume::Sender<AppEvent>, socket: SocketGuard) {
    loop {
        match listener.accept().await {
            Ok((mut stream, _)) => {
                if let Err(error) = accept_one(&mut stream, &event_tx).await {
                    tracing::warn!("深链请求处理失败，原因: {error}");
                }
            }
            Err(error) => {
                // 守卫在此只用于报错时定位端点；析构发生在监听任务结束之时。
                tracing::error!(
                    "实例套接字接受连接失败，路径: {:?}，原因: {error}",
                    socket.path
                );
            }
        }
    }
}

/// 读取并投递单条请求；空报文（对端连上就断开）忽略，超限报文被截断后丢弃。
async fn accept_one<R: AsyncRead + Unpin>(
    stream: &mut R,
    event_tx: &flume::Sender<AppEvent>,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream.take(MAX_REQUEST_BYTES));
    let mut line = String::new();
    let read = tokio::time::timeout(REQUEST_TIMEOUT, reader.read_line(&mut line))
        .await
        .map_err(|_| anyhow!("等待深链请求超时，已丢弃"))?
        .context("读取深链请求失败")?;
    if read == 0 {
        // 对端没有写入任何内容就断开：不是请求，忽略。
        return Ok(());
    }
    let request = line.trim_end_matches(['\n', '\r']).trim();
    let url = if request.is_empty() {
        None
    } else if is_ssh_url(request) {
        Some(request.to_string())
    } else {
        // 只接受本协议的 scheme：套接字虽限定同用户，仍按不可信输入对待。
        return Err(anyhow!(
            "深链请求不是 ssh:// 地址，已拒绝，原始输入: {request}"
        ));
    };
    tracing::info!(
        url = url.as_deref().unwrap_or("<仅唤起>"),
        "收到外部深链唤起请求"
    );
    // try_send：事件通道满说明界面循环已积压，丢弃这次唤起并留痕，
    // 绝不在接收循环里阻塞（会拖垮后续请求）。
    if event_tx.try_send(AppEvent::DeepLink { url }).is_err() {
        tracing::warn!("深链唤起未能投递到界面事件通道（队列已满），本次请求已丢弃");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixStream;

    #[test]
    fn urls_from_args_keeps_only_ssh_scheme_arguments() {
        let args = vec![
            "/usr/bin/transom".to_string(),
            "  ssh://deploy@10.0.0.8 ".to_string(),
            "--flag".to_string(),
            "/home/user/some.file".to_string(),
            "SSH://host.example".to_string(),
        ];
        assert_eq!(
            urls_from_args(args),
            vec![
                "ssh://deploy@10.0.0.8".to_string(),
                "SSH://host.example".to_string()
            ]
        );
    }

    #[test]
    fn urls_from_args_skips_program_name() {
        assert!(urls_from_args(vec!["/usr/bin/transom".to_string()]).is_empty());
    }

    fn channel() -> (flume::Sender<AppEvent>, flume::Receiver<AppEvent>) {
        flume::bounded::<AppEvent>(4)
    }

    #[tokio::test]
    async fn accept_delivers_ssh_url() {
        use tokio::io::AsyncWriteExt;
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let (tx, rx) = channel();
        let write = async {
            client
                .write_all(b"ssh://root@host.example:2222\n")
                .await
                .unwrap();
        };
        let read = async { accept_one(&mut server, &tx).await };
        let ((), result) = tokio::join!(write, read);
        result.unwrap();
        match rx.recv().unwrap() {
            AppEvent::DeepLink { url } => {
                assert_eq!(url.as_deref(), Some("ssh://root@host.example:2222"));
            }
            _ => panic!("应投递深链事件"),
        }
    }

    /// 空行是"仅唤起"的合法语义：转发进程没有地址时也必须能叫醒界面。
    #[tokio::test]
    async fn accept_treats_blank_line_as_activate() {
        use tokio::io::AsyncWriteExt;
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let (tx, rx) = channel();
        let write = async {
            client.write_all(b"\n").await.unwrap();
        };
        let read = async { accept_one(&mut server, &tx).await };
        let ((), result) = tokio::join!(write, read);
        result.unwrap();
        assert!(matches!(
            rx.recv().unwrap(),
            AppEvent::DeepLink { url: None }
        ));
    }

    /// 深链请求是安全边界：非本 scheme 的文本必须被拒绝而不是当成地址。
    #[tokio::test]
    async fn accept_rejects_non_ssh_lines() {
        use tokio::io::AsyncWriteExt;
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let (tx, rx) = channel();
        let write = async {
            client.write_all(b"rm -rf /\n").await.unwrap();
        };
        let read = async { accept_one(&mut server, &tx).await };
        let ((), result) = tokio::join!(write, read);
        assert!(result.is_err());
        assert!(rx.is_empty());
    }

    /// 对端连上就断开不是请求：不应产生任何事件。
    #[tokio::test]
    async fn accept_ignores_immediate_eof() {
        let (_client, mut server) = UnixStream::pair().unwrap();
        drop(_client);
        let (tx, rx) = channel();
        accept_one(&mut server, &tx).await.unwrap();
        assert!(rx.is_empty());
    }

    /// 超长报文被截断：读取上限必须生效，否则对端可以随意放大本进程内存。
    #[tokio::test]
    async fn accept_caps_oversized_request() {
        use tokio::io::AsyncWriteExt;
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let (tx, rx) = channel();
        let payload = format!(
            "ssh://{}@host.example\n",
            "a".repeat(MAX_REQUEST_BYTES as usize)
        );
        let write = async {
            // 对端只读前 4096 字节后就不再消费，写入需要容忍管道半关闭。
            let _ = client.write_all(payload.as_bytes()).await;
        };
        let read = async { accept_one(&mut server, &tx).await };
        let ((), result) = tokio::join!(write, read);
        result.unwrap();
        match rx.recv().unwrap() {
            AppEvent::DeepLink { url } => {
                let url = url.expect("截断后的前缀仍是合法 scheme 开头");
                assert!(url.len() <= MAX_REQUEST_BYTES as usize);
            }
            _ => panic!("应投递深链事件"),
        }
    }

    /// 唯一临时套接字路径：并行跑测试时用进程号与递增序号隔离，收尾由调用方删除。
    fn temp_socket_path(tag: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "transom-deeplink-{}-{tag}-{seq}.sock",
            std::process::id()
        ));
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        path
    }

    /// 端到端闭环：绑定即锁、二次启动转发、主实例收到事件。
    ///
    /// 走真实文件系统套接字而不是内存套接字对，为的是覆盖 bind 失败分支、
    /// `probe_alive` 判定与阻塞写转发——这三段只在真实路径上才有意义。
    #[test]
    fn second_instance_forwards_url_to_running_primary() {
        let path = temp_socket_path("forward");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (tx, rx) = channel();

        let primary = acquire_primary_at(&path, &[])
            .unwrap()
            .expect("首个实例应绑定成功");
        let _watch = primary.serve(&runtime, tx);

        // 模拟第二个进程：带地址启动，应当绑定失败并把地址转交后退出。
        let urls = vec!["ssh://root@host.example:2222".to_string()];
        let second = acquire_primary_at(&path, &urls).unwrap();
        assert!(second.is_none(), "已有实例在跑时不应再成为主实例");

        let event = runtime.block_on(rx.recv_async()).unwrap();
        match event {
            AppEvent::DeepLink { url } => {
                assert_eq!(url.as_deref(), Some("ssh://root@host.example:2222"));
            }
            _ => panic!("应投递深链事件"),
        }

        drop(runtime);
        let _ = std::fs::remove_file(&path);
    }

    /// 上次崩溃留下的陈旧套接字必须被识别并清理，否则应用永远启动不了。
    #[test]
    fn stale_socket_is_reclaimed_on_bind_conflict() {
        let path = temp_socket_path("stale");
        // 占位文件：不是套接字，对端必然连不上，等价于进程已死后的残留端点。
        std::fs::write(&path, b"stale").unwrap();

        let primary = acquire_primary_at(&path, &[])
            .unwrap()
            .expect("陈旧端点应被清理并重新绑定");
        assert!(probe_alive(&path), "重新绑定后本进程应持有可连接的套接字");

        drop(primary);
        let _ = std::fs::remove_file(&path);
    }
}
