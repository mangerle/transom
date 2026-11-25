//! FTP 目录浏览、元数据查询与文件系统操作。

use suppaftp::list::ListParser;
use transom_core::{AppError, ErrorCode, RemoteFile, RemoteMetadata};

use super::FtpClient;
use super::listing::{convert_entry, mode_from_mlsx_facts, system_time_to_unix};
use crate::file_order::sort_remote_files;
use crate::sftp::MAX_LIST_ENTRIES;

impl FtpClient {
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
            sort_remote_files(&mut entries);
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
}
