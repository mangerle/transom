//! SFTP 内置编辑器的远端文本读写：整份读取、形态规整（BOM/CRLF/GBK）、
//! 版本指纹冲突比对、写前备份与覆盖回传。
//!
//! 设计原理（Why）：
//! - 读写走独立通道不进传输队列，理由见控制器 `SftpOperation::ReadText` 注释。
//! - 编辑缓冲区持有的并非远端原样字节：读取时剥离 BOM、把 CRLF 统一为 LF、
//!   非 UTF-8 内容按 GBK 解码（只读预览），全部记录在 `RemoteTextMeta` 的形态
//!   标记里，回传时按标记原样还原。若不做往返还原，保存一次就会悄悄改掉文件
//!   的 BOM 与行尾风格，产生满屏假 diff。
//! - 备份（设置项开启时）在覆盖前把远端当前内容写入 `{path}.transom-bak`，
//!   是 SFTP 协议下零 inode 副作用的兜底：写入中断后旧内容仍可从备份恢复。
//!   rename 原子替换会改变 inode、可能破坏正在读取文件的进程，故不采用。
//!   备份属尽力而为：目标不存在或读取失败时跳过，备份文件本身写不进去才中止
//!   覆盖（说明同目录写入必然失败，提前止损）。

use crate::sftp::{SftpClient, TRANSFER_CHUNK, sftp_error, sftp_io_error};
use russh_sftp::client::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use transom_core::{AppError, ErrorCode, RemoteTextMeta};

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
/// 备份读取的尺寸余量：防止 stat 与读取之间文件被替换成更大的内容导致误判。
const BACKUP_SLACK_BYTES: u64 = 64 * 1024;

impl SftpClient {
    /// 读取远端文本文件全文，供内置编辑器打开；返回规整后的文本与读取时刻的
    /// 版本指纹（含形态标记）。
    ///
    /// 设计原理（Why）：
    /// - `max_bytes` 在读取循环内二次设限。进入前虽然已按目录列表的尺寸
    ///   拦过一次，但列表可能过期（文件在列出后变大），循环内设限才真正保证超限
    ///   内容不会被整份吸进内存。
    /// - UTF-8 校验与 GBK 兜底解码、BOM/CRLF 规整都在边界一次完成，UI 拿到的
    ///   已是可编辑的规整文本，形态还原由回传侧按指纹标记负责。
    /// - 指纹字节数取远端 stat 值（stat 缺失时退回原始字节数）：比对对象始终是
    ///   远端 stat，规整会改变缓冲区长度，不能拿缓冲区长度当指纹。
    /// - 代价与局限：无进度回调、无校验和，适合上限内的配置与日志片段；更大的
    ///   文件应当走传输队列或在远端编辑。
    ///
    /// # Errors
    /// 路径不存在、权限不足、会话中断、超过 `max_bytes`，或内容既不是 UTF-8
    /// 也无法按 GBK 解码（疑似二进制）时返回 `ErrorCode::SftpFailed`。
    pub async fn read_text(
        &self,
        path: &str,
        max_bytes: u64,
    ) -> Result<(String, RemoteTextMeta), AppError> {
        let remote = self.session.open(path).await.map_err(sftp_error)?;
        let opened = remote.metadata().await.ok();
        let total = opened.as_ref().and_then(|metadata| metadata.size);
        // mtime 取读取前的 stat：若文件在读取过程中被改写，随后保存前的比对
        // 会因 mtime 不一致命中冲突，恰好覆盖这个竞态窗口。
        let mtime = opened.as_ref().and_then(|metadata| metadata.mtime);
        if let Some(total) = total
            && total > max_bytes
        {
            let _ = remote.close().await;
            return Err(AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("文件 {path} 大小 {total} 字节，超过编辑器上限 {max_bytes} 字节"),
            ));
        }

