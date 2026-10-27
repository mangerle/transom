//! 已知主机指纹（TOFU）的 JSON 存储：内存态权威，落盘异步。
//!
//! 设计原理（Why）：
//! - `record`/`remove` 在 SSH 握手回调（tokio 工作线程）里被调用，同步
//!   写盘会把小文件 IO 带进异步路径，磁盘慢或家目录在 NFS 上时拖住握手；
//!   因此内存态在调用瞬间生效，落盘交给常驻写盘线程异步完成；
//! - 待写槽是覆盖式单槽（只保留最新全量快照）：记录频率极低（TOFU 首连
//!   与手动删除），中间态没有保留价值，全量快照天然幂等；
//! - 写盘失败只告警不冒泡：调用方（握手）早已返回，且失败模式是"下次
//!   连接重新提示确认指纹"——重新确认是安全方向，不会静默信任；
//! - Drop 时停机并 join 写盘线程：退出路径上最后一个快照不丢。
//!
//! 代价与局限：`record` 返回 Ok 不再代表"已落盘"，只代表"已进入内存与
//! 待写槽"；进程在写盘完成前被强杀会丢最后若干条记录（下次重连重新确认）。

use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use transom_core::{
    AppError, ErrorCode, HostKeyInfo, HostKeyRecord, HostKeyStatus, KnownHostsStore, normalize_host,
};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct KnownHostsFile {
    #[serde(default = "schema_version")]
    version: u32,
    #[serde(default)]
    hosts: Vec<HostKeyRecord>,
}

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Default for KnownHostsFile {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            hosts: Vec::new(),
        }
    }
}

/// 待写槽与工作状态（覆盖式）：只保留最新全量快照。
struct PersistSlot {
    pending: Option<Vec<HostKeyRecord>>,
    /// 写盘线程正在写：`flush` 需要在此状态下继续等待。
    writing: bool,
    /// 停机信号；收到后把剩余快照写完即退出。
    shutdown: bool,
}

/// 写盘线程与调用方的共享状态。
struct PersistWorker {
    slot: Mutex<PersistSlot>,
    wake: Condvar,
}

impl PersistWorker {
    fn new() -> Self {
        Self {
            slot: Mutex::new(PersistSlot {
                pending: None,
                writing: false,
                shutdown: false,
            }),
            wake: Condvar::new(),
        }
    }

    /// 覆盖式入队：新快照替换旧快照并唤醒写盘线程。
    fn enqueue(&self, snapshot: Vec<HostKeyRecord>) {
        self.slot.lock().pending = Some(snapshot);
        self.wake.notify_one();
    }
}

pub struct JsonKnownHostsStore {
    inner: Mutex<Vec<HostKeyRecord>>,
    worker: Arc<PersistWorker>,
    /// 常驻写盘线程句柄；Drop 时停机并 join。
    writer: Option<JoinHandle<()>>,
}

impl JsonKnownHostsStore {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let path = path.as_ref().to_path_buf();
        let inner = match fs::read_to_string(&path) {
            Ok(text) => {
                let file: KnownHostsFile = serde_json::from_str(&text).map_err(|error| {
                    AppError::with_detail(
                        ErrorCode::Configuration,
                        format!("invalid known_hosts file {}: {error}", path.display()),
                    )
                })?;
                file.hosts
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(AppError::with_detail(
                    ErrorCode::Io,
                    format!(
                        "读取已知主机文件失败，路径: {}，原因: {error}",
                        path.display()
                    ),
                ));
            }
        };
        let worker = Arc::new(PersistWorker::new());
        let writer = spawn_writer(path, Arc::clone(&worker)).map_err(|error| {
            AppError::with_detail(
                ErrorCode::Io,
                format!("创建 known_hosts 写盘线程失败，原因: {error}"),
            )
        })?;
        Ok(Self {
            inner: Mutex::new(inner),
            worker,
            writer: Some(writer),
        })
    }

    /// 持久化全量快照：经存储层的原子写落盘（唯一临时名 + fsync + rename）。
    fn persist_snapshot(hosts: &[HostKeyRecord], path: &Path) -> Result<(), AppError> {
        let file = KnownHostsFile {
            version: SCHEMA_VERSION,
            hosts: hosts.to_vec(),
        };
        let data = serde_json::to_vec_pretty(&file)
            .map_err(|error| AppError::with_detail(ErrorCode::Configuration, error.to_string()))?;
        crate::file_io::write_atomic(path, &data)
    }

    /// 等待待写快照落盘（阻塞）。仅测试与显式校验路径使用；
    /// 生产退出路径由 Drop 的停机 + join 保证。
    #[cfg(test)]
    fn flush(&self) {
        let mut slot = self.worker.slot.lock();
        while slot.pending.is_some() || slot.writing {
            self.worker.wake.wait(&mut slot);
        }
    }
}

impl Drop for JsonKnownHostsStore {
    fn drop(&mut self) {
        {
            let mut slot = self.worker.slot.lock();
            slot.shutdown = true;
        }
        self.worker.wake.notify_all();
        if let Some(handle) = self.writer.take()
            && handle.join().is_err()
        {
            log::warn!("known_hosts 写盘线程异常退出，最后一个快照可能未落盘");
        }
    }
}

