//! 模拟 Docker 引擎的集成测试：用 Unix socket 验证探测、版本协商与列表解析。
//!
//! 设计原理（Why）：本机 CI 不一定安装 Docker/Podman，但 HTTP 握手、路径
//! 版本前缀与 JSON 收敛必须被真实覆盖；一个最小 HTTP 响应端就能验证整条链路，
//! 又不引入外部服务依赖。

#![cfg(unix)]

use std::io::{Read, Write};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use transom_container::{Engine, open_exec, probe_explicit};
use transom_core::{
    ContainerConnection, ContainerEndpoint, ContainerEngineKind, TerminalIo, TerminalSize,
};

const VERSION_JSON: &str =
    r#"{"Version":"27.0.0","ApiVersion":"1.41","Platform":{"Name":"Docker Engine"}}"#;

const CONTAINERS_JSON: &str = r#"[
  {
    "Id": "sha256:aaaaaaaaaaaaaaaa",
    "Names": ["/web-1"],
    "Image": "nginx:latest",
    "Created": 1700000000,
    "Ports": [{"IP":"0.0.0.0","PrivatePort":80,"PublicPort":8080,"Type":"tcp"}],
    "Labels": {
      "com.docker.compose.project": "shop",
      "com.docker.compose.project.working_dir": "/srv/shop",
      "com.docker.compose.project.config_files": "/srv/shop/compose.yml"
    },
    "State": "running",
    "Status": "Up 1 hour",
    "Mounts": [{"Type":"volume","Name":"shop-data","Source":"/var/lib/docker/volumes/shop-data","Destination":"/data","RW":true}]
  },
  {
    "Id": "sha256:bbbbbbbbbbbbbbbb",
    "Names": ["/web-2"],
    "Image": "nginx:latest",
    "Created": 1700000000,
    "Ports": [],
    "Labels": {"com.docker.compose.project": "shop"},
    "State": "exited",
    "Status": "Exited (0) 5 minutes ago",
    "Mounts": []
  }
]"#;

/// 启动一个最小 HTTP/1.1 服务端；每个请求新建连接并由服务端关闭。
fn spawn_mock_engine(path: PathBuf) -> tokio::task::JoinHandle<()> {
    // 先同步绑定再派生接收循环：调用方紧接着就会连接，不能让 bind 与 connect
    // 在同一个运行时里赛跑。
    let listener = UnixListener::bind(&path).expect("绑定测试 socket");
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 16 * 1024];
                let Ok(count) = stream.read(&mut buffer).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buffer[..count]);
                let target = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("");
                let (body, extra_header) = if target.contains("/_ping") {
                    ("OK".to_string(), "API-Version: 1.41\r\n")
                } else if target.contains("/version") {
                    (VERSION_JSON.to_string(), "")
                } else if target.contains("/containers/json") {
                    (CONTAINERS_JSON.to_string(), "")
                } else {
                    (r#"{"message":"unexpected path"}"#.to_string(), "")
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{extra_header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probes_versioned_api_and_parses_containers() {
    let directory = tempfile::tempdir().expect("临时目录");
    let socket = directory.path().join("docker.sock");
    let server = spawn_mock_engine(socket.clone());

    let engine: Engine = probe_explicit(
        ContainerEngineKind::Docker,
        &ContainerEndpoint::Unix { path: socket },
    )
    .await
    .expect("探测模拟引擎");
    assert_eq!(engine.info.version, "27.0.0");
    assert_eq!(engine.info.api_version, "1.41");
    assert!(!engine.cli_backend());

    let containers = engine.list_containers().await.expect("列出容器");
    assert_eq!(containers.len(), 2);
    assert_eq!(containers[0].name, "web-1");
    assert_eq!(containers[0].ports_label(), "8080->80/tcp");
    assert_eq!(containers[0].compose_project.as_deref(), Some("shop"));

    let projects = engine.compose_projects().await.expect("聚合 Compose 项目");
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "shop");
    assert_eq!(projects[0].running, 1);
    assert_eq!(projects[0].total, 2);

    server.abort();
}

/// 模拟 exec：创建 exec → 升级连接 → 原样回显 → 退出码查询。
fn spawn_mock_exec_engine(path: PathBuf) -> tokio::task::JoinHandle<()> {
    let listener = UnixListener::bind(&path).expect("绑定 exec 测试 socket");
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let (head, _body) = read_request(&mut stream).await;
                let target = head
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("");
                if target.contains("/_ping") {
                    let response = "HTTP/1.1 200 OK\r\nAPI-Version: 1.41\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
                    let _ = stream.write_all(response.as_bytes()).await;
                } else if target.contains("/version") {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{VERSION_JSON}",
                        VERSION_JSON.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                } else if target.contains("/exec/exec1/start") {
                    let response = "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tcp\r\nContent-Type: application/vnd.docker.raw-stream\r\n\r\n";
                    if stream.write_all(response.as_bytes()).await.is_err() {
                        return;
                    }
                    let mut buffer = [0u8; 32];
                    if let Ok(count) = stream.read(&mut buffer).await {
                        let _ = stream.write_all(&buffer[..count]).await;
                    }
                } else if target.contains("/exec/exec1/json") {
                    let body = r#"{"ExitCode":0,"Running":false}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                } else if target.contains("/containers/") && target.contains("/exec") {
                    let body = r#"{"Id":"exec1"}"#;
                    let response = format!(
                        "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                } else {
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                }
                let _ = stream.shutdown().await;
            });
        }
    })
}

/// 读取完整请求头与按 Content-Length 指定的请求体。
async fn read_request(stream: &mut tokio::net::UnixStream) -> (String, Vec<u8>) {
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    while let Ok(count) = stream.read(&mut chunk).await {
        if count == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..count]);
        let Some(header_end) = find_bytes(&buffer, b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&buffer[..header_end + 4]).to_string();
        let content_length = head
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        let body_start = header_end + 4;
        while buffer.len() < body_start + content_length {
            let Ok(count) = stream.read(&mut chunk).await else {
                break;
            };
            if count == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..count]);
        }
        let body_end = (body_start + content_length).min(buffer.len());
        return (head, buffer[body_start..body_end].to_vec());
    }
    (String::new(), Vec::new())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_upgrade_bridges_blocking_read_and_write() {
    let directory = tempfile::tempdir().expect("临时目录");
    let socket = directory.path().join("exec.sock");
    let server = spawn_mock_exec_engine(socket.clone());

    let engine: Engine = probe_explicit(
        ContainerEngineKind::Docker,
        &ContainerEndpoint::Unix { path: socket },
    )
    .await
    .expect("探测模拟引擎");
    let (io, _exit) = open_exec(
        &engine,
        &ContainerConnection::new("web-1"),
        TerminalSize::new(80, 24),
    )
    .await
    .expect("打开模拟 exec");

    let TerminalIo {
        mut reader,
        mut writer,
        cleanup,
        ..
    } = io;
    let read_task = std::thread::spawn(move || {
        let mut buffer = [0u8; 5];
        reader.read_exact(&mut buffer).expect("读取回显");
        buffer
    });
    writer.write_all(b"hello").expect("写入终端");
    writer.flush().expect("刷新终端");
    assert_eq!(read_task.join().expect("读取线程"), *b"hello");
    drop(cleanup);
    server.abort();
}
