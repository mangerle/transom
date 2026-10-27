use russh::keys::{Algorithm, PrivateKey, key::safe_rng};
use russh::server::{self, Auth, Msg, Server as _};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode,
};
use russh_sftp::server::Handler as SftpHandler;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use transom_core::{
    AuthMethod, ConnectionSecrets, DirDiffStatus, SessionOptions, SshConnection, TerminalSize,
    TransferFilter,
};
use transom_net::{PasswordPrompter, SftpClient, SshSession, connect_ssh};
use transom_store::JsonKnownHostsStore;

struct TestServer {
    root: Arc<PathBuf>,
}

impl server::Server for TestServer {
    type Handler = TestServerHandler;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        TestServerHandler {
            root: self.root.clone(),
            channels: HashMap::new(),
        }
    }
}

struct TestServerHandler {
    root: Arc<PathBuf>,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl server::Handler for TestServerHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if name == "sftp"
            && let Some(channel) = self.channels.remove(&channel_id)
        {
            let root = self.root.clone();
            let _ = session.channel_success(channel_id);
            tokio::spawn(async move {
                russh_sftp::server::run(
                    channel.into_stream(),
                    FsSftp {
                        root,
                        readdir_done: HashSet::new(),
                    },
                )
                .await;
            });
        }
        Ok(())
    }
}

struct FsSftp {
    root: Arc<PathBuf>,
    readdir_done: HashSet<String>,
}

impl FsSftp {
    fn local(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    fn attrs(&self, path: &str) -> Result<FileAttributes, StatusCode> {
        self.attrs_with(path, true)
    }

    /// OpenSSH 一致的 lstat 语义属性（不跟随符号链接，供 LSTAT 与 READDIR 使用）
    fn lstat_attrs(&self, path: &str) -> Result<FileAttributes, StatusCode> {
        self.attrs_with(path, false)
    }

    fn attrs_with(&self, path: &str, follow: bool) -> Result<FileAttributes, StatusCode> {
        let fs_path = self.local(path);
        let metadata = if follow {
            std::fs::metadata(&fs_path)
        } else {
            std::fs::symlink_metadata(&fs_path)
        }
        .map_err(|_| StatusCode::NoSuchFile)?;
        let mut attrs = FileAttributes::empty();
        attrs.size = Some(metadata.len());
        if metadata.is_dir() {
            attrs.set_dir(true);
        } else if metadata.file_type().is_symlink() {
            attrs.set_symlink(true);
        } else {
            attrs.set_regular(true);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            attrs.permissions = Some(metadata.permissions().mode());
        }
        attrs.mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as u32);
        Ok(attrs)
    }
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_string(),
        language_tag: "en".to_string(),
    }
}

impl SftpHandler for FsSftp {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let mut options = std::fs::OpenOptions::new();
        options.read(pflags.contains(OpenFlags::READ));
        options.write(pflags.contains(OpenFlags::WRITE));
        if pflags.contains(OpenFlags::CREATE) {
            if pflags.contains(OpenFlags::EXCLUDE) {
                options.create_new(true);
            } else {
                options.create(true);
            }
        }
        if pflags.contains(OpenFlags::TRUNCATE) {
            options.truncate(true);
        }
        options
            .open(self.local(&filename))
            .map_err(|_| StatusCode::Failure)?;
        Ok(Handle {
            id,
            handle: filename,
        })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let mut file = tokio::fs::File::open(self.local(&handle))
            .await
            .map_err(|_| StatusCode::Failure)?;
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|_| StatusCode::Failure)?;
        let mut data = vec![0u8; len as usize];
        let count = file
            .read(&mut data)
            .await
            .map_err(|_| StatusCode::Failure)?;
        if count == 0 {
            return Err(StatusCode::Eof);
        }
        data.truncate(count);
        Ok(Data { id, data })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.local(&handle))
            .await
            .map_err(|_| StatusCode::Failure)?;
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|_| StatusCode::Failure)?;
        file.write_all(&data)
            .await
            .map_err(|_| StatusCode::Failure)?;
        file.flush().await.map_err(|_| StatusCode::Failure)?;
        Ok(ok(id))
    }

    async fn close(&mut self, id: u32, _handle: String) -> Result<Status, Self::Error> {
        Ok(ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        if !self.local(&path).is_dir() {
            return Err(StatusCode::NoSuchFile);
        }
        self.readdir_done.remove(&path);
        Ok(Handle { id, handle: path })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        if !self.readdir_done.insert(handle.clone()) {
            return Err(StatusCode::Eof);
        }
        let mut files = Vec::new();
        for entry in std::fs::read_dir(self.local(&handle)).map_err(|_| StatusCode::Failure)? {
            let entry = entry.map_err(|_| StatusCode::Failure)?;
            let name = entry.file_name().to_string_lossy().to_string();
            let path = format!("{}/{}", handle.trim_end_matches('/'), name);
            if let Ok(attrs) = self.lstat_attrs(&path) {
                files.push(File::new(name, attrs));
            }
        }
        Ok(Name { id, files })
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let path = if path == "." || path.is_empty() {
            "/".to_string()
        } else if path.starts_with('/') {
            path
        } else {
            format!("/{path}")
        };
        Ok(Name {
            id,
            files: vec![File::dummy(path)],
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        Ok(Attrs {
            id,
            attrs: self.attrs(&path)?,
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        Ok(Attrs {
            id,
            attrs: self.lstat_attrs(&path)?,
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        self.stat(id, handle).await
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        std::fs::create_dir(self.local(&path)).map_err(|_| StatusCode::Failure)?;
        Ok(ok(id))
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        std::fs::remove_file(self.local(&filename)).map_err(|_| StatusCode::Failure)?;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        std::fs::remove_dir(self.local(&path)).map_err(|_| StatusCode::Failure)?;
        Ok(ok(id))
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        std::fs::rename(self.local(&oldpath), self.local(&newpath))
            .map_err(|_| StatusCode::Failure)?;
        Ok(ok(id))
    }

    /// 模拟 OpenSSH 事实标准：报文首参为被指向目标（oldpath），次参为新建链接（newpath）。
    /// russh-sftp 服务端骨架按协议草案命名形参（首参落在 `linkpath`、次参落在 `targetpath`），
    /// 因此这里将首参当作目标、次参当作链接路径调用系统 symlink，以验证客户端报文顺序正确。
    async fn symlink(
        &mut self,
        id: u32,
        linkpath: String,
        targetpath: String,
    ) -> Result<Status, Self::Error> {
        let (oldpath, newpath) = (linkpath, targetpath);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(oldpath, self.local(&newpath))
                .map_err(|_| StatusCode::Failure)?;
            Ok(ok(id))
        }
        #[cfg(not(unix))]
        {
            let _ = (id, oldpath, newpath);
            Err(StatusCode::OpUnsupported)
        }
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let target = std::fs::read_link(self.local(&path)).map_err(|_| StatusCode::NoSuchFile)?;
        Ok(Name {
            id,
            files: vec![File::dummy(target.to_string_lossy().to_string())],
        })
    }

    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        if let Some(mtime) = attrs.mtime {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(self.local(&path))
                .map_err(|_| StatusCode::Failure)?;
            file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(u64::from(mtime)))
                .map_err(|_| StatusCode::Failure)?;
        }
        Ok(ok(id))
    }
}

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
            TestServer { root: server_root }.new_client(None),
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
    assert_eq!(transferred, 10);
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
    assert_eq!(transferred, 5);
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
