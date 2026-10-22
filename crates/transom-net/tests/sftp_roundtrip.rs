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
use transom_core::{AuthMethod, ConnectionSecrets, SessionOptions, SshConnection, TerminalSize};
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
        let metadata = std::fs::metadata(self.local(path)).map_err(|_| StatusCode::NoSuchFile)?;
        let mut attrs = FileAttributes::empty();
        attrs.size = Some(metadata.len());
        if metadata.is_dir() {
            attrs.set_dir(true);
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
            if let Ok(attrs) = self.attrs(&path) {
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
        self.stat(id, path).await
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
        },
    };

    let session = tokio::time::timeout(
        Duration::from_secs(8),
        connect_ssh(
            &connection,
            TerminalSize::new(80, 24),
            ConnectionSecrets {
                password: Some(zeroize::Zeroizing::new("secret".to_string())),
                key_passphrase: None,
            },
            Arc::new(PasswordPrompter::new("secret")),
            known_hosts,
        ),
    )
    .await
    .expect("connect timed out")
    .expect("connect failed");

    let SshSession { io, join, remote } = session;
    let client = SftpClient::open(&remote).await.expect("open sftp");

    let (path, entries) = client.list("/").await.expect("list root");
    assert_eq!(path, "/");
    assert!(entries.iter().any(|entry| entry.name == "seed.txt"));

    let local = dir.path().join("local.txt");
    tokio::fs::write(&local, b"hello sftp").await.unwrap();
    let transferred = client
        .upload(&local, "/uploaded.txt", |_, _| {})
        .await
        .expect("upload");
    assert_eq!(transferred, 10);

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

    client.close().await;
    drop(client);
    drop(io);
    let _ = tokio::time::timeout(Duration::from_secs(3), join).await;
    server_task.abort();
}