        let raw_len = read_file_capped(remote, path, max_bytes).await?;
        let raw_size = u64::try_from(raw_len.len()).unwrap_or(u64::MAX);
        let (content, transcoded, bom, crlf) = normalize_read_text(raw_len, path)?;
        let meta = RemoteTextMeta {
            size: total.unwrap_or(raw_size),
            mtime,
            bom,
            crlf,
            transcoded,
        };
        Ok((content, meta))
    }

    /// 将整份文本内容写回远端覆盖，供内置编辑器保存使用；返回远端确认后的
    /// 版本指纹（含形态标记），作为面板下一次比对与还原的基准。
    ///
    /// 设计原理（Why）：
    /// - 必须带 TRUNCATE 打开：SFTP 的写入只覆盖前 N 字节，若编辑后内容比原文件
    ///   短，不截断就会在远端留下旧文件的尾巴，得到一个"新内容 + 残留旧尾巴"的
    ///   脏文件，而这恰好是改配置最常见的操作（删掉几行再保存）。
    /// - `expected` 携带打开时的版本指纹做乐观锁：写前先 stat 比对，远端已被
    ///   他人修改则拒绝覆盖并返回 `ErrorCode::SftpTextConflict`，由面板弹出冲突
    ///   横幅交给用户决策。`force` 为 true 时跳过比对（用户已明确强制覆盖），
    ///   stat-then-write 非原子，比对与写入之间的窗口是该方案的固有限制。
    /// - 回传内容按 `expected` 的形态标记还原 BOM/CRLF 后再落盘，保证
    ///   "打开 → 保存"不改变文件的编码形态。
    /// - 写完后重新 stat 比对字节数：截断与部分写是唯一能被廉价发现的失败形态；
    ///   校验和要把整份内容重读一遍，对几百 KB 的配置编辑不划算。
    /// - 代价与局限：非原子操作，写入过程中断会在远端留下半份文件（开启备份
    ///   设置时可从 `{path}.transom-bak` 恢复）。
    ///
    /// # Errors
    /// 版本指纹不一致返回 `ErrorCode::SftpTextConflict`；备份文件无法写入、
    /// 打开、写入、刷新失败，或写回后远端大小与提交字节数不一致时返回
    /// `ErrorCode::SftpFailed`。
    /// 为 Sudo 提权写入做与普通写回一致的冲突检查与载荷形态还原。
    ///
    /// 提权通道绕过了 SFTP 的写接口，但"是否覆盖他人改动""BOM/CRLF 是否原样
    /// 保留"这两件事不能绕过：前者返回 `SftpTextConflict` 让界面走冲突横幅，
    /// 后者保证提权保存一次不会把文件行尾整体换掉。
    ///
    /// # Errors
    /// 远端 stat 失败返回 `SftpFailed`；版本指纹不一致返回 `SftpTextConflict`。
    pub async fn prepare_sudo_write(
        &self,
        path: &str,
        content: &[u8],
        expected: Option<RemoteTextMeta>,
        force: bool,
    ) -> Result<Vec<u8>, AppError> {
        if let Some(expected) = expected
            && !force
        {
            let metadata = self.session.metadata(path).await.map_err(sftp_error)?;
            if let Some(current) = remote_text_meta_of(metadata.size, metadata.mtime)
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
        Ok(restore_write_payload(content, expected.as_ref()))
    }

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
            let metadata = self.session.metadata(path).await.map_err(sftp_error)?;
            if let Some(current) = remote_text_meta_of(metadata.size, metadata.mtime)
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
        let mut remote = self
            .session
            .open_with_flags(
                path,
                russh_sftp::protocol::OpenFlags::WRITE
                    | russh_sftp::protocol::OpenFlags::CREATE
                    | russh_sftp::protocol::OpenFlags::TRUNCATE,
            )
            .await
            .map_err(sftp_error)?;
        remote.write_all(&payload).await.map_err(sftp_io_error)?;
        remote.flush().await.map_err(sftp_io_error)?;
        let submitted = payload.len() as u64;
        let _ = remote.close().await;

        let metadata = self.session.metadata(path).await.map_err(sftp_error)?;
        let written = metadata.size.ok_or_else(|| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("写回后无法读取远端文件 {path} 的大小，无法确认写入结果"),
            )
        })?;
        if written != submitted {
            return Err(AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("远端确认 {written} 字节，提交内容 {submitted} 字节，写入可能被中断"),
            ));
        }
        let (bom, crlf, transcoded) = expected
            .map(|meta| (meta.bom, meta.crlf, meta.transcoded))
            .unwrap_or((false, false, false));
        Ok(RemoteTextMeta {
            size: written,
            mtime: metadata.mtime,
            bom,
            crlf,
            transcoded,
        })
    }

    /// 覆盖前备份远端当前内容到 `{path}.transom-bak`。
    ///
    /// 尽力而为语义：目标不存在（新文件）、无法取属性或读取失败时记调试日志并
    /// 跳过，不阻塞保存；备份文件本身写不进去时返回错误中止覆盖。
    async fn backup_remote_file(&self, path: &str) -> Result<(), AppError> {
        let backup_path = format!("{path}.transom-bak");
        let Ok(metadata) = self.session.metadata(path).await else {
            log::debug!("跳过远端备份：无法获取文件属性（可能不存在），路径: {path}");
            return Ok(());
        };
        let Some(size) = metadata.size else {
            log::debug!("跳过远端备份：远端未回传文件大小，路径: {path}");
            return Ok(());
        };
        let Ok(file) = self.session.open(path).await else {
            log::debug!("跳过远端备份：打开远端文件失败，路径: {path}");
            return Ok(());
        };
        let cap = size.saturating_add(BACKUP_SLACK_BYTES);
        let Ok(raw) = read_file_capped(file, path, cap).await else {
            log::debug!("跳过远端备份：读取当前内容失败，路径: {path}");
            return Ok(());
        };
        let mut backup = self
            .session
            .open_with_flags(
                &backup_path,
                russh_sftp::protocol::OpenFlags::WRITE
                    | russh_sftp::protocol::OpenFlags::CREATE
                    | russh_sftp::protocol::OpenFlags::TRUNCATE,
            )
            .await
            .map_err(sftp_error)?;
        backup
            .write_all(&raw)
            .await
            .map_err(sftp_io_error)
            .inspect_err(|_| {
                log::warn!(
                    "写入远端备份文件失败，已中止覆盖，路径: {path}，备份路径: {backup_path}"
                );
            })?;
        backup.flush().await.map_err(sftp_io_error)?;
        let _ = backup.close().await;
        log::debug!(
            "已备份远端文件当前内容，路径: {path}，字节数: {}",
            raw.len()
        );
        Ok(())
    }
}

