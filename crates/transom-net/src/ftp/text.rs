//! FTP 文件预览、文本读写与内容级远端操作。

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use transom_core::{AppError, ErrorCode, PreviewMode, RemoteTextMeta};

use super::FtpClient;
use super::transfer_io_error;
use crate::sftp::text::{normalize_read_text, remote_text_meta_of, restore_write_payload};
use crate::sftp::{BACKUP_SLACK_BYTES, TRANSFER_CHUNK};

impl FtpClient {
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
    pub(super) async fn size_optional(&self, path: &str) -> Result<Option<u64>, AppError> {
        self.with_stream(async |stream| Ok(stream.size(path).await.ok()))
            .await
    }

    /// 查询远端条目的修改时间（Unix 秒）。
    pub(super) async fn mtime_optional(&self, path: &str) -> Result<Option<u32>, AppError> {
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
