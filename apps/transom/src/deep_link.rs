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
use transom_core::{AppEvent, DeepLinkRequest, is_ssh_url, paths};

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

/// 转发协议里目录请求的行前缀：路径可以合法地以 `/` 开头，必须先于
/// `ssh://` 判定剥离，避免把本地路径误当协议地址。
const DIR_LINE_PREFIX: &str = "dir:";

/// 一次启动参数解析出的唤起意图。
///
/// 设计原理（Why）：`ssh://` 地址与目录参数互不冲突（`transom ssh://a ssh://b`
/// 合法，开多个标签），但目录只取第一个——第二个来源不明的目录该忽略而不是
/// 静默换靶。非法状态用枚举表达见 `DeepLinkRequest`，这里只是解析中间态。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LaunchRequest {
    /// 启动参数中出现的 `ssh://` 地址，按出现顺序。
    pub urls: Vec<String>,
    /// `--dir=` / `--dir X` / 裸绝对路径参数给出的工作目录（取第一个）。
    pub dir: Option<PathBuf>,
}

/// 从命令行参数解析唤起意图（跳过程序名）。
///
/// 目录参数接受三种形态：`--dir=/path`（桌面条目 `X-TerminalArgDir` 让
/// launcher 采用的形式）、`--dir /path` 与裸位置参数（第三方工具直接传目录）；
/// 位置参数只在"绝对路径且当前存在为目录"时采信——协议上除
/// `ssh://` 外的参数本就忽略，这里再加存在性校验，杜绝把陈旧路径带进转发。
/// launcher 会先剥掉条目 Exec 里的 `%u` 再拼参数，因此目录参数不会与地址
/// 形态混淆。其余未知参数继续忽略（启动器附带杂参是常态）。
///
/// 已知缺口：`xdg-terminal-exec -- <命令>` 对未声明 `X-TerminalArgExec` 的
/// 条目会注入兜底的 `-e <命令>`，而"在终端里执行这条命令"的语义 transom
/// 尚未实现，这类参数落到忽略分支——终端会开、命令不跑。已在两份 README 的
/// 缺口清单中明示，不做静默支持也不假装成功。
pub fn request_from_args<I: IntoIterator<Item = String>>(args: I) -> LaunchRequest {
    let mut request = LaunchRequest::default();
    let mut argv = args.into_iter().skip(1).peekable();
    while let Some(arg) = argv.next() {
        let arg = arg.trim().to_string();
        if let Some(rest) = arg.strip_prefix("--dir=") {
            request.set_dir(rest);
            continue;
        }
        if arg == "--dir" {
            if let Some(value) = argv.next() {
                request.set_dir(value.trim());
            } else {
                log::warn!("启动参数 --dir 后面缺少目录值，已忽略");
            }
            continue;
        }
        if is_ssh_url(&arg) {
            request.urls.push(arg);
            continue;
        }
        // 裸位置参数只在形如绝对路径时才可能是目录请求。
        if arg.starts_with('/') {
            request.set_dir(&arg);
        }
    }
    request
}

impl LaunchRequest {
    /// 记录第一个可信目录；后续目录参数忽略并留痕（歧义不猜）。
    fn set_dir(&mut self, raw: &str) {
        if self.dir.is_some() {
            log::warn!("启动参数中出现第二个目录请求，已忽略: {raw}");
            return;
        }
        let candidate = Path::new(raw);
        if candidate.is_absolute() && candidate.is_dir() {
            self.dir = Some(candidate.to_path_buf());
        } else {
            // 只采信现存绝对路径：相对形态既无从判定基准目录，也不符合
            // launcher 的传参约定，按忽略处理。
            log::debug!("目录参数不是现存绝对目录，已忽略: {raw}");
        }
    }
}

/// 单条转发请求的文本行：ssh 地址原样，目录加 `dir:` 前缀。
fn request_lines(request: &LaunchRequest) -> Vec<String> {
    let mut lines = request.urls.clone();
    if let Some(dir) = &request.dir {
        lines.push(format!("{DIR_LINE_PREFIX}{}", dir.display()));
    }
    lines
}

/// 决定本进程的角色：绑定套接字成功即成为主实例；失败说明已有实例在跑，
/// 把请求转交后返回 `None`，调用方应当直接退出。
///
/// 转发是同步阻塞写：此刻还没有 tokio 运行时，且写完即退出进程，不值得为
/// 一次性的启动动作引入异步。
pub fn acquire_primary(request: &LaunchRequest) -> anyhow::Result<Option<PrimaryInstance>> {
    acquire_primary_at(&paths::instance_socket_path(), request)
}