/// 把打开的远端文件读到末尾，总字节量超过 `limit` 时关闭文件并报错。
///
/// 文件的关闭在函数内兜底完成，调用方无需（也不应）再关闭。
async fn read_file_capped(mut file: File, path: &str, limit: u64) -> Result<Vec<u8>, AppError> {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    // stat 缺失时容量从 0 起步，靠 Vec 扩容兜底；已知尺寸时预分配避免反复搬移。
    let mut buffer = Vec::new();
    let mut chunk = vec![0u8; TRANSFER_CHUNK];
    loop {
        let count = match file.read(&mut chunk).await {
            Ok(count) => count,
            Err(error) => {
                let _ = file.close().await;
                return Err(sftp_io_error(error));
            }
        };
        if count == 0 {
            break;
        }
        if buffer.len().saturating_add(count) > limit {
            let _ = file.close().await;
            return Err(AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("文件 {path} 实际大小超过读取上限 {limit} 字节，已放弃读取"),
            ));
        }
        buffer.extend_from_slice(&chunk[..count]);
    }
    let _ = file.close().await;
    Ok(buffer)
}

/// 把远端原始字节规整为可编辑文本：剥离 BOM、统一 CRLF 为 LF、
/// 非 UTF-8 内容按 GBK 兜底解码。
///
/// 返回 `(文本, 是否 GBK 转码, 是否有 BOM, 是否 CRLF)`；形态标记由回传侧
/// `restore_write_payload` 消费，保证保存往返不改变文件形态。
pub(crate) fn normalize_read_text(
    mut raw: Vec<u8>,
    path: &str,
) -> Result<(String, bool, bool, bool), AppError> {
    let bom = raw.starts_with(&UTF8_BOM);
    if bom {
        raw.drain(..UTF8_BOM.len());
    }
    let (text, transcoded) = match String::from_utf8(raw) {
        Ok(text) => (text, false),
        Err(error) => {
            let bytes = error.into_bytes();
            let (decoded, had_errors) = encoding_rs::GBK.decode_without_bom_handling(&bytes);
            if had_errors {
                return Err(AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!(
                        "文件 {path} 不是 UTF-8 文本，且无法按 GBK 解码（可能是二进制内容），内置编辑器无法打开"
                    ),
                ));
            }
            (decoded.into_owned(), true)
        }
    };
    let crlf = text.contains("\r\n");
    let text = if crlf {
        text.replace("\r\n", "\n")
    } else {
        text
    };
    Ok((text, transcoded, bom, crlf))
}

