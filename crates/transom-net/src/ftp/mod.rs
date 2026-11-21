//! FTP/FTPS 客户端封装。
//!
//! 设计原理（Why）：
//! - 实现初衷：SFTP 面板的数据源抽象已经只依赖 `SftpOperation` 契约与
//!   `ConnectionId`，引入 FTP 只需在 `transom-net` 提供一个与 `SftpClient`
//!   方法集对齐的具体客户端，面板与协议契约层完全不用动；
//! - 核心优势：控制连接断线自愈（FTP 服务器普遍对空闲控制连接设限，几十秒
//!   无操作即断开）——操作失败且错误属于连接级故障时把流整体废弃，下一次
//!   操作自动重连重登录，调用方无感；传输类操作不在此列，它们由执行器为
//!   每个任务单独开连接（见 [`FtpClient::open_transfer_client`]），取消即
//!   整条丢弃，不存在"半截传输污染控制通道"的问题；
//! - 代价与局限：FTP 是弱语义协议——没有原子 rename 语义、没有统一权限
//!   模型、目录列表格式依赖服务端实现，能力面天然小于 SFTP；软链接目标、
//!   远端归档、远端检索与提权写入没有协议级等价物，由上层按"不支持"拒绝。
//!
//! 密码经钥匙串取出后存放在 [`zeroize::Zeroizing`] 中，丢弃时擦除内存。

mod listing;
mod stream;
mod transfer;

use std::sync::Arc;

use suppaftp::list::ListParser;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use transom_core::{
    AppError, ErrorCode, FtpConnection, PreviewMode, RemoteFile, RemoteMetadata, RemoteTextMeta,
};
use zeroize::Zeroizing;

use crate::sftp::text::{normalize_read_text, remote_text_meta_of, restore_write_payload};
use crate::sftp::{BACKUP_SLACK_BYTES, MAX_LIST_ENTRIES, TRANSFER_CHUNK};
use listing::{convert_entry, mode_from_mlsx_facts};
pub(crate) use stream::FtpStream;

/// 单个 FTP 会话的客户端；克隆共享同一条控制连接。
///
/// 控制连接包在 tokio 互斥锁里：FTP 控制通道一次只能跑一条命令，锁把并发
/// 调用串行化，与协议约束一致；锁内不含长时间数据传输（传输走独立连接），
/// 交互操作不会被吞吐任务长期占住锁。
#[derive(Clone)]
pub struct FtpClient {
    session: Arc<Mutex<FtpSession>>,
    config: FtpConnection,
    password: Zeroizing<String>,
}

/// 控制连接的可恢复状态：`None` 表示需要（重新）建立连接。
struct FtpSession {
    stream: Option<FtpStream>,
}

impl FtpClient {
    /// 建立并登录一个 FTP 会话。
    ///
    /// # Errors
    /// 拨号超时、TLS 握手失败或认证被拒绝时分别返回
    /// `ErrorCode::Timeout`/`ErrorCode::ConnectionFailed`/`ErrorCode::AuthFailed`。
    pub async fn open(config: &FtpConnection, password: Option<&str>) -> Result<Self, AppError> {
        let client = Self {
            session: Arc::new(Mutex::new(FtpSession { stream: None })),
            config: config.clone(),
            password: Zeroizing::new(password.unwrap_or_default().to_string()),
        };
        {
            let mut session = client.session.lock().await;
            client.reconnect_inner(&mut session).await?;
        }
        Ok(client)
    }

    /// 为传输任务建立一条独立的 FTP 连接。
    ///
    /// 设计原理（Why）：与 SFTP 侧预留第二传输通道同理——FTP 的控制通道一次
    /// 只能服务一个数据连接，且任务取消（tokio abort）要求"整条连接随之废弃"
    /// 才有干净的取消语义。每个传输任务独立拨号登录，取消时连接整体关闭，
    /// 交互连接不受任何影响；代价是每个任务多一次握手，对以吞吐为主的传输
    /// 场景可忽略。
    ///
    /// # Errors
    /// 同 [`FtpClient::open`]。
    pub async fn open_transfer_client(&self) -> Result<Self, AppError> {
        FtpClient::open(&self.config, Some(&self.password)).await
    }

    /// 尽力关闭控制连接；用于会话停机路径。
    pub async fn close(&self) {
        let mut session = self.session.lock().await;
        if let Some(stream) = session.stream.as_mut() {
            stream.quit().await;
        }
        session.stream = None;
    }

