use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use transom_core::{
    AppError, ErrorCode, HostKeyInfo, HostKeyRecord, HostKeyStatus, KnownHostsStore,
    normalize_host, private_fs,
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

    fn persist(&self, hosts: &[HostKeyRecord]) -> Result<(), AppError> {
        let parent = self.path.parent().ok_or_else(|| {
            AppError::with_detail(ErrorCode::Configuration, "存储路径非法：没有父目录可写")
        })?;
        private_fs::ensure_private_dir(parent).map_err(AppError::from)?;

        let file = KnownHostsFile {
            version: SCHEMA_VERSION,
            hosts: hosts.to_vec(),
        };
        let data = serde_json::to_vec_pretty(&file)
            .map_err(|error| AppError::with_detail(ErrorCode::Configuration, error.to_string()))?;

        let temp = self.path.with_extension("json.tmp");
        let mut temp_file = private_fs::open_private_truncate(&temp).map_err(AppError::from)?;
        temp_file.write_all(&data).map_err(AppError::from)?;
        temp_file.flush().map_err(AppError::from)?;
        drop(temp_file);
        #[cfg(windows)]
        {
            // Windows rename 不覆盖已存在文件；先删除旧记录。NotFound 表示首次写入，
            // 其它删除失败必须上报，否则会直接 rename 失败、用户以为记录成功。
            if let Err(error) = fs::remove_file(&self.path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(AppError::from(error));
            }
        }
        fs::rename(&temp, &self.path).map_err(AppError::from)?;
        Ok(())
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
