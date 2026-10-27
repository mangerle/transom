//! 深链与单实例测试用例。

use super::service::{MAX_REQUEST_BYTES, accept_stream, acquire_primary_at, probe_alive};
use super::types::{LaunchRequest, request_from_args};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use transom_core::{AppEvent, DeepLinkRequest};

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

#[test]
fn request_from_args_rejects_stale_or_relative_dirs() {
    let request = request_from_args(vec![
        "prog".to_string(),
        "--dir=/definitely/not/here-42".to_string(),
    ]);
    assert_eq!(request.dir, None);
    let request = request_from_args(vec!["prog".to_string(), "--dir=relative/path".to_string()]);
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

#[tokio::test]
async fn accept_ignores_immediate_eof() {
    let (_client, mut server) = UnixStream::pair().unwrap();
    drop(_client);
    let (tx, rx) = channel();
    accept_stream(&mut server, &tx).await.unwrap();
    assert!(rx.is_empty());
}

#[tokio::test]
async fn accept_caps_oversized_request() {
    let (mut client, mut server) = UnixStream::pair().unwrap();
    let (tx, rx) = channel();
    let payload = format!(
        "ssh://{}@host.example\n",
        "a".repeat(MAX_REQUEST_BYTES as usize)
    );
    let write = async {
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

#[test]
fn stale_socket_is_reclaimed_on_bind_conflict() {
    let path = temp_socket_path("stale");
    std::fs::write(&path, b"stale").unwrap();

    let primary = acquire_primary_at(&path, &LaunchRequest::default())
        .unwrap()
        .expect("陈旧端点应被清理并重新绑定");
    assert!(probe_alive(&path), "重新绑定后本进程应持有可连接的套接字");

    drop(primary);
    let _ = std::fs::remove_file(&path);
}
