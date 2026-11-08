//! Windows 单实例传输：当前用户私有命名管道。
//!
//! 设计原理（Why）：`FILE_FLAG_FIRST_PIPE_INSTANCE` 让首个实例的创建成为
//! 原子判定，后续进程通过同一管道转发请求；管道由 transom-platform 使用
//! 私有 DACL 创建，避免其它本地用户注入深链。
//!
//! 代价与局限：Windows 没有 Unix socket 文件式陈旧端点，进程退出时内核即
//! 回收管道，因此不需要额外清理文件。

use super::protocol::{accept_stream, write_request};
use super::types::LaunchRequest;
use anyhow::{Context as _, anyhow};
use std::fs::File;
use std::os::windows::io::IntoRawHandle;
use std::time::Duration;
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use transom_core::AppEvent;
use transom_platform::named_pipe::{connect_private_client, create_private_server};
use transom_platform::paths;
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::GetCurrentProcessId;

/// 首个实例持有的初始管道句柄。
pub struct PrimaryInstance {
    first_pipe: File,
    pipe_name: String,
}

/// 决定本进程角色：创建首个 pipe instance 成功即为主实例；否则把请求转发
/// 给已有实例并返回 `None`。
pub fn acquire_primary(request: &LaunchRequest) -> anyhow::Result<Option<PrimaryInstance>> {
    let pipe_name = pipe_name()?;
    acquire_primary_at(&pipe_name, request)
}

fn acquire_primary_at(
    pipe_name: &str,
    request: &LaunchRequest,
) -> anyhow::Result<Option<PrimaryInstance>> {
    match create_private_server(pipe_name, true) {
        Ok(first_pipe) => Ok(Some(PrimaryInstance {
            first_pipe,
            pipe_name: pipe_name.to_string(),
        })),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied
                    | std::io::ErrorKind::AddrInUse
                    | std::io::ErrorKind::AlreadyExists
            ) =>
        {
            match forward_to_primary(pipe_name, request) {
                Ok(()) => Ok(None),
                Err(forward_error) => {
                    // 首实例在判定与连接之间退出时，本进程可以接管；只有重新
                    // 创建首个实例仍失败，才把原始转交错误返回给用户。
                    match create_private_server(pipe_name, true) {
                        Ok(first_pipe) => Ok(Some(PrimaryInstance {
                            first_pipe,
                            pipe_name: pipe_name.to_string(),
                        })),
                        Err(_) => Err(forward_error),
                    }
                }
            }
        }
        Err(error) => Err(anyhow!(
            "创建实例命名管道失败，管道: {pipe_name}，原因: {error}"
        )),
    }
}

fn forward_to_primary(pipe_name: &str, request: &LaunchRequest) -> anyhow::Result<()> {
    let mut client = connect_private_client(pipe_name)
        .with_context(|| format!("已有实例管道不可连接，管道: {pipe_name}"))?;
    write_request(&mut client, request)
        .with_context(|| format!("向已运行实例转交唤起请求失败，管道: {pipe_name}"))
}

impl PrimaryInstance {
    /// 把初始管道接入控制平面运行时并持续接受请求。
    pub fn serve(self, runtime: &Runtime, event_tx: flume::Sender<AppEvent>) -> JoinHandle<()> {
        let Self {
            first_pipe,
            pipe_name,
        } = self;
        let _guard = runtime.enter();
        let first_server = match async_server(first_pipe) {
            Ok(server) => server,
            Err(error) => {
                log::error!("实例命名管道切换到异步监听失败，管道: {pipe_name}，原因: {error}");
                return runtime.spawn(async {});
            }
        };
        let join = runtime.spawn(run_loop(first_server, pipe_name, event_tx));
        runtime.spawn(async move {
            match join.await {
                Ok(()) => log::info!("Windows 深链监听任务已退出"),
                Err(error) if error.is_panic() => {
                    log::error!("Windows 深链监听任务异常崩溃，外部唤起将不再响应: {error}")
                }
                Err(error) => log::warn!("Windows 深链监听任务被取消: {error}"),
            }
        })
    }
}

async fn run_loop(
    mut server: NamedPipeServer,
    pipe_name: String,
    event_tx: flume::Sender<AppEvent>,
) {
    loop {
        if let Err(error) = server.connect().await {
            log::error!("命名管道等待客户端连接失败，管道: {pipe_name}，原因: {error}");
            return;
        }
        // 先创建后继监听实例再处理当前连接：客户端连续唤起时不会撞上
        // “连接刚关闭、下一实例尚未创建”的短暂 NotFound 窗口。
        let next_server = match create_private_server(&pipe_name, false).and_then(async_server) {
            Ok(server) => Some(server),
            Err(error) => {
                log::error!("创建后继命名管道实例失败，管道: {pipe_name}，原因: {error}");
                None
            }
        };
        if let Err(error) = accept_stream(&mut server, &event_tx).await {
            log::warn!("Windows 深链请求处理失败，管道: {pipe_name}，原因: {error}");
        }
        drop(server);
        server = match next_server {
            Some(server) => server,
            None => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                match create_private_server(&pipe_name, false).and_then(async_server) {
                    Ok(server) => server,
                    Err(error) => {
                        log::error!("恢复命名管道监听失败，管道: {pipe_name}，原因: {error}");
                        return;
                    }
                }
            }
        };
    }
}

fn async_server(file: File) -> std::io::Result<NamedPipeServer> {
    let raw = file.into_raw_handle();
    unsafe { NamedPipeServer::from_raw_handle(raw) }
}

fn pipe_name() -> anyhow::Result<String> {
    let mut session_id = 0u32;
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session_id) }
        .map_err(|error| anyhow!("读取当前 Windows 会话 ID 失败: {error}"))?;
    let channel = if paths::is_current_exe_dev() {
        "dev"
    } else {
        "stable"
    };
    Ok(format!(
        r"\\.\pipe\{}-{channel}-{session_id}",
        paths::APP_ID
    ))
}

#[cfg(test)]
mod tests {
    use super::{acquire_primary_at, pipe_name};
    use crate::deep_link::{LaunchRequest, request_from_args};
    use transom_core::{AppEvent, DeepLinkRequest};

    #[test]
    fn pipe_name_is_user_session_scoped() {
        let name = pipe_name().unwrap();
        assert!(name.starts_with(r"\\.\pipe\com.github.mangerle.transom-"));
    }

    #[test]
    fn second_instance_forwards_to_primary_named_pipe() {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = format!(
            r"\\.\pipe\com.github.mangerle.transom-test-{}-{seq}",
            std::process::id()
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (tx, rx) = flume::bounded::<AppEvent>(4);
        let primary = acquire_primary_at(&name, &LaunchRequest::default())
            .unwrap()
            .expect("首个测试实例应创建命名管道");
        let _watch = primary.serve(&runtime, tx);

        let request = request_from_args([
            "transom".to_string(),
            "ssh://root@host.example:2222".to_string(),
        ]);
        assert!(
            acquire_primary_at(&name, &request).unwrap().is_none(),
            "第二个实例必须转交请求"
        );
        match runtime.block_on(rx.recv_async()).unwrap() {
            AppEvent::DeepLink {
                request: DeepLinkRequest::Ssh { url },
            } => assert_eq!(url, "ssh://root@host.example:2222"),
            _ => panic!("应收到 SSH 深链请求"),
        }
    }
}