/// 写盘线程主体：等唤醒、取最新快照、原子写。
fn writer_loop(path: PathBuf, worker: Arc<PersistWorker>) {
    loop {
        let snapshot = {
            let mut slot = worker.slot.lock();
            while slot.pending.is_none() && !slot.shutdown {
                worker.wake.wait(&mut slot);
            }
            if slot.pending.is_none() {
                // 停机且无待写：退出（有剩余快照时先写完再退）。
                return;
            }
            slot.writing = true;
            slot.pending.take()
        };
        if let Some(records) = snapshot
            && let Err(error) = JsonKnownHostsStore::persist_snapshot(&records, &path)
        {
            // 不冒泡：调用方（SSH 握手）早已返回；失败模式是下次连接重新
            // 提示确认指纹，属安全方向的降级。
            log::warn!("主机指纹落盘失败，本次仍保留在内存，原因: {error}");
        }
        {
            let mut slot = worker.slot.lock();
            slot.writing = false;
        }
        worker.wake.notify_all();
    }
}

fn spawn_writer(path: PathBuf, worker: Arc<PersistWorker>) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("transom-known-hosts-writer".into())
        .spawn(move || writer_loop(path, worker))
}

impl KnownHostsStore for JsonKnownHostsStore {
    fn check(&self, host: &str, port: u16, info: &HostKeyInfo) -> Result<HostKeyStatus, AppError> {
        let host = normalize_host(host);
        let hosts = self.inner.lock();
        let Some(record) = hosts
            .iter()
            .find(|record| record.host == host && record.port == port)
        else {
            return Ok(HostKeyStatus::Unknown);
        };
        if record.fingerprint == info.fingerprint {
            Ok(HostKeyStatus::Match)
        } else {
            Ok(HostKeyStatus::Mismatch {
                previous_fingerprint: record.fingerprint.clone(),
            })
        }
    }

    fn record(&self, host: &str, port: u16, info: &HostKeyInfo) -> Result<(), AppError> {
        let host = normalize_host(host);
        let snapshot = {
            let mut hosts = self.inner.lock();
            hosts.retain(|record| !(record.host == host && record.port == port));
            hosts.push(HostKeyRecord {
                host,
                port,
                algorithm: info.algorithm.clone(),
                fingerprint: info.fingerprint.clone(),
                public_key: info.public_key.clone(),
            });
            hosts.clone()
        };
        self.worker.enqueue(snapshot);
        Ok(())
    }

    fn remove(&self, host: &str, port: u16) -> Result<(), AppError> {
        let host = normalize_host(host);
        let snapshot = {
            let mut hosts = self.inner.lock();
            hosts.retain(|record| !(record.host == host && record.port == port));
            hosts.clone()
        };
        self.worker.enqueue(snapshot);
        Ok(())
    }

    fn list(&self) -> Result<Vec<HostKeyRecord>, AppError> {
        Ok(self.inner.lock().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn info(fingerprint: &str) -> HostKeyInfo {
        HostKeyInfo::new("ssh-ed25519", fingerprint, Some("ssh-ed25519 AAAA".into()))
    }

    #[cfg(unix)]
    #[test]
    fn persisted_known_hosts_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let path = dir.path().join("known_hosts.json");
        let store = JsonKnownHostsStore::new(&path).unwrap();
        store
            .record("example.com", 22, &info("SHA256:aaa"))
            .unwrap();
        // 落盘是异步的：断言文件属性前等待本批快照写完。
        store.flush();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "主机密钥指纹不得被同机其它用户读取");
    }

    #[test]
    fn tofu_flow_persists_and_detects_changes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("known_hosts.json");
        let store = JsonKnownHostsStore::new(&path).unwrap();

        assert_eq!(
            store.check("Example.COM", 22, &info("SHA256:aaa")).unwrap(),
            HostKeyStatus::Unknown
        );
        store
            .record("Example.COM", 22, &info("SHA256:aaa"))
            .unwrap();
        assert_eq!(
            store.check("example.com", 22, &info("SHA256:aaa")).unwrap(),
            HostKeyStatus::Match
        );
        assert_eq!(
            store.check("example.com", 22, &info("SHA256:bbb")).unwrap(),
            HostKeyStatus::Mismatch {
                previous_fingerprint: "SHA256:aaa".into()
            }
        );

        // 跨进程语义依赖落盘：等待异步写完成后重开文件核对。
        store.flush();
        let reopened = JsonKnownHostsStore::new(&path).unwrap();
        assert_eq!(
            reopened
                .check("example.com", 22, &info("SHA256:aaa"))
                .unwrap(),
            HostKeyStatus::Match
        );
        assert_eq!(reopened.list().unwrap().len(), 1);
    }

    #[test]
    fn remove_forgets_a_host() {
        let dir = tempdir().unwrap();
        let store = JsonKnownHostsStore::new(dir.path().join("known_hosts.json")).unwrap();
        store.record("host", 2222, &info("SHA256:aaa")).unwrap();
        store.remove("host", 2222).unwrap();
        assert_eq!(
            store.check("host", 2222, &info("SHA256:aaa")).unwrap(),
            HostKeyStatus::Unknown
        );
    }

    /// 覆盖式槽只保留最新快照：连续两次记录后 flush，文件里是两条而非旧态。
    #[test]
    fn rapid_records_converge_to_latest_snapshot() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("known_hosts.json");
        let store = JsonKnownHostsStore::new(&path).unwrap();
        store.record("a.example", 22, &info("SHA256:aaa")).unwrap();
        store.record("b.example", 22, &info("SHA256:bbb")).unwrap();
        store.flush();

        let reopened = JsonKnownHostsStore::new(&path).unwrap();
        let hosts = reopened.list().unwrap();
        assert_eq!(hosts.len(), 2, "最后一次全量快照必须包含全部记录");
    }
}
