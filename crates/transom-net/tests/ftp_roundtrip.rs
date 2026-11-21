//! FTP/FTPS 客户端回环集成测试：进程内 mock 服务器上的完整协议对话。
//!
//! 覆盖面板执行器的全部能力面：登录、列表（MLSD）、属性（MLST/SIZE/MDTM）、
//! 建目录、重命名、删除（含递归）、单文件上传下载（覆盖与双向断点续传）、
//! 目录递归传输、错误语义与断线自愈。
//!
//! 测试布局约定：本地夹具目录与 mock 服务器根目录必须是两个不同目录——
//! STOR 会在服务端创建/截断目标文件，若本地源文件与远端路径落在同一物理
//! 目录，截断竞态会让上传读到被清空的源文件，这不是客户端缺陷。

mod common;

use common::mock_ftp::{MOCK_FTP_PASSWORD, MOCK_FTP_USER, spawn, spawn_with_command_limit};
use tempfile::TempDir;
use transom_core::{FtpConnection, FtpSecurity, TransferFilter, TransferStart};
use transom_net::FtpClient;

/// 以测试凭据连接 mock 服务器。
async fn connect(config: &FtpConnection) -> FtpClient {
    FtpClient::open(config, Some(MOCK_FTP_PASSWORD))
        .await
        .unwrap()
}