/// 按版本指纹的形态标记把编辑缓冲区内容还原为远端原形态字节。
///
/// 指纹为 `None`（新文件）时原样返回；CRLF 还原只处理不含 CR 的内容，
/// 已含 CR 视为用户保留了原始行尾，不做二次转换。
pub(crate) fn restore_write_payload(content: &[u8], meta: Option<&RemoteTextMeta>) -> Vec<u8> {
    let Some(meta) = meta else {
        return content.to_vec();
    };
    let mut payload = if meta.crlf && !content.contains(&b'\r') {
        let mut restored = Vec::with_capacity(content.len() + 16);
        for &byte in content {
            if byte == b'\n' {
                restored.push(b'\r');
            }
            restored.push(byte);
        }
        restored
    } else {
        content.to_vec()
    };
    if meta.bom {
        let mut with_bom = Vec::with_capacity(payload.len() + UTF8_BOM.len());
        with_bom.extend_from_slice(&UTF8_BOM);
        with_bom.append(&mut payload);
        payload = with_bom;
    }
    payload
}

/// 把远端 stat 结果折算成版本指纹；缺少字节数时返回 `None`，调用方据此跳过
/// 比对而不是误判冲突（无法验证不等于发生了冲突）。只取两个所需字段而非
/// 整个 stat 对象，避免为一次折算引入协议层类型依赖。
fn remote_text_meta_of(size: Option<u64>, mtime: Option<u32>) -> Option<RemoteTextMeta> {
    Some(RemoteTextMeta {
        size: size?,
        mtime,
        bom: false,
        crlf: false,
        transcoded: false,
    })
}

#[cfg(test)]
mod tests {
    use super::{normalize_read_text, restore_write_payload};
    use transom_core::RemoteTextMeta;

    fn meta(bom: bool, crlf: bool, transcoded: bool) -> RemoteTextMeta {
        RemoteTextMeta {
            size: 0,
            mtime: None,
            bom,
            crlf,
            transcoded,
        }
    }

    #[test]
    fn test_normalize_keeps_plain_utf8_untouched() {
        let (text, transcoded, bom, crlf) =
            normalize_read_text(b"hello\nworld\n".to_vec(), "/a").expect("普通 UTF-8 应解码成功");
        assert_eq!(text, "hello\nworld\n");
        assert!(!transcoded && !bom && !crlf);
    }

    #[test]
    fn test_normalize_strips_bom_and_unifies_crlf() {
        let mut raw = vec![0xEF, 0xBB, 0xBF];
        raw.extend_from_slice(b"a\r\nb\r\n");
        let (text, transcoded, bom, crlf) =
            normalize_read_text(raw, "/a").expect("BOM + CRLF 应规整成功");
        assert_eq!(text, "a\nb\n");
        assert!(bom && crlf && !transcoded);
    }

    #[test]
    fn test_normalize_decodes_gbk_and_flags_transcoded() {
        // "中文" 的 GBK 编码字节。
        let gbk_bytes = vec![0xD6, 0xD0, 0xCE, 0xC4];
        let (text, transcoded, bom, crlf) =
            normalize_read_text(gbk_bytes, "/a").expect("GBK 内容应兜底解码成功");
        assert_eq!(text, "中文");
        assert!(transcoded && !bom && !crlf);
    }

    #[test]
    fn test_normalize_rejects_binary() {
        // 无效 GBK 序列（0xFF 单字节不可映射）应判为二进制并拒绝打开。
        let result = normalize_read_text(vec![0xFF, 0xFE, 0x00, 0x01], "/a");
        assert!(result.is_err(), "二进制内容应返回错误而不是解码成乱码");
    }

    #[test]
    fn test_restore_payload_without_meta_is_identity() {
        assert_eq!(restore_write_payload(b"abc", None), b"abc".to_vec());
    }

    #[test]
    fn test_restore_payload_restores_crlf_and_bom() {
        let payload = restore_write_payload(b"a\nb\n", Some(&meta(true, true, false)));
        assert_eq!(payload, {
            let mut expected = vec![0xEF, 0xBB, 0xBF];
            expected.extend_from_slice(b"a\r\nb\r\n");
            expected
        });
    }

    #[test]
    fn test_restore_payload_roundtrip_with_normalize() {
        let original: &[u8] = b"x: 1\r\ny: 2\r\n";
        let (text, transcoded, bom, crlf) =
            normalize_read_text(original.to_vec(), "/a").expect("规整应成功");
        assert!(!transcoded && !bom && crlf);
        let restored = restore_write_payload(text.as_bytes(), Some(&meta(false, crlf, false)));
        assert_eq!(restored, original.to_vec(), "往返还原必须字节一致");
    }

    #[test]
    fn test_restore_payload_keeps_explicit_cr_untouched() {
        // 用户手工输入了 CR（内容已含 \r）：不做二次转换。
        let payload = restore_write_payload(b"a\r\nb", Some(&meta(false, true, false)));
        assert_eq!(payload, b"a\r\nb".to_vec());
    }
}
