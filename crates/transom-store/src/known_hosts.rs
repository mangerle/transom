use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
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

pub struct JsonKnownHostsStore {
    path: PathBuf,
    inner: Mutex<Vec<HostKeyRecord>>,
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
        Ok(Self {
            path,
            inner: Mutex::new(inner),
        })
    }

    /// 持久化：内存快照经存储层的原子写落盘（唯一临时名 + fsync + rename）。
    ///
    /// 与其它持久化文件共用 `file_io::write_atomic`：这是本文件曾自行复述过
    /// 的一段流程，重复实现漏掉 fsync 时会在断电后留下空指纹库。
    fn persist(&self, hosts: &[HostKeyRecord]) -> Result<(), AppError> {
        let file = KnownHostsFile {
            version: SCHEMA_VERSION,
            hosts: hosts.to_vec(),
        };
        let data = serde_json::to_vec_pretty(&file)
            .map_err(|error| AppError::with_detail(ErrorCode::Configuration, error.to_string()))?;
        crate::file_io::write_atomic(&self.path, &data)
    }
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
        let mut hosts = self.inner.lock();
        hosts.retain(|record| !(record.host == host && record.port == port));
        hosts.push(HostKeyRecord {
            host,
            port,
            algorithm: info.algorithm.clone(),
            fingerprint: info.fingerprint.clone(),
            public_key: info.public_key.clone(),
        });
        self.persist(&hosts)
    }

    fn remove(&self, host: &str, port: u16) -> Result<(), AppError> {
        let host = normalize_host(host);
        let mut hosts = self.inner.lock();
        hosts.retain(|record| !(record.host == host && record.port == port));
        self.persist(&hosts)
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
}