    /// 在健康连接上执行一次控制操作；连接级故障废弃当前流，等待下次重建。
    ///
    /// 生命周期说明：操作用 [`AsyncFnOnce`] 承接——异步闭包的 future 可以
    /// 借用其入参（`&mut FtpStream`），不必再做类型擦除装箱。
    async fn with_stream<T>(
        &self,
        operation: impl AsyncFnOnce(&mut FtpStream) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut session = self.session.lock().await;
        if session.stream.is_none() {
            self.reconnect_inner(&mut session).await?;
        }
        let Some(stream) = session.stream.as_mut() else {
            return Err(AppError::with_detail(
                ErrorCode::ConnectionFailed,
                "FTP 连接未能建立",
            ));
        };
        match operation(stream).await {
            Ok(value) => Ok(value),
            Err(error) => {
                // 连接级故障（拨号断开、响应错乱、控制超时）会让行协议错位，
                // 整条废弃最安全；语义性拒绝（550/502 等）是干净往返，连接可继续用。
                if matches!(error.code, ErrorCode::ConnectionFailed | ErrorCode::Timeout) {
                    session.stream = None;
                }
                Err(error)
            }
        }
    }

    /// 重建控制连接并完成登录初始化。
    async fn reconnect_inner(&self, session: &mut FtpSession) -> Result<(), AppError> {
        let mut stream = FtpStream::connect(&self.config).await?;
        stream
            .login_and_setup(&self.config.username, &self.password, self.config.passive)
            .await?;
        session.stream = Some(stream);
        Ok(())
    }

