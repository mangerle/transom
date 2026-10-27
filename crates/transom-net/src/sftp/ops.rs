//! SFTP 目录列表、元数据与常规文件系统操作。

use super::client::{SftpClient, list_limit_reached, sftp_error};
use transom_core::{
    AppError, ErrorCode, RemoteFile, RemoteMetadata, is_safe_file_name,
};

impl SftpClient {
    pub async fn list(&self, path: &str) -> Result<(String, Vec<RemoteFile>), AppError> {
        let canonical = self.session.canonicalize(path).await.map_err(sftp_error)?;
        let mut entries = Vec::new();
        let mut rejected = 0usize;
        for entry in self
            .session
            .read_dir(&canonical)
            .await
            .map_err(sftp_error)?
        {
            let name = entry.file_name();
            // 超限即中止：不把截断的目录视图交给界面（会误导删除/同步
            // 这类破坏性操作），也不让异常目录吃光客户端内存。
            if list_limit_reached(entries.len()) {
                return Err(AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!(
                        "目录条目超过上限，已中止读取，请使用过滤或进入子目录，目录: {canonical}"
                    ),
                ));
            }
            // 服务端控制 filename，绝不能把它当成可信的单段路径：非法名字
            // 一旦进入 UI，下载时 join 到本地目录就会越界写文件。
            if !is_safe_file_name(&name) {
                rejected = rejected.saturating_add(1);
                continue;
            }
            let metadata = entry.metadata();
            entries.push(RemoteFile {
                name,
                path: entry.path(),
                is_dir: metadata.is_dir(),
                is_symlink: metadata.is_symlink(),
                size: metadata.size.unwrap_or(0),
                modified: metadata.mtime,
                permissions: metadata.permissions,
            });
        }
        if rejected > 0 {
            // 聚合为一条告警：恶意服务端可以返回大量非法名字，逐条记录会刷爆日志。
            log::warn!("SFTP 目录列表包含 {rejected} 个非法文件名，已跳过，目录: {canonical}");
        }
        entries.sort_by(|a, b| {
            b.is_dir.cmp(&a.is_dir).then_with(|| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            })
        });
        Ok((canonical, entries))
    }

    /// 读取远端路径的详细属性（lstat 语义，不跟随符号链接）。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：属性查看弹窗需要 uid/gid、访问时间与符号链接目标等完整元数据，
    ///   目录列表条目仅为展示优化的最小字段，故通过独立请求即时获取最新属性；
    /// - 核心优势：采用 lstat 语义可准确识别符号链接本体；链接目标读取失败时
    ///   降级为 `None` 而不中断整体属性展示（错误局部消化）；
    /// - 代价与局限：每次查看属性都会产生额外网络往返，适用于低频交互场景。
    ///
    /// # Errors
    /// 远端路径不存在、权限不足或会话中断时返回 `ErrorCode::SftpFailed`。
    pub async fn stat(&self, path: &str) -> Result<RemoteMetadata, AppError> {
        let metadata = self
            .session
            .symlink_metadata(path)
            .await
            .map_err(sftp_error)?;
        let is_symlink = metadata.is_symlink();
        let link_target = if is_symlink {
            self.session.read_link(path).await.ok()
        } else {
            None
        };
        Ok(RemoteMetadata {
            path: path.to_string(),
            is_dir: metadata.is_dir(),
            is_symlink,
            size: metadata.size.unwrap_or(0),
            uid: metadata.uid,
            user: metadata.user,
            gid: metadata.gid,
            group: metadata.group,
            permissions: metadata.permissions,
            atime: metadata.atime,
            mtime: metadata.mtime,
            link_target,
        })
    }

    pub async fn create_dir(&self, path: &str) -> Result<(), AppError> {
        self.session.create_dir(path).await.map_err(sftp_error)
    }

    /// 递归创建远端目录，已存在的目录视为成功。
    ///
    /// 设计原理（Why）：SFTP 只有单层 MKDIR，没有 `mkdir -p`；目录传输需要
    /// 逐级创建。逐段调用并校验失败后目录是否真实存在，可避免把“已存在”
    /// 与权限不足、磁盘故障等真实错误混为一谈。
    pub(crate) async fn create_dir_all(&self, path: &str) -> Result<(), AppError> {
        if path.is_empty() || path == "/" || path == "." {
            return Ok(());
        }
        let absolute = path.starts_with('/');
        let mut current = String::new();
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            if absolute || !current.is_empty() {
                current.push('/');
            }
            current.push_str(segment);
            self.ensure_dir(&current).await?;
        }
        Ok(())
    }

    /// 确保单个远端目录存在；创建失败时通过属性确认是否已存在同名目录。
    pub(crate) async fn ensure_dir(&self, path: &str) -> Result<(), AppError> {
        if let Err(error) = self.session.create_dir(path).await {
            let exists_as_dir = self
                .session
                .metadata(path)
                .await
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false);
            if !exists_as_dir {
                return Err(AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!("创建远端目录失败 {path}: {error}"),
                ));
            }
        }
        Ok(())
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        self.session.rename(from, to).await.map_err(sftp_error)
    }

    /// 创建远端软链接：`target` 为被指向的既有路径，`link_path` 为新建链接自身路径。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：`SSH_FXP_SYMLINK` 报文内两个路径参数的先后顺序存在历史分歧——
    ///   OpenSSH 服务端将首个字符串解释为 targetpath（被指向目标）、次个为 linkpath
    ///   （新建链接），与协议草案的字段声明顺序恰好相反；主流服务端均遵循 OpenSSH 行为；
    /// - 核心优势：在客户端封装层统一消化该分歧，参数语义以本方法签名为准，
    ///   上层无需感知报文细节，杜绝链接方向被静默颠倒；
    /// - 代价与局限：面向严格遵循协议草案的少数非 OpenSSH 服务端时链接方向会相反，
    ///   当前以事实标准（OpenSSH）为准；russh-sftp 客户端形参命名沿用草案顺序，
    ///   因此这里对调传入，实际报文顺序为 (target, link_path)。
    ///
    /// # Errors
    /// 链接目标不存在、链接路径已存在或权限不足时返回 `ErrorCode::SftpFailed`。
    pub async fn symlink(&self, target: &str, link_path: &str) -> Result<(), AppError> {
        self.session
            .symlink(target, link_path)
            .await
            .map_err(sftp_error)
    }

    /// 删除远端文件或目录。
    ///
    /// SFTP 的 RMDIR 只能删除空目录，目录分支因此改为先收集并删除全部后代，
    /// 再自底向上删除目录本身，语义对齐本地文件管理器的“删除目录”。
    pub async fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        if is_dir {
            self.delete_dir_recursive(path).await
        } else {
            self.session.remove_file(path).await.map_err(sftp_error)
        }
    }

    /// 修改远程文件或目录的 POSIX 权限位（低 9 位生效，保留原有文件类型高位）
    pub async fn chmod(&self, path: &str, permissions: u32) -> Result<(), AppError> {
        let mut meta = self
            .session
            .metadata(path)
            .await
            .unwrap_or_else(|_| russh_sftp::protocol::FileAttributes::empty());
        let high_bits = meta.permissions.unwrap_or(0) & !0o777;
        meta.permissions = Some(high_bits | (permissions & 0o777));
        self.session
            .set_metadata(path, meta)
            .await
            .map_err(sftp_error)
    }

    /// 仅设置远端文件的修改时间，不触碰权限与其他属性。
    ///
    /// 目录同步依赖两端 mtime 判断新旧；若上传后不保留源文件时间，
    /// 下一次比较会把刚上传的文件误判为对端更新，从而反复回传。
    ///
    /// # Errors
    /// 远端路径不存在、权限不足或服务端不支持 SETSTAT 时返回
    /// `ErrorCode::SftpFailed`；调用方通常以降级方式处理该错误。
    pub async fn set_mtime(&self, path: &str, mtime: u32) -> Result<(), AppError> {
        let mut attributes = russh_sftp::protocol::FileAttributes::empty();
        attributes.mtime = Some(mtime);
        self.session
            .set_metadata(path, attributes)
            .await
            .map_err(sftp_error)
    }
}