/// 构造指向 mock 服务器的连接参数。
fn ftp_config(port: u16) -> FtpConnection {
    FtpConnection {
        host: "127.0.0.1".into(),
        port,
        username: MOCK_FTP_USER.into(),
        security: FtpSecurity::Plain,
        passive: true,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ftp_login_list_transfer_and_mutations() {
    // 远端视图（mock 根目录）与本地夹具目录严格分离。
    let remote_dir = TempDir::new().unwrap();
    let local_dir = TempDir::new().unwrap();
    std::fs::write(remote_dir.path().join("seed.txt"), b"seed-content").unwrap();
    let (port, _server) = spawn(remote_dir.path()).await;
    let config = ftp_config(port);
    let client = connect(&config).await;

    // 登录主目录与目录列表：MLSD 机器格式解析为领域条目。
    let (home, entries) = client.list("/").await.unwrap();
    assert_eq!(home, "/");
    assert_eq!(entries.len(), 1, "只应列出 seed.txt 一个条目");
    let entry = &entries[0];
    assert_eq!(entry.name, "seed.txt");
    assert!(!entry.is_dir);
    assert_eq!(entry.size, "seed-content".len() as u64);
    assert_eq!(entry.path, "/seed.txt");
    assert_eq!(entry.permissions, Some(0o644), "MLSD 的 UNIX.mode 应落契约");

    // 属性查询：MLST 单次往返带尺寸与 mtime。
    let metadata = client.stat("/seed.txt").await.unwrap();
    assert!(!metadata.is_dir);
    assert_eq!(metadata.size, "seed-content".len() as u64);
    assert!(metadata.mtime.is_some(), "MLST 应回传修改时间");

    // 上传（覆盖）：内容逐字节落盘，返回值为累计字节数。
    let payload = b"upload-body-0123456789".to_vec();
    let upload_source = local_dir.path().join("uploaded.bin");
    std::fs::write(&upload_source, &payload).unwrap();
    let transferred = client
        .upload_resumable(
            &upload_source,
            "/uploaded.bin",
            TransferStart::Overwrite,
            |_, _| {},
        )
        .await
        .unwrap()
        .transferred;
    assert_eq!(transferred, payload.len() as u64);
    assert_eq!(
        std::fs::read(remote_dir.path().join("uploaded.bin")).unwrap(),
        payload,
        "上传内容必须逐字节一致"
    );

    // 下载（覆盖）：远端内容还原到本地目录。
    let download_target = local_dir.path().join("downloaded.txt");
    let transferred = client
        .download_resumable(
            "/seed.txt",
            &download_target,
            TransferStart::Overwrite,
            |_, _| {},
        )
        .await
        .unwrap()
        .transferred;
    assert_eq!(transferred, "seed-content".len() as u64);
    assert_eq!(std::fs::read(&download_target).unwrap(), b"seed-content");

    // 建目录与重命名。
    client.create_dir("/archive").await.unwrap();
    client
        .rename("/seed.txt", "/archive/seed-renamed.txt")
        .await
        .unwrap();
    assert!(remote_dir.path().join("archive/seed-renamed.txt").exists());
    assert!(!remote_dir.path().join("seed.txt").exists());
    let (_, entries) = client.list("/").await.unwrap();
    assert_eq!(entries.len(), 2, "archive 目录与 uploaded.bin");
    assert!(entries[0].is_dir, "目录必须排在文件之前");

    // 递归删除：目录内文件先删、目录自底向上移除。
    client.delete("/archive", true).await.unwrap();
    assert!(!remote_dir.path().join("archive").exists());

    client.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ftp_resume_continues_from_offset_in_both_directions() {
    let remote_dir = TempDir::new().unwrap();
    let local_dir = TempDir::new().unwrap();
    let (port, _server) = spawn(remote_dir.path()).await;
    let config = ftp_config(port);
    let client = connect(&config).await;

    // 上传续传：远端已有前 10 字节，Continue 从偏移处覆写剩余内容。
    // 计数口径与 SFTP 侧一致：transferred 为累计值（偏移 + 本次字节）。
    let full = b"0123456789XYZ".to_vec();
    let source = local_dir.path().join("full.bin");
    std::fs::write(&source, &full).unwrap();
    std::fs::write(remote_dir.path().join("remote.bin"), b"0123456789").unwrap();
    let transferred = client
        .upload_resumable(
            &source,
            "/remote.bin",
            TransferStart::Continue { offset: 10 },
            |_, _| {},
        )
        .await
        .unwrap()
        .transferred;
    assert_eq!(transferred, full.len() as u64, "累计计数含断点前字节");
    assert_eq!(transferred - 10, 3, "本次实际只传偏移之后的 3 字节");
    assert_eq!(
        std::fs::read(remote_dir.path().join("remote.bin")).unwrap(),
        full,
        "REST+STOR 必须在偏移处覆写而不是追加"
    );

    // 下载续传：本地已有前 10 字节，Continue 只补齐偏移之后的内容。
    let target = local_dir.path().join("resumed.bin");
    std::fs::write(&target, b"0123456789").unwrap();
    let transferred = client
        .download_resumable(
            "/remote.bin",
            &target,
            TransferStart::Continue { offset: 10 },
            |_, _| {},
        )
        .await
        .unwrap()
        .transferred;
    assert_eq!(transferred, full.len() as u64);
    assert_eq!(
        std::fs::read(&target).unwrap(),
        full,
        "下载续传拼接结果一致"
    );

    // 偏移超出远端长度时必须退回整体覆盖（断点失效保护）。
    let transferred = client
        .upload_resumable(
            &source,
            "/remote.bin",
            TransferStart::Continue { offset: 999 },
            |_, _| {},
        )
        .await
        .unwrap()
        .transferred;
    assert_eq!(transferred, full.len() as u64, "断点失效退回从头覆盖");
    assert_eq!(
        std::fs::read(remote_dir.path().join("remote.bin")).unwrap(),
        full
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ftp_directory_transfer_and_error_semantics() {
    let remote_dir = TempDir::new().unwrap();
    let local_dir = TempDir::new().unwrap();
    let (port, _server) = spawn(remote_dir.path()).await;
    let config = ftp_config(port);
    let client = connect(&config).await;

    // 本地构造嵌套目录树后整体上传；空目录也必须同步。
    let source_root = local_dir.path().join("tree");
    std::fs::create_dir_all(source_root.join("nested")).unwrap();
    std::fs::write(source_root.join("a.txt"), b"alpha").unwrap();
    std::fs::write(source_root.join("nested/b.txt"), b"beta").unwrap();
    std::fs::create_dir(source_root.join("empty")).unwrap();

    let summary = client
        .upload_dir(&source_root, "/tree", &TransferFilter::default(), |_, _| {})
        .await
        .unwrap();
    assert_eq!(summary.transferred, ("alpha".len() + "beta".len()) as u64);
    assert_eq!(
        std::fs::read(remote_dir.path().join("tree/a.txt")).unwrap(),
        b"alpha"
    );
    assert_eq!(
        std::fs::read(remote_dir.path().join("tree/nested/b.txt")).unwrap(),
        b"beta"
    );
    assert!(
        remote_dir.path().join("tree/empty").is_dir(),
        "空目录也要同步"
    );

    // 整体下载还原目录结构。
    let target_root = local_dir.path().join("downloaded-tree");
    let summary = client
        .download_dir("/tree", &target_root, &TransferFilter::default(), |_, _| {})
        .await
        .unwrap();
    assert_eq!(summary.transferred, ("alpha".len() + "beta".len()) as u64);
    assert_eq!(std::fs::read(target_root.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(
        std::fs::read(target_root.join("nested/b.txt")).unwrap(),
        b"beta"
    );

    // 错误语义：认证失败按 AuthFailed 归类；目录不存在按 FtpFailed。
    let bad_config = FtpConnection {
        host: "127.0.0.1".into(),
        port,
        username: MOCK_FTP_USER.into(),
        security: FtpSecurity::Plain,
        passive: true,
    };
    let Err(error) = FtpClient::open(&bad_config, Some("wrong-password")).await else {
        panic!("错误密码不应登录成功");
    };
    assert_eq!(error.code, transom_core::ErrorCode::AuthFailed);
    let error = client.list("/nonexistent").await.unwrap_err();
    assert_eq!(error.code, transom_core::ErrorCode::FtpFailed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ftp_reconnect_heals_broken_control_connection() {
    // 断线自愈：服务端在处理 10 条命令后静默掐断控制连接。客户端的下一次
    // 操作感知连接级故障（ConnectionFailed，连接已废弃待重建），再下一次
    // 操作自动重连重登录恢复正常——空闲控制连接被服务端掐断是 FTP 的常态，
    // 调用方无需手动重建会话。
    let remote_dir = TempDir::new().unwrap();
    std::fs::write(remote_dir.path().join("alive.txt"), b"still-here").unwrap();
    let (port, _server) = spawn_with_command_limit(remote_dir.path(), 10).await;
    let config = ftp_config(port);
    let client = connect(&config).await;

    // 登录（USER/PASS/TYPE/PWD 共 4 条）+ 首次列表（PASV/MLSD 2 条）后仍在限额内。
    let (path, entries) = client.list("/").await.unwrap();
    assert_eq!(path, "/");
    assert_eq!(entries.len(), 1);

    // 继续正常使用直至越过命令限额，连接被掐断。
    loop {
        match client.list("/").await {
            Ok(_) => continue,
            Err(error) => {
                assert_eq!(
                    error.code,
                    transom_core::ErrorCode::ConnectionFailed,
                    "连接被掐断后应感知连接级故障"
                );
                break;
            }
        }
    }

    // 下一次操作自动重连重登录：无需调用方干预。
    let (_, entries) = client.list("/").await.unwrap();
    assert_eq!(entries.len(), 1, "自愈后列表恢复正常");
    assert_eq!(entries[0].name, "alive.txt");
}
