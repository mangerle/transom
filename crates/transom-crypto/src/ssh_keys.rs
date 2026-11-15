//! OpenSSH 密钥对生成、读取与本地公钥清单扫描。
//!
//! 设计原理（Why）：
//! - 私钥仅写入用户指定的绝对路径，且拒绝覆盖已有文件，避免误伤既有身份；
//! - 生成逻辑保持同步且不依赖 UI，调用方可将 RSA 4096 之类的耗时运算放入后台线程；
//! - 公钥清单只解析 `*.pub`，损坏或非公钥文件被静默跳过，保证向导始终可用。

use rand::rng;
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey, PublicKey};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use transom_core::{AppError, ErrorCode};
use transom_platform::private_fs;

/// 向导支持的密钥算法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAlgorithm {
    Ed25519,
    Rsa,
}

impl KeyAlgorithm {
    /// 面向界面的算法名称。
    pub fn label(self) -> &'static str {
        match self {
            Self::Ed25519 => "Ed25519",
            Self::Rsa => "RSA 4096",
        }
    }

    /// 与 OpenSSH 习惯一致的默认私钥文件名。
    pub fn default_file_name(self) -> &'static str {
        match self {
            Self::Ed25519 => "id_ed25519",
            Self::Rsa => "id_rsa",
        }
    }

    fn ssh_algorithm(self) -> Algorithm {
        match self {
            Self::Ed25519 => Algorithm::Ed25519,
            Self::Rsa => Algorithm::Rsa { hash: None },
        }
    }
}

/// 本地公钥文件的摘要信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKeyInfo {
    pub path: PathBuf,
    pub algorithm: String,
    pub fingerprint: String,
    pub comment: String,
    /// 单行 OpenSSH 公钥文本，可直接写入 `authorized_keys`。
    pub openssh: String,
}

/// 新生成密钥对的结果。
#[derive(Debug, Clone)]
pub struct GeneratedKey {
    pub private_path: PathBuf,
    pub public_path: PathBuf,
    pub public_key: PublicKeyInfo,
}

/// 生成并落盘一对 OpenSSH 密钥。
///
/// 私钥默认以 0600 权限写入（由 `ssh-key` 在 Unix 上保证），
/// 公钥写入 `<私钥路径>.pub`。传入非空 `passphrase` 时使用
/// bcrypt-pbkdf + AES-256-CTR 加密私钥。
///
/// # Errors
/// - 私钥路径不是绝对路径、注释包含换行或目标文件已存在时返回 `Configuration`；
/// - 密钥生成、加密或写盘失败时返回对应的 `Configuration` / `Io` 错误。
pub fn generate_keypair(
    algorithm: KeyAlgorithm,
    comment: &str,
    private_path: &Path,
    passphrase: Option<&str>,
) -> Result<GeneratedKey, AppError> {
    validate_comment(comment)?;
    if !private_path.is_absolute() {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "私钥保存路径必须是绝对路径",
        ));
    }
    if private_path.exists() {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            format!(
                "私钥文件已存在，为避免覆盖请更换文件名: {}",
                private_path.display()
            ),
        ));
    }
    let public_path = public_path_for(private_path);
    if public_path.exists() {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            format!(
                "公钥文件已存在，为避免覆盖请更换文件名: {}",
                public_path.display()
            ),
        ));
    }
    let parent = private_path
        .parent()
        .ok_or_else(|| AppError::with_detail(ErrorCode::Configuration, "私钥保存路径缺少父目录"))?;
    create_private_parent(parent)?;

    let mut key = PrivateKey::random(&mut rng(), algorithm.ssh_algorithm()).map_err(|error| {
        AppError::with_detail(
            ErrorCode::Configuration,
            format!("生成 {} 密钥失败: {error}", algorithm.label()),
        )
    })?;
    let comment = comment.trim();
    if !comment.is_empty() {
        key.set_comment(comment);
    }
    if let Some(passphrase) = passphrase.filter(|value| !value.is_empty()) {
        key = key.encrypt(&mut rng(), passphrase).map_err(|error| {
            AppError::with_detail(ErrorCode::Configuration, format!("加密私钥失败: {error}"))
        })?;
    }

    key.write_openssh_file(private_path, LineEnding::LF)
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::Io,
                format!(
                    "写入私钥失败，路径: {}，原因: {error}",
                    private_path.display()
                ),
            )
        })?;
    let public = key.public_key();
    let openssh = public.to_openssh().map_err(|error| {
        AppError::with_detail(ErrorCode::Configuration, format!("编码公钥失败: {error}"))
    })?;
    fs::write(&public_path, format!("{openssh}\n")).map_err(|error| {
        AppError::with_detail(
            ErrorCode::Io,
            format!(
                "写入公钥失败，路径: {}，原因: {error}",
                public_path.display()
            ),
        )
    })?;

    let public_key = PublicKeyInfo {
        path: public_path.clone(),
        algorithm: algorithm_label(public.algorithm()),
        fingerprint: public.fingerprint(HashAlg::Sha256).to_string(),
        comment: public.comment().as_str_lossy().to_string(),
        openssh,
    };
    Ok(GeneratedKey {
        private_path: private_path.to_path_buf(),
        public_path,
        public_key,
    })
}

