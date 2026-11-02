use md5::Md5;
use russh::server::{self, Auth, Msg};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode,
};
use russh_sftp::server::Handler as SftpHandler;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub struct TestServer {
    pub root: Arc<PathBuf>,
}

impl TestServer {
    pub fn new(root: Arc<PathBuf>) -> Self {
        Self { root }
    }
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

pub struct TestServerHandler {
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

    /// 模拟 Linux 校验命令（sha256sum/md5sum 及其 BSD 变体）。
    ///
    /// 设计原理（Why）：客户端会先用 SSH exec 在服务端本地算哈希，失败才降级为
    /// SFTP 流式读取。若 mock 不响应 exec 请求，每次校验都要等满客户端 30 秒
    /// 超时，整个往返测试被拖成数分钟且完全在测超时逻辑，而不是被测行为。
    async fn exec_request(
        &mut self,
        channel_id: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data);
        let _ = session.channel_success(channel_id);
        match checksum_command_output(&command, &self.root) {
            Some(output) => {
                let _ = session.data(channel_id, output);
                let _ = session.exit_status_request(channel_id, 0);
            }
            None => {
                let _ = session.data(channel_id, b"command not found\n".to_vec());
                let _ = session.exit_status_request(channel_id, 127);
            }
        }
        let _ = session.close(channel_id);
        Ok(())
    }
}

/// 支持的模拟校验工具：命令前缀与算法。
const CHECKSUM_TOOLS: [(&str, ChecksumKind); 4] = [
    ("sha256sum -- ", ChecksumKind::Sha256),
    ("shasum -a 256 -- ", ChecksumKind::Sha256),
    ("md5sum -- ", ChecksumKind::Md5),
    ("md5 -q -- ", ChecksumKind::Md5),
];

#[derive(Clone, Copy)]
enum ChecksumKind {
    Sha256,
    Md5,
}

/// 解析校验命令并返回与真实工具一致格式的输出行；无法识别时返回 `None`。
fn checksum_command_output(command: &str, root: &Path) -> Option<Vec<u8>> {
    let (kind, rest) = CHECKSUM_TOOLS
        .iter()
        .find_map(|(prefix, kind)| command.strip_prefix(prefix).map(|rest| (*kind, rest)))?;
    // 客户端用单引号包裹路径：`sha256sum -- '<path>' 2>/dev/null || ...`
    let path = rest.split('\'').nth(1)?;
    let bytes = std::fs::read(root.join(path.trim_start_matches('/'))).ok()?;
    let digest = match kind {
        ChecksumKind::Sha256 => Sha256::digest(&bytes).to_vec(),
        ChecksumKind::Md5 => Md5::digest(&bytes).to_vec(),
    };
    Some(format!("{}  {path}\n", hex_encode(&digest)).into_bytes())
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
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
        let local = self.local(&path);
        if let Some(size) = attrs.size {
            // SETSTAT 的 size 语义是截断/扩展文件；续传收尾截断依赖它
            std::fs::OpenOptions::new()
                .write(true)
                .open(&local)
                .map_err(|_| StatusCode::Failure)?
                .set_len(size)
                .map_err(|_| StatusCode::Failure)?;
        }
        if let Some(mtime) = attrs.mtime {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(&local)
                .map_err(|_| StatusCode::Failure)?;
            file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(u64::from(mtime)))
                .map_err(|_| StatusCode::Failure)?;
        }
        Ok(ok(id))
    }
}
