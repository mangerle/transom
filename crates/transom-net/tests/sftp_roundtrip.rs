mod common;

use common::mock_sftp::TestServer;
use russh::keys::{Algorithm, PrivateKey, key::safe_rng};
use russh::server::{self, Server as _};
use std::sync::Arc;
use std::time::Duration;
use transom_core::{
    AuthMethod, ConnectionSecrets, DirDiffStatus, SessionOptions, SshConnection, TerminalSize,
    TransferFilter,
};
use transom_net::{PasswordPrompter, SftpClient, SshSession, connect_ssh};
use transom_store::JsonKnownHostsStore;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sftp_list_upload_download_and_mutations() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("seed.txt"), b"seed").unwrap();
    let known_hosts =
        Arc::new(JsonKnownHostsStore::new(dir.path().join("known_hosts.json")).unwrap());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let mut server_config = server::Config {
        inactivity_timeout: None,
        ..Default::default()
    };
    server_config
        .keys
        .push(PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap());
    let server_config = Arc::new(server_config);
    let server_root = Arc::new(dir.path().to_path_buf());
    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let handle = server::run_stream(
            server_config,
            socket,
            TestServer::new(server_root).new_client(None),
        )
        .await
        .unwrap();
        let _ = handle.await;
    });

    let connection = SshConnection {
        host: "127.0.0.1".into(),
        port,
        username: "tester".into(),
        auth: AuthMethod::Password { save: false },
        options: SessionOptions {
            connect_timeout_secs: 5,
            keepalive_secs: 0,
            allow_keyboard_interactive: false,

            auto_reconnect: false,
            agent_forward: false,
            wol_mac: None,
        },
        proxy_jump: None,
    };

    let session = tokio::time::timeout(
        Duration::from_secs(8),
        connect_ssh(transom_net::SshConnectParams {
            connection: &connection,
            jumps: &[],
            size: TerminalSize::new(80, 24),
            credentials: ConnectionSecrets {
                password: Some(zeroize::Zeroizing::new("secret".to_string())),
                key_passphrase: None,
            },
            prompter: Arc::new(PasswordPrompter::new("secret")),
            known_hosts,
            proxy: transom_core::OutboundProxy::default(),
            allow_sha1_publickey_auth: true,
        }),
    )
    .await
    .expect("connect timed out")
    .expect("connect failed");

    let SshSession {
        io, join, remote, ..
    } = session;
    let client = SftpClient::open(&remote).await.expect("open sftp");

    let (path, entries) = client.list("/").await.expect("list root");
    assert_eq!(path, "/");
    assert!(entries.iter().any(|entry| entry.name == "seed.txt"));

    let local = dir.path().join("local.txt");
    tokio::fs::write(&local, b"hello sftp").await.unwrap();
    set_modified(&local, 12_345);
    let transferred = client
        .upload(&local, "/uploaded.txt", |_, _| {})
        .await
        .expect("upload");
    assert_eq!(transferred, 10);
    assert_eq!(
        client
            .stat("/uploaded.txt")
            .await
            .expect("stat uploaded")
            .mtime,
        Some(12_345),
        "上传后应保留本地文件修改时间"
    );
    client
        .verify_file(&local, "/uploaded.txt")
        .await
        .expect("上传文件 SHA256 校验");
    client
        .verify_file_with_known_local(
            &local,
            "/uploaded.txt",
            Some("a3ca2ea509ad4a9e3673a7652100889535d9d73d3434f02bc07f0ded2f65187c"),
        )
        .await
        .expect("在途哈希一致时校验成功");
    assert!(
        client
            .verify_file_with_known_local(&local, "/uploaded.txt", Some("bad_hash_digest"))
            .await
            .is_err(),
        "在途哈希不一致时校验必须失败"
    );
    tokio::fs::write(&local, b"tampered").await.unwrap();
    assert!(
        client.verify_file(&local, "/uploaded.txt").await.is_err(),
        "本地内容被篡改后校验必须失败"
    );
    tokio::fs::write(&local, b"hello sftp").await.unwrap();

    let downloaded = dir.path().join("downloaded.txt");
    let transferred = client
        .download("/uploaded.txt", &downloaded, |_, _| {})
        .await
        .expect("download");
    assert_eq!(transferred, 10);
    assert_eq!(
        tokio::fs::read(&downloaded).await.unwrap(),
        b"hello sftp".to_vec()
    );
    assert_eq!(
        modified_seconds(&downloaded),
        12_345,
        "下载后应保留远端文件修改时间"
    );

    let oversized_local = dir.path().join("oversized-local.txt");
    tokio::fs::write(&oversized_local, b"0123456789ABCDEF")
        .await
        .unwrap();
    let transferred = client
        .download_resumable("/uploaded.txt", &oversized_local, true, |_, _| {})
        .await
        .expect("目标更大时应截断重传");
    assert_eq!(transferred.transferred, 10);
    assert!(
        transferred.local_sha256.is_some(),
        "截断全量重传应产生在途哈希"
    );
    assert_eq!(
        tokio::fs::read(&oversized_local).await.unwrap(),
        b"hello sftp".to_vec(),
        "下载续传不能保留目标文件多余的尾部"
    );

    let small_local = dir.path().join("small.txt");
    tokio::fs::write(&small_local, b"small").await.unwrap();
    std::fs::write(dir.path().join("remote-larger.txt"), b"0123456789ABCDEF").unwrap();
    let transferred = client
        .upload_resumable(&small_local, "/remote-larger.txt", true, |_, _| {})
        .await
        .expect("远端更大时应截断重传");
    assert_eq!(transferred.transferred, 5);
    assert!(
        transferred.local_sha256.is_some(),
        "截断全量上传应产生在途哈希"
    );
    assert_eq!(
        std::fs::read(dir.path().join("remote-larger.txt")).unwrap(),
        b"small",
        "上传续传不能保留远端文件多余的尾部"
    );

    // 9 MiB 往返覆盖深预取（单次 READ 约 255 KiB）与跨多个预取窗口的 EOF
    // 边界；旧实现的 8 MiB 分块分支已移除，这里确认大文件仍能完整往返。
    let large_local = dir.path().join("large-local.bin");
    let large_bytes: Vec<u8> = (0..9 * 1024 * 1024)
        .map(|index| (index % 251) as u8)
        .collect();
    tokio::fs::write(&large_local, &large_bytes).await.unwrap();
    let transferred = client
        .upload(&large_local, "/large.bin", |_, _| {})
        .await
        .expect("上传大文件");
    assert_eq!(transferred, large_bytes.len() as u64);
    let large_downloaded = dir.path().join("large-downloaded.bin");
    let transferred = client
        .download("/large.bin", &large_downloaded, |_, _| {})
        .await
        .expect("下载大文件");
    assert_eq!(transferred, large_bytes.len() as u64);
    assert_eq!(
        tokio::fs::read(&large_downloaded).await.unwrap(),
        large_bytes,
        "大文件往返内容必须一致"
    );

    let concurrency_dir = dir.path().join("concurrency-src");
    std::fs::create_dir_all(&concurrency_dir).unwrap();
    let concurrent_one = concurrency_dir.join("one.txt");
    let concurrent_two = concurrency_dir.join("two.txt");
    tokio::fs::write(&concurrent_one, b"first").await.unwrap();
    tokio::fs::write(&concurrent_two, b"second").await.unwrap();
    let (first, second) = tokio::join!(
        client.upload(&concurrent_one, "/concurrent-one.txt", |_, _| {}),
        client.upload(&concurrent_two, "/concurrent-two.txt", |_, _| {})
    );
    assert_eq!(first.expect("并发上传一"), 5);
    assert_eq!(second.expect("并发上传二"), 6);
    assert_eq!(
        std::fs::read(dir.path().join("concurrent-one.txt")).unwrap(),
        b"first"
    );
    assert_eq!(
        std::fs::read(dir.path().join("concurrent-two.txt")).unwrap(),
        b"second"
    );

    client.create_dir("/folder").await.expect("mkdir");
    client
        .rename("/uploaded.txt", "/folder/moved.txt")
        .await
        .expect("rename");
    let (_, entries) = client.list("/folder").await.expect("list folder");
    assert!(entries.iter().any(|entry| entry.name == "moved.txt"));
    client
        .delete("/folder/moved.txt", false)
        .await
        .expect("delete file");
    client.delete("/folder", true).await.expect("delete folder");

    client
        .create_dir("/recursive")
        .await
        .expect("mkdir recursive");
    client
        .create_dir("/recursive/a")
        .await
        .expect("mkdir recursive/a");
    client
        .create_dir("/recursive/a/b")
        .await
        .expect("mkdir recursive/a/b");
    std::fs::write(dir.path().join("recursive/root.txt"), b"root").unwrap();
    std::fs::write(dir.path().join("recursive/a/one.txt"), b"one").unwrap();
    std::fs::write(dir.path().join("recursive/a/b/two.txt"), b"two").unwrap();
    client
        .symlink("/seed.txt", "/recursive/seed.link")
        .await
        .expect("symlink inside recursive dir");
    client
        .delete("/recursive", true)
        .await
        .expect("非空目录应可递归删除");
    assert!(!dir.path().join("recursive").exists());
    assert!(
        dir.path().join("seed.txt").exists(),
        "递归删除不得跟随符号链接误删目标"
    );

    let meta = client.stat("/seed.txt").await.expect("stat seed");
    assert!(!meta.is_dir);
    assert!(!meta.is_symlink);
    assert_eq!(meta.size, 4);
    assert!(meta.mtime.is_some());

    client
        .symlink("/seed.txt", "/seed.link")
        .await
        .expect("create symlink");
    let link_meta = client.stat("/seed.link").await.expect("stat symlink");
    assert!(link_meta.is_symlink);
    assert_eq!(link_meta.link_target.as_deref(), Some("/seed.txt"));
    #[cfg(unix)]
    assert_eq!(
        std::fs::read_link(dir.path().join("seed.link")).expect("read created link"),
        std::path::PathBuf::from("/seed.txt")
    );
    client
        .delete("/seed.link", false)
        .await
        .expect("delete symlink");
    assert!(!dir.path().join("seed.link").exists());
    assert!(dir.path().join("seed.txt").exists());

    let local_tree = dir.path().join("local-tree");
    std::fs::create_dir_all(local_tree.join("src")).unwrap();
    std::fs::create_dir_all(local_tree.join("empty/nested")).unwrap();
    std::fs::create_dir_all(local_tree.join(".git/objects")).unwrap();
    std::fs::create_dir_all(local_tree.join("node_modules/pkg")).unwrap();
    std::fs::write(local_tree.join("src/main.rs"), b"fn main() {}\n").unwrap();
    set_modified(&local_tree.join("src/main.rs"), 5_000);
    std::fs::write(local_tree.join(".git/config"), b"[core]\n").unwrap();
    std::fs::write(
        local_tree.join("node_modules/pkg/index.js"),
        b"module.exports = 1;\n",
    )
    .unwrap();
    std::fs::write(local_tree.join("cache.tmp"), b"temporary").unwrap();

    let filter = TransferFilter::from_text(".git/\nnode_modules/\n*.tmp");
    let expected_tree_bytes = "fn main() {}\n".len() as u64;
    let summary = client
        .upload_dir(&local_tree, "/tree", true, &filter, true, |_, _| {})
        .await
        .expect("upload directory");
    assert_eq!(summary.transferred, expected_tree_bytes);
    assert!(summary.skipped >= 3, "应跳过 .git、node_modules 与 *.tmp");

    let (_, tree_entries) = client.list("/tree").await.expect("list uploaded tree");
    assert!(
        tree_entries
            .iter()
            .any(|entry| entry.name == "src" && entry.is_dir)
    );
    assert!(
        tree_entries
            .iter()
            .any(|entry| entry.name == "empty" && entry.is_dir)
    );
    assert!(!tree_entries.iter().any(|entry| entry.name == ".git"));
    assert!(
        !tree_entries
            .iter()
            .any(|entry| entry.name == "node_modules")
    );
    assert!(!tree_entries.iter().any(|entry| entry.name == "cache.tmp"));
    let empty_meta = client.stat("/tree/empty/nested").await.expect("empty dir");
    assert!(empty_meta.is_dir);
    assert_eq!(
        client
            .stat("/tree/src/main.rs")
            .await
            .expect("stat uploaded tree file")
            .mtime,
        Some(5_000)
    );

    let downloaded_tree = dir.path().join("downloaded-tree");
    let summary = client
        .download_dir("/tree", &downloaded_tree, true, &filter, true, |_, _| {})
        .await
        .expect("download directory");
    assert_eq!(summary.transferred, expected_tree_bytes);
    assert_eq!(
        std::fs::read(downloaded_tree.join("src/main.rs")).unwrap(),
        b"fn main() {}\n"
    );
    assert_eq!(
        modified_seconds(&downloaded_tree.join("src/main.rs")),
        5_000
    );
    assert!(downloaded_tree.join("empty/nested").is_dir());
    assert!(!downloaded_tree.join(".git").exists());

    let invalid_filter = TransferFilter::from_text("[broken");
    assert!(
        client
            .upload_dir(
                &local_tree,
                "/invalid",
                true,
                &invalid_filter,
                true,
                |_, _| {}
            )
            .await
            .is_err(),
        "非法过滤规则应直接失败"
    );

    let sync_local = dir.path().join("sync-local");
    let sync_remote = dir.path().join("sync-remote");
    std::fs::create_dir_all(&sync_local).unwrap();
    std::fs::create_dir_all(&sync_remote).unwrap();
    std::fs::create_dir_all(sync_local.join("empty-local")).unwrap();
    std::fs::create_dir_all(sync_local.join(".git")).unwrap();
    std::fs::create_dir_all(sync_remote.join(".git")).unwrap();
    std::fs::write(sync_local.join(".git/config"), b"local-git").unwrap();
    std::fs::write(sync_remote.join(".git/config"), b"remote-git").unwrap();
    std::fs::write(sync_local.join("same.txt"), b"same").unwrap();
    std::fs::write(sync_remote.join("same.txt"), b"same").unwrap();
    set_modified(&sync_local.join("same.txt"), 1_000);
    set_modified(&sync_remote.join("same.txt"), 1_000);
    std::fs::write(sync_local.join("local-newer.txt"), b"local-newer!").unwrap();
    std::fs::write(sync_remote.join("local-newer.txt"), b"remote-newer").unwrap();
    set_modified(&sync_local.join("local-newer.txt"), 2_000);
    set_modified(&sync_remote.join("local-newer.txt"), 1_000);
    std::fs::write(sync_local.join("remote-newer.txt"), b"local-newer!").unwrap();
    std::fs::write(sync_remote.join("remote-newer.txt"), b"remote-newer").unwrap();
    set_modified(&sync_local.join("remote-newer.txt"), 1_000);
    set_modified(&sync_remote.join("remote-newer.txt"), 2_000);
    std::fs::write(sync_local.join("local-only.txt"), b"local-only").unwrap();
    std::fs::write(sync_remote.join("remote-only.txt"), b"remote-only!").unwrap();
    std::fs::write(sync_local.join("conflict"), b"file").unwrap();
    std::fs::create_dir_all(sync_remote.join("conflict")).unwrap();
    std::fs::write(sync_remote.join("conflict/child.txt"), b"child").unwrap();

    let filter = TransferFilter::from_text(".git/");
    let comparison = client
        .compare_directories(&sync_local, "/sync-remote", &filter)
        .await
        .expect("compare directories");
    assert_eq!(comparison.skipped, 2, "两端各有被排除的 .git 目录");
    let status_of = |path: &str| {
        comparison
            .entries
            .iter()
            .find(|entry| entry.relative_path == path)
            .map(|entry| entry.status)
    };
    assert_eq!(
        status_of("local-newer.txt"),
        Some(DirDiffStatus::LocalNewer)
    );
    assert_eq!(
        status_of("remote-newer.txt"),
        Some(DirDiffStatus::RemoteNewer)
    );
    assert_eq!(status_of("local-only.txt"), Some(DirDiffStatus::LocalOnly));
    assert_eq!(
        status_of("remote-only.txt"),
        Some(DirDiffStatus::RemoteOnly)
    );
    assert_eq!(status_of("empty-local"), Some(DirDiffStatus::LocalOnly));
    assert_eq!(status_of("conflict"), Some(DirDiffStatus::TypeMismatch));
    assert_eq!(
        status_of("conflict/child.txt"),
        Some(DirDiffStatus::TypeMismatch),
        "祖先类型冲突必须传染到子树"
    );
    assert!(
        !comparison
            .entries
            .iter()
            .any(|entry| entry.relative_path == "same.txt")
    );

    let items = comparison
        .entries
        .iter()
        .filter_map(|entry| entry.sync_item())
        .collect::<Vec<_>>();
    let expected_sync_bytes = items.iter().map(|item| item.size).sum::<u64>();
    let summary = client
        .sync_items(&sync_local, "/sync-remote", &items, true, |_, _| {})
        .await
        .expect("sync directories");
    assert_eq!(summary.transferred, expected_sync_bytes);
    assert_eq!(
        std::fs::read(sync_remote.join("local-only.txt")).unwrap(),
        b"local-only"
    );
    assert_eq!(
        std::fs::read(sync_remote.join("local-newer.txt")).unwrap(),
        b"local-newer!"
    );
    assert_eq!(
        std::fs::read(sync_local.join("remote-newer.txt")).unwrap(),
        b"remote-newer"
    );
    assert_eq!(
        std::fs::read(sync_local.join("remote-only.txt")).unwrap(),
        b"remote-only!"
    );
    assert!(sync_remote.join("empty-local").is_dir());
    assert!(sync_local.join("conflict").is_file());
    assert!(sync_remote.join("conflict").is_dir());
    assert_eq!(
        std::fs::read(sync_remote.join(".git/config")).unwrap(),
        b"remote-git",
        "被排除的远端目录不应被同步修改"
    );
    assert!(sync_local.join(".git/config").exists());

    let comparison_after_sync = client
        .compare_directories(&sync_local, "/sync-remote", &filter)
        .await
        .expect("compare after sync");
    assert!(
        comparison_after_sync
            .entries
            .iter()
            .all(|entry| entry.status == DirDiffStatus::TypeMismatch),
        "同步后除类型冲突外不应残留差异: {:?}",
        comparison_after_sync.entries
    );

    client.close().await;
    drop(client);
    drop(io);
    let _ = tokio::time::timeout(Duration::from_secs(3), join).await;
    server_task.abort();
}

fn set_modified(path: &std::path::Path, seconds: u64) {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("打开文件以设置修改时间");
    file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(seconds))
        .expect("设置文件修改时间");
}

fn modified_seconds(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .expect("读取文件属性")
        .modified()
        .expect("读取修改时间")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("修改时间早于 Unix 纪元")
        .as_secs()
}