/// 扫描目录下的 `*.pub` 公钥文件。
///
/// 目录不存在时返回空列表；单个文件无法解析时跳过而不中断整体扫描，
/// 这样用户手工放入的无关文本或损坏文件不会阻塞密钥管理界面。
///
/// # Errors
/// 目录存在但无法读取时返回 `Io` 错误。
pub fn scan_public_keys(ssh_dir: &Path) -> Result<Vec<PublicKeyInfo>, AppError> {
    let reader = match fs::read_dir(ssh_dir) {
        Ok(reader) => reader,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(AppError::with_detail(
                ErrorCode::Io,
                format!(
                    "读取 SSH 目录失败，路径: {}，原因: {error}",
                    ssh_dir.display()
                ),
            ));
        }
    };

    let mut keys = Vec::new();
    for entry in reader.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("pub") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(public) = PublicKey::from_openssh(&content) else {
            continue;
        };
        let Ok(openssh) = public.to_openssh() else {
            continue;
        };
        keys.push(PublicKeyInfo {
            path,
            algorithm: algorithm_label(public.algorithm()),
            fingerprint: public.fingerprint(HashAlg::Sha256).to_string(),
            comment: public.comment().as_str_lossy().to_string(),
            openssh,
        });
    }
    keys.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(keys)
}

fn validate_comment(comment: &str) -> Result<(), AppError> {
    if comment
        .chars()
        .any(|value| value == '\n' || value == '\r' || value == '\0')
    {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "密钥注释不能包含换行符或空字符",
        ));
    }
    Ok(())
}

fn create_private_parent(parent: &Path) -> Result<(), AppError> {
    private_fs::ensure_private_dir(parent).map_err(|error| {
        AppError::with_detail(
            ErrorCode::Io,
            format!(
                "创建密钥目录失败，路径: {}，原因: {error}",
                parent.display()
            ),
        )
    })
}

fn public_path_for(private_path: &Path) -> PathBuf {
    let mut name = OsString::from(private_path.as_os_str());
    name.push(".pub");
    PathBuf::from(name)
}

fn algorithm_label(algorithm: Algorithm) -> String {
    match algorithm {
        Algorithm::Ed25519 => "Ed25519".to_string(),
        Algorithm::Rsa { .. } => "RSA".to_string(),
        other => other.as_str().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_ed25519_pair_and_scans_it() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let private_path = dir.path().join("id_test");
        let generated =
            generate_keypair(KeyAlgorithm::Ed25519, "tester@transom", &private_path, None)
                .expect("生成 Ed25519 密钥");

        assert!(generated.private_path.exists());
        assert!(generated.public_path.exists());
        assert!(generated.public_key.fingerprint.starts_with("SHA256:"));
        assert_eq!(generated.public_key.comment, "tester@transom");
        assert_eq!(generated.public_key.algorithm, "Ed25519");

        let keys = scan_public_keys(dir.path()).expect("扫描公钥");
        assert_eq!(keys, vec![generated.public_key.clone()]);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&generated.private_path)
                .expect("读取私钥元数据")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn refuses_to_overwrite_and_rejects_bad_input() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let private_path = dir.path().join("id_test");
        generate_keypair(KeyAlgorithm::Ed25519, "", &private_path, None).expect("首次生成");
        let overwrite = generate_keypair(KeyAlgorithm::Ed25519, "", &private_path, None);
        assert!(overwrite.is_err());

        let multiline = generate_keypair(
            KeyAlgorithm::Ed25519,
            "bad\ncomment",
            &dir.path().join("another"),
            None,
        );
        assert!(multiline.is_err());

        let relative = generate_keypair(KeyAlgorithm::Ed25519, "", Path::new("id_rel"), None);
        assert!(relative.is_err());
    }

    #[test]
    fn encrypts_private_key_when_passphrase_is_given() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let private_path = dir.path().join("id_encrypted");
        generate_keypair(
            KeyAlgorithm::Ed25519,
            "tester@transom",
            &private_path,
            Some("正确口令"),
        )
        .expect("生成加密密钥");

        let content = fs::read_to_string(&private_path).expect("读取私钥");
        let key = PrivateKey::from_openssh(&content).expect("解析 OpenSSH 私钥");
        assert!(key.is_encrypted());
        assert!(key.decrypt("正确口令").is_ok());
    }
}
