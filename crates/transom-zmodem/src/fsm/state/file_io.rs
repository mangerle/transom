//! ZMODEM 会话的本地文件打开、读写与关闭。

use super::Fsm;
use std::path::PathBuf;

use crate::files::{self, DownloadWriter, UploadFile};
use crate::sender::BLOCK_SIZE;

impl Fsm {
    /// 打开下载文件；失败按 lrzsz 语义请求对端跳过当前文件。
    pub(super) fn open_download_file(&mut self, name: String) {
        match DownloadWriter::create(&self.config.download_dir, &name) {
            Ok(writer) => {
                log::info!("ZMODEM 下载开始写入文件: {}", writer.path().display());
                self.download_file = Some(writer);
            }
            Err(error) => {
                log::warn!("ZMODEM 下载文件创建失败，将请求对端跳过: {name}，原因: {error}");
                let actions = self.receiver.notify_create_failed();
                self.run_recv(actions);
            }
        }
    }

    /// 追加写盘；写失败即中止会话（磁盘满或句柄失效都无继续价值）。
    pub(super) fn append_download_data(&mut self, data: Vec<u8>) {
        let Some(writer) = self.download_file.as_mut() else {
            return;
        };
        if let Err(error) = writer.write(&data) {
            log::error!("ZMODEM 下载数据写盘失败，传输中止: {error}");
            let actions = self.receiver.cancel();
            self.run_recv(actions);
        }
    }

    /// 文件收尾：冲刷缓冲并强制同步，再释放句柄。
    ///
    /// 同步失败只告警不改变协议结局：数据已全部收到，磁盘层面的失败
    /// 不应把一次成功的传输改判为失败，但必须留下排查线索。
    pub(super) fn close_download_file(&mut self) {
        if let Some(mut writer) = self.download_file.take()
            && let Err(error) = writer.finish()
        {
            log::warn!(
                "ZMODEM 下载文件收尾同步失败: {}，原因: {error}",
                writer.path().display()
            );
        }
    }

    /// 打开待上传文件；失败跳过并推进到下一个。
    pub(super) fn open_upload_file(&mut self, path: PathBuf) {
        match files::open_upload_file(&path) {
            Ok(opened) => {
                let UploadFile {
                    file,
                    size,
                    mtime,
                    name,
                } = opened;
                self.upload_file = Some(file);
                self.sender.set_current_file(name);
                let actions = self.sender.notify_file_ready(size, mtime);
                self.run_send(actions);
            }
            Err(error) => {
                log::warn!(
                    "ZMODEM 上传文件打开失败，跳过该文件: {}，原因: {error}",
                    path.display()
                );
                let actions = self.sender.notify_open_failed();
                self.run_send(actions);
            }
        }
    }

    /// 从上传文件按状态机给定偏移读取下一块并推送。
    pub(super) fn read_upload_block(&mut self, offset: u64) {
        let Some(file) = self.upload_file.as_mut() else {
            return;
        };
        let mut chunk = vec![0u8; BLOCK_SIZE];
        match files::read_block(file, offset, &mut chunk) {
            Ok(count) => {
                chunk.truncate(count);
                let actions = self.sender.push_data(&chunk, count < BLOCK_SIZE);
                self.run_send(actions);
            }
            Err(error) => {
                log::error!("ZMODEM 上传文件读写失败，传输中止，偏移: {offset}，原因: {error}");
                let actions = self.sender.cancel();
                self.run_send(actions);
            }
        }
    }
}