    /// 传输执行器专用：直接锁定会话状态，传输体在锁内顺序执行。
    ///
    /// 仅限独立传输连接使用；交互操作必须走 [`FtpClient::with_stream`]，
    /// 否则长传输会占住共享锁阻塞交互。
    async fn lock_session(&self) -> tokio::sync::MutexGuard<'_, FtpSession> {
        self.session.lock().await
    }

    /// 会话未连接时先建立连接；传输执行器在拿到会话锁后调用。
    async fn ensure_connected(&self, session: &mut FtpSession) -> Result<(), AppError> {
        if session.stream.is_none() {
            self.reconnect_inner(session).await?;
        }
        Ok(())
    }

    /// 列出目录内容，返回目录路径与条目列表。
    ///
    /// 优先 MLSD（机器可读、字段齐全），服务端不支持时退化 POSIX `LIST`；
    /// 与 SFTP 列表同口径：过滤不可信文件名、条目数设上限、目录排前。
    ///
    /// # Errors
    /// 目录不存在、无权限或连接故障时返回 `ErrorCode::FtpFailed` 等。
    pub async fn list(&self, path: &str) -> Result<(String, Vec<RemoteFile>), AppError> {
        self.with_stream(async |stream| {
            let (lines, with_mode) = match stream.mlsd(Some(path)).await {
                Ok(lines) => (lines, true),
                // 500/502 表示服务器不支持 MLSD，退化人类可读列表；其他错误原样返回。
                Err(error) if error.code == ErrorCode::NotSupported => {
                    (stream.list(Some(path)).await?, false)
                }
                Err(error) => return Err(error),
            };
            let mut entries = Vec::with_capacity(lines.len().min(1024));
            let mut rejected = 0usize;
            for line in lines {
                if entries.len() >= MAX_LIST_ENTRIES {
                    return Err(AppError::with_detail(
                        ErrorCode::FtpFailed,
                        format!(
                            "目录条目超过上限，已中止读取，请使用过滤或进入子目录，目录: {path}"
                        ),
                    ));
                }
                let permissions = with_mode.then(|| mode_from_mlsx_facts(&line)).flatten();
                match convert_entry(&line, path, permissions) {
                    Some(entry) => entries.push(entry),
                    // 不可解析行（如 "total 24"）静默跳过，非法文件名聚合计数。
                    None => rejected += 1,
                }
            }
            if rejected > 0 {
                log::warn!(
                    "FTP 目录列表包含 {rejected} 条不可解析或非法的内容，已跳过，目录: {path}"
                );
            }
            entries.sort_by(|a, b| {
                b.is_dir.cmp(&a.is_dir).then_with(|| {
                    a.name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase())
                })
            });
            Ok((path.to_string(), entries))
        })
        .await
    }

    /// 读取远端条目的属性快照。
    ///
    /// 优先 `MLST`（单次往返带尺寸与修改时间），退化 `SIZE`+`MDTM`；
    /// FTP 无法提供 uid/gid/属主等信息，全部按 `None` 降级展示。
    ///
    /// # Errors
    /// 路径不存在、无权限或连接故障时返回 `ErrorCode::FtpFailed`。
    pub async fn stat(&self, path: &str) -> Result<RemoteMetadata, AppError> {
        self.with_stream(async |stream| {
            if let Ok(line) = stream.mlst(path).await
                && let Some(file) = ListParser::parse_mlsd(&line).ok()
            {
                return Ok(RemoteMetadata {
                    path: path.to_string(),
                    is_dir: file.is_directory(),
                    is_symlink: file.is_symlink(),
                    size: file.size() as u64,
                    uid: None,
                    user: None,
                    gid: None,
                    group: None,
                    permissions: mode_from_mlsx_facts(&line),
                    atime: None,
                    mtime: system_time_to_unix(file.modified()),
                    link_target: file
                        .symlink()
                        .and_then(|target| target.to_str().map(str::to_owned)),
                });
            }
            // MLST 不可用时的兜底：SIZE 只对文件有意义；CWD 成功即证明存在
            // 且为目录，两者都失败说明路径不存在或不可访问，必须显式报错，
            // 不能静默折叠成"空目录"——能力探测（如目录确保创建）依赖
            // "路径不存在"与"确实是目录"的区分。
            if let Ok(size) = stream.size(path).await {
                let mtime = stream.mdtm(path).await.ok().flatten();
                return Ok(RemoteMetadata {
                    path: path.to_string(),
                    is_dir: false,
                    is_symlink: false,
                    size,
                    uid: None,
                    user: None,
                    gid: None,
                    group: None,
                    permissions: None,
                    atime: None,
                    mtime,
                    link_target: None,
                });
            }
            if stream.cwd(path).await.is_ok() {
                return Ok(RemoteMetadata {
                    path: path.to_string(),
                    is_dir: true,
                    is_symlink: false,
                    size: 0,
                    uid: None,
                    user: None,
                    gid: None,
                    group: None,
                    permissions: None,
                    atime: None,
                    mtime: None,
                    link_target: None,
                });
            }
            Err(AppError::with_detail(
                ErrorCode::FtpFailed,
                format!("远端路径不存在或不可访问: {path}"),
            ))
        })
        .await
    }

    /// 创建远端目录。
    pub async fn create_dir(&self, path: &str) -> Result<(), AppError> {
        self.with_stream(async |stream| stream.mkdir(path).await)
            .await
    }

    /// 重命名/移动远端条目。
    pub async fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        self.with_stream(async |stream| stream.rename(from, to).await)
            .await
    }

    /// 删除远端条目；目录按递归语义删除。
    ///
    /// # Errors
    /// 文件删除失败或目录子树中任一条目删除失败时返回 `ErrorCode::FtpFailed`；
    /// 已完成的部分删除不会回滚。
    pub async fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        if is_dir {
            self.delete_dir_recursive(path).await
        } else {
            self.with_stream(async |stream| stream.rm(path).await).await
        }
    }

    /// 递归删除远端目录：显式栈收集整棵子树后自底向上删除。
    ///
    /// 设计原理（Why）与 SFTP 侧一致：RMD 只能删空目录、显式栈防爆栈；
    /// 差异点是 FTP 拿不到可靠的符号链接标记，"是否目录"以 MLSD 优先、
    /// 退化 LIST 形态按解析结果判定。
    ///
    /// # Errors
    /// 根目录非法、读取目录、删除文件或删除目录失败时返回 `ErrorCode::FtpFailed`。
    pub async fn delete_dir_recursive(&self, root: &str) -> Result<(), AppError> {
        if root.is_empty() || root == "/" {
            return Err(AppError::with_detail(
                ErrorCode::FtpFailed,
                "不允许递归删除远端根目录",
            ));
        }
        let mut directories = Vec::new();
        let mut files = Vec::new();
        // 显式栈遍历：列表条目已过安全文件名校验，路径拼接不会越界。
        let mut pending = vec![root.to_string()];
        while let Some(directory) = pending.pop() {
            directories.push(directory.clone());
            let (_, entries) = self.list(&directory).await?;
            for entry in entries {
                if entry.is_dir {
                    pending.push(entry.path);
                } else {
                    files.push(entry.path);
                }
            }
        }
        for file in &files {
            self.with_stream(async |stream| stream.rm(file).await)
                .await?;
        }
        for directory in directories.iter().rev() {
            self.with_stream(async |stream| stream.rmdir(directory).await)
                .await?;
        }
        Ok(())
    }

    /// 设置远端权限（`SITE CHMOD`）；服务器不支持时按"不支持"报错。
    pub async fn chmod(&self, path: &str, permissions: u32) -> Result<(), AppError> {
        let command = format!("CHMOD {:o} {path}", permissions & 0o7777);
        self.with_stream(async |stream| stream.site(&command).await)
            .await
    }

    /// 读取远端文件内容用于预览；`Full` 模式超过 `max_bytes` 直接拒绝。
    ///
    /// 返回 `(内容, 是否被截断)`；截断读取只取上限内的前缀，调用方据此展示
    /// "仅显示前 N 字节"。
    ///
    /// # Errors
    /// 路径不存在、权限不足、超限或连接故障时返回 `ErrorCode::FtpFailed`。
    pub async fn read_preview(
        &self,
        path: &str,
        mode: PreviewMode,
        max_bytes: u64,
    ) -> Result<(Vec<u8>, bool), AppError> {
        let total = self.size_optional(path).await?;
        if mode == PreviewMode::Full
            && let Some(total) = total
            && total > max_bytes
        {
            return Err(AppError::with_detail(
                ErrorCode::FtpFailed,
                format!("文件 {path} 大小 {total} 字节，超过预览上限 {max_bytes} 字节"),
            ));
        }
        let mut truncated = total.is_some_and(|total| total > max_bytes);
        let mut content = self.read_capped(path, max_bytes.saturating_add(1)).await?;
        if !truncated && content.len() as u64 > max_bytes {
            content.truncate(max_bytes as usize);
            truncated = true;
        }
        Ok((content, truncated))
    }

    /// 读取远端文本文件全文，供内置编辑器打开；返回规整后的文本与读取时刻的
    /// 版本指纹（含 BOM/CRLF/GBK 形态标记），规整逻辑与 SFTP 侧完全共用。
    ///
    /// # Errors
    /// 路径不存在、权限不足、超过 `max_bytes`、内容疑似二进制或连接故障时
    /// 返回 `ErrorCode::FtpFailed`。
    pub async fn read_text(
        &self,
        path: &str,
        max_bytes: u64,
    ) -> Result<(String, RemoteTextMeta), AppError> {
        let total = self.size_optional(path).await?;
        if let Some(total) = total
            && total > max_bytes
        {
            return Err(AppError::with_detail(
                ErrorCode::FtpFailed,
                format!("文件 {path} 大小 {total} 字节，超过编辑器上限 {max_bytes} 字节"),
            ));
        }
        // mtime 取读取前的值：文件在读取过程中被改写时，保存前的比对会因
        // mtime 不一致命中冲突，恰好覆盖这个竞态窗口。
        let mtime = self.mtime_optional(path).await?;
        let raw = self.read_capped(path, max_bytes).await?;
        let (text, transcoded, bom, crlf) = normalize_read_text(raw, path)?;
        let meta = RemoteTextMeta {
            size: total.unwrap_or(text.len() as u64),
            mtime,
            bom,
            crlf,
            transcoded,
        };
        Ok((text, meta))
    }

    /// 覆盖写回远端文本文件；带乐观锁比对与可选的远端备份。
    ///
    /// # Errors
    /// 版本指纹冲突（`ErrorCode::SftpTextConflict`）、远端写回尺寸校验不一致
    /// 或连接故障时返回错误；备份读取失败按尽力而为语义跳过。
    pub async fn write_text(
        &self,
        path: &str,
        content: &[u8],
        expected: Option<RemoteTextMeta>,
        force: bool,
        backup: bool,
    ) -> Result<RemoteTextMeta, AppError> {
        if let Some(expected) = expected
            && !force
        {
            let (size, mtime) = self.size_and_mtime(path).await?;
            if let Some(current) = remote_text_meta_of(size, mtime)
                && expected.conflicts_with(&current)
            {
                return Err(AppError::with_detail(
                    ErrorCode::SftpTextConflict,
                    format!(
                        "文件 {path} 自打开后已被修改：当前 {} 字节（mtime {:?}），打开时 {} 字节（mtime {:?}）",
                        current.size, current.mtime, expected.size, expected.mtime
                    ),
                ));
            }
        }
        let payload = restore_write_payload(content, expected.as_ref());
        if backup {
            self.backup_remote_file(path).await?;
        }
        self.upload_bytes(path, &payload).await?;
        let (size, mtime) = self.size_and_mtime(path).await?;
        let written = size.ok_or_else(|| {
            AppError::with_detail(
                ErrorCode::FtpFailed,
                format!("写回后无法读取远端文件 {path} 的大小，无法确认写入结果"),
            )
        })?;
        if written != payload.len() as u64 {
            return Err(AppError::with_detail(
                ErrorCode::FtpFailed,
                format!(
                    "远端确认 {written} 字节，提交内容 {} 字节，写入可能被中断",
                    payload.len()
                ),
            ));
        }
        let (bom, crlf, transcoded) = expected
            .map(|meta| (meta.bom, meta.crlf, meta.transcoded))
            .unwrap_or((false, false, false));
        Ok(RemoteTextMeta {
            size: written,
            mtime,
            bom,
            crlf,
            transcoded,
        })
    }

    /// 把远端当前内容备份为 `{path}.transom-bak`（尽力而为，与 SFTP 语义一致）。
    async fn backup_remote_file(&self, path: &str) -> Result<(), AppError> {
        let backup_path = format!("{path}.transom-bak");
        let total = match self.size_optional(path).await {
            Ok(Some(total)) if total > 0 => total.saturating_add(BACKUP_SLACK_BYTES),
            Ok(_) => return Ok(()),
            Err(_) => return Ok(()),
        };
        let Ok(raw) = self.read_capped(path, total).await else {
            log::debug!("跳过远端备份：读取当前内容失败，路径: {path}");
            return Ok(());
        };
        if let Err(error) = self.upload_bytes(&backup_path, &raw).await {
            return Err(AppError::with_detail(
                ErrorCode::FtpFailed,
                format!(
                    "写入远端备份文件失败 {backup_path}: {}",
                    error.detail.unwrap_or_default()
                ),
            ));
        }
        Ok(())
    }

    /// 查询远端文件的字节大小；服务器不回传（如目录）时为 `None`。
    async fn size_optional(&self, path: &str) -> Result<Option<u64>, AppError> {
        self.with_stream(async |stream| Ok(stream.size(path).await.ok()))
            .await
    }

    /// 查询远端条目的修改时间（Unix 秒）。
    async fn mtime_optional(&self, path: &str) -> Result<Option<u32>, AppError> {
        self.with_stream(async |stream| stream.mdtm(path).await)
            .await
    }

    /// 一次拿尺寸与修改时间（写回校验用）。
    async fn size_and_mtime(&self, path: &str) -> Result<(Option<u64>, Option<u32>), AppError> {
        self.with_stream(async |stream| {
            let size = stream.size(path).await.ok();
            let mtime = stream.mdtm(path).await.ok().flatten();
            Ok((size, mtime))
        })
        .await
    }

    /// 通过独立传输连接读取远端文件的前 `limit` 字节。
    async fn read_capped(&self, path: &str, limit: u64) -> Result<Vec<u8>, AppError> {
        let transfer = self.open_transfer_client().await?;
        transfer
            .with_stream(async |stream| {
                let mut data = stream.download_stream(path, 0).await?;
                let mut content =
                    Vec::with_capacity(usize::try_from(limit.min(1 << 20)).unwrap_or(1024));
                let mut buffer = vec![0u8; TRANSFER_CHUNK];
                loop {
                    let count = data.read(&mut buffer).await.map_err(transfer_io_error)?;
                    if count == 0 {
                        break;
                    }
                    let remaining = limit.saturating_sub(content.len() as u64);
                    let take = (count as u64).min(remaining) as usize;
                    content.extend_from_slice(&buffer[..take]);
                    if take < count || content.len() as u64 >= limit {
                        break;
                    }
                }
                data.finish().await?;
                Ok(content)
            })
            .await
    }

    /// 通过独立传输连接把字节块整体覆盖写入远端文件（STOR）。
    async fn upload_bytes(&self, path: &str, payload: &[u8]) -> Result<(), AppError> {
        let transfer = self.open_transfer_client().await?;
        transfer
            .with_stream(async |stream| {
                let mut data = stream.upload_stream(path, 0).await?;
                data.write_all(payload).await.map_err(transfer_io_error)?;
                data.finish().await?;
                Ok(())
            })
            .await
    }
}

/// 传输数据流的底层 IO 故障统一转为领域错误。
fn transfer_io_error(error: std::io::Error) -> AppError {
    AppError::with_detail(ErrorCode::FtpFailed, format!("FTP 数据传输失败: {error}"))
}

/// 把 suppaftp 解析出的服务端时间收敛为 Unix 秒；早于纪元按 0 处理。
fn system_time_to_unix(time: std::time::SystemTime) -> Option<u32> {
    let seconds = time
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    u32::try_from(seconds).ok()
}