/// [`acquire_primary`] 的可注入路径版本。
///
/// 生产入口只有一条路径（`XDG_RUNTIME_DIR` 下的固定端点），但"绑定即锁 +
/// 陈旧清理 + 跨进程转发"三段逻辑必须在真实文件系统套接字上被验证，
/// 因此把路径作为参数暴露——与 `parse_ssh_config` / `parse_ssh_config_with_home`
/// 同一手法。
fn acquire_primary_at(
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
                log::error!(
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
/// `accept_stream` 把空报文判为非请求，因此不会产生多余的唤起事件。
fn probe_alive(socket_path: &Path) -> bool {
    StdUnixStream::connect(socket_path).is_ok()
}

/// 把请求逐行写入已运行实例的套接字；没有任何请求时发一个空行，语义是"仅唤起"。
fn forward(socket_path: &Path, request: &LaunchRequest) -> io::Result<()> {
    let stream = StdUnixStream::connect(socket_path)?;
    // 对端接收循环停滞时不无限等待：写超时后报错，由调用方决定后续处理。
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
///
/// 串行而非派生任务：深链由用户点击触发，频率极低，串行天然限制并发任务数；
/// 单次读取另有超时兜底，对端假死不会拖住整个循环。
async fn run_loop(listener: UnixListener, event_tx: flume::Sender<AppEvent>, socket: SocketGuard) {
    loop {
        match listener.accept().await {
            Ok((mut stream, _)) => {
                if let Err(error) = accept_stream(&mut stream, &event_tx).await {
                    log::warn!("深链请求处理失败，原因: {error}");
                }
            }
            Err(error) => {
                // 守卫在此只用于报错时定位端点；析构发生在监听任务结束之时。
                log::error!(
                    "实例套接字接受连接失败，路径: {:?}，原因: {error}",
                    socket.path
                );
            }
        }
    }
}

/// 读完一条连接上的所有请求并逐条投递。
///
/// 设计原理（Why）：转发方把地址与目录等多条请求写在同一次连接里（见
/// `forward`），早期实现只读首行就丢弃连接，导致"带目录的第二条请求永远
/// 收不到"——多个 `ssh://` 地址同样只会生效第一个。这里按行读到 EOF，
/// 空报文（连上就断开）视为无请求。整条连接的字节上限仍由
/// `MAX_REQUEST_BYTES` 兜底，超限部分被截断。
async fn accept_stream<R: AsyncRead + Unpin>(
    stream: &mut R,
    event_tx: &flume::Sender<AppEvent>,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream.take(MAX_REQUEST_BYTES));
    loop {
        // 每行独立计时：对端写完一半挂起时不能永久占住接收循环。
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
    // try_send：事件通道满说明界面循环已积压，丢弃这次唤起并留痕，
    // 绝不在接收循环里阻塞（会拖垮后续请求）。
    if event_tx.try_send(AppEvent::DeepLink { request }).is_err() {
        log::warn!("深链唤起未能投递到界面事件通道（队列已满），本次请求已丢弃");
    }
    Ok(())
}

/// 把一行请求文本洗成 [`DeepLinkRequest`]：协议只认三种形态，其余一律拒绝。
///
/// 套接字虽限定同用户（0600 + 0700 目录），仍按不可信输入处理：目录必须是
/// 现存绝对路径，杜绝用伪造相对路径把终端开进意料之外的位置。
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixStream;

    /// 测试专用的真实临时目录：并行隔离用 pid + 递增序号，收尾删除。
    fn temp_dir(tag: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "transom-deeplink-{tag}-{}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("测试临时目录应可创建");
        path
    }

    fn channel() -> (flume::Sender<AppEvent>, flume::Receiver<AppEvent>) {
        flume::bounded::<AppEvent>(4)
    }

    #[test]
    fn request_from_args_keeps_only_protocol_arguments() {
        let dir = temp_dir("args");
        let args = vec![
            "/usr/bin/transom".to_string(),
            "  ssh://deploy@10.0.0.8 ".to_string(),
            "--flag".to_string(),
            dir.display().to_string(),
            "SSH://host.example".to_string(),
        ];
        let request = request_from_args(args);
        assert_eq!(
            request.urls,
            vec![
                "ssh://deploy@10.0.0.8".to_string(),
                "SSH://host.example".to_string()
            ]
        );
        assert_eq!(request.dir.as_deref(), Some(dir.as_path()));
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn request_from_args_skips_program_name() {
        assert_eq!(
            request_from_args(vec!["/usr/bin/transom".to_string()]),
            LaunchRequest::default()
        );
    }

    #[test]
    fn request_from_args_accepts_dir_option_forms() {
        let dir = temp_dir("opt");
        for form in [
            vec!["prog".to_string(), format!("--dir={}", dir.display())],
            vec![
                "prog".to_string(),
                "--dir".to_string(),
                dir.display().to_string(),
            ],
        ] {
            let request = request_from_args(form);
            assert_eq!(
                request.dir.as_deref(),
                Some(dir.as_path()),
                "--dir 两种形态都要认"
            );
        }
        let _ = std::fs::remove_dir(&dir);
    }

    /// 目录参数必须"绝对 + 现存"：陈旧或相对路径按忽略处理（存在性校验前置）。
    #[test]
    fn request_from_args_rejects_stale_or_relative_dirs() {
        let request = request_from_args(vec![
            "prog".to_string(),
            "--dir=/definitely/not/here-42".to_string(),
        ]);
        assert_eq!(request.dir, None);
        let request =
            request_from_args(vec!["prog".to_string(), "--dir=relative/path".to_string()]);
        assert_eq!(request.dir, None);
    }

    #[test]
    fn request_from_args_takes_first_dir_only() {
        let first = temp_dir("first");
        let second = temp_dir("second");
        let request = request_from_args(vec![
            "prog".to_string(),
            first.display().to_string(),
            second.display().to_string(),
        ]);
        assert_eq!(request.dir.as_deref(), Some(first.as_path()));
        let _ = std::fs::remove_dir(&first);
        let _ = std::fs::remove_dir(&second);
    }

    #[test]
    fn request_from_args_consumes_value_after_dir_flag() {
        // `--dir ssh://x`：ssh 地址被当作目录值吃掉，不应再出现在地址表里；
        // 目录值无效（不是现存目录）时两者皆空——歧义参数的行为要钉死。
        let request = request_from_args(vec![
            "prog".to_string(),
            "--dir".to_string(),
            "ssh://host.example".to_string(),
        ]);
        assert!(request.urls.is_empty());
        assert_eq!(request.dir, None);
    }

    #[tokio::test]
    async fn accept_delivers_ssh_url() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let (tx, rx) = channel();
        let write = async {
            client
                .write_all(b"ssh://root@host.example:2222\n")
                .await
                .unwrap();
            // 读侧按连接读到 EOF，写完必须半关闭否则接收循环等不到结束。
            client.shutdown().await.unwrap();
        };
        let read = async { accept_stream(&mut server, &tx).await };
        let ((), result) = tokio::join!(write, read);
        result.unwrap();
        match rx.recv().unwrap() {
            AppEvent::DeepLink {
                request: DeepLinkRequest::Ssh { url },
            } => {
                assert_eq!(url, "ssh://root@host.example:2222");
            }
            _ => panic!("应投递 ssh 直连请求"),
        }
    }

    /// 空行是"仅唤起"的合法语义：转发进程没有请求时也必须能叫醒界面。
    #[tokio::test]
    async fn accept_treats_blank_line_as_activate() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let (tx, rx) = channel();
        let write = async {
            client.write_all(b"\n").await.unwrap();
            client.shutdown().await.unwrap();
        };
        let read = async { accept_stream(&mut server, &tx).await };
        let ((), result) = tokio::join!(write, read);
        result.unwrap();
        assert!(matches!(
            rx.recv().unwrap(),
            AppEvent::DeepLink {
                request: DeepLinkRequest::Activate
            }
        ));
    }

    /// `dir:` 前缀行解析为目录请求，路径原样进入事件。
    #[tokio::test]
    async fn accept_delivers_dir_request() {
        let dir = temp_dir("accept");
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let (tx, rx) = channel();
        let payload = format!("dir:{}\n", dir.display());
        let write = async {
            client.write_all(payload.as_bytes()).await.unwrap();
            client.shutdown().await.unwrap();
        };
        let read = async { accept_stream(&mut server, &tx).await };
        let ((), result) = tokio::join!(write, read);
        result.unwrap();
        match rx.recv().unwrap() {
            AppEvent::DeepLink {
                request: DeepLinkRequest::Dir { path },
            } => assert_eq!(path, dir),
            _ => panic!("应投递目录请求"),
        }
        let _ = std::fs::remove_dir(&dir);
    }

    /// 回归：同一连接上的多条请求必须全部生效（早期实现只读首行就丢连接，
    /// 带目录的第二条请求永远收不到，多个 ssh 地址也只生效第一个）。
    #[tokio::test]
    async fn accept_drives_all_requests_on_one_connection() {
        let dir = temp_dir("multi");
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let (tx, rx) = channel();
        let payload = format!("ssh://a@host.example\ndir:{}\n", dir.display());
        let write = async {
            client.write_all(payload.as_bytes()).await.unwrap();
            client.shutdown().await.unwrap();
        };
        let read = async { accept_stream(&mut server, &tx).await };
        let ((), result) = tokio::join!(write, read);
        result.unwrap();
        match rx.recv().unwrap() {
            AppEvent::DeepLink {
                request: DeepLinkRequest::Ssh { url },
            } => assert_eq!(url, "ssh://a@host.example"),
            _ => panic!("首条应为 ssh 请求"),
        }
        match rx.recv().unwrap() {
            AppEvent::DeepLink {
                request: DeepLinkRequest::Dir { path },
            } => assert_eq!(path, dir),
            _ => panic!("第二条应为目录请求"),
        }
        let _ = std::fs::remove_dir(&dir);
    }

    /// 深链请求是安全边界：非本协议的文本与非现存目录都必须被拒绝。
    #[tokio::test]
    async fn accept_rejects_foreign_and_stale_lines() {
        let cases: Vec<String> = vec![
            "rm -rf /\n".to_string(),
            "dir:/definitely/not/here-42\n".to_string(),
            "dir:relative/path\n".to_string(),
        ];
        for payload in cases {
            let (mut client, mut server) = UnixStream::pair().unwrap();
            let (tx, rx) = channel();
            let write = async {
                client.write_all(payload.as_bytes()).await.unwrap();
            };
            let read = async { accept_stream(&mut server, &tx).await };
            let ((), result) = tokio::join!(write, read);
            assert!(result.is_err(), "非法请求必须报错: {payload:?}");
            assert!(rx.is_empty(), "非法请求不得产生事件");
        }
    }

    /// 对端连上就断开不是请求：不应产生任何事件。
    #[tokio::test]
    async fn accept_ignores_immediate_eof() {
        let (_client, mut server) = UnixStream::pair().unwrap();
        drop(_client);
        let (tx, rx) = channel();
        accept_stream(&mut server, &tx).await.unwrap();
        assert!(rx.is_empty());
    }

    /// 超长报文被截断：读取上限必须生效，否则对端可以随意放大本进程内存。
    #[tokio::test]
    async fn accept_caps_oversized_request() {
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
        let read = async { accept_stream(&mut server, &tx).await };
        let ((), result) = tokio::join!(write, read);
        result.unwrap();
        match rx.recv().unwrap() {
            AppEvent::DeepLink {
                request: DeepLinkRequest::Ssh { url },
            } => {
                assert!(url.len() <= MAX_REQUEST_BYTES as usize);
            }
            _ => panic!("应投递 ssh 直连请求"),
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

    /// 端到端闭环：绑定即锁、二次启动转发（含目录）、主实例收到两类事件。
    ///
    /// 走真实文件系统套接字而不是内存套接字对，为的是覆盖 bind 失败分支、
    /// `probe_alive` 判定与阻塞写转发——这三段只在真实路径上才有意义。
    #[test]
    fn second_instance_forwards_requests_to_running_primary() {
        let path = temp_socket_path("forward");
        let dir = temp_dir("forward");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (tx, rx) = channel();

        let primary = acquire_primary_at(&path, &LaunchRequest::default())
            .unwrap()
            .expect("首个实例应绑定成功");
        let _watch = primary.serve(&runtime, tx);

        // 模拟第二个进程：带地址与目录启动，应当绑定失败并把两行请求转交后退出。
        let request = LaunchRequest {
            urls: vec!["ssh://root@host.example:2222".to_string()],
            dir: Some(dir.clone()),
        };
        let second = acquire_primary_at(&path, &request).unwrap();
        assert!(second.is_none(), "已有实例在跑时不应再成为主实例");

        let first_event = runtime.block_on(rx.recv_async()).unwrap();
        match first_event {
            AppEvent::DeepLink {
                request: DeepLinkRequest::Ssh { url },
            } => assert_eq!(url, "ssh://root@host.example:2222"),
            _ => panic!("应先投递 ssh 直连请求"),
        }
        let second_event = runtime.block_on(rx.recv_async()).unwrap();
        match second_event {
            AppEvent::DeepLink {
                request: DeepLinkRequest::Dir { path: got },
            } => assert_eq!(got, dir),
            _ => panic!("应投递目录请求"),
        }

        drop(runtime);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    /// 上次崩溃留下的陈旧套接字必须被识别并清理，否则应用永远启动不了。
    #[test]
    fn stale_socket_is_reclaimed_on_bind_conflict() {
        let path = temp_socket_path("stale");
        // 占位文件：不是套接字，对端必然连不上，等价于进程已死后的残留端点。
        std::fs::write(&path, b"stale").unwrap();

        let primary = acquire_primary_at(&path, &LaunchRequest::default())
            .unwrap()
            .expect("陈旧端点应被清理并重新绑定");
        assert!(probe_alive(&path), "重新绑定后本进程应持有可连接的套接字");

        drop(primary);
        let _ = std::fs::remove_file(&path);
    }
}
