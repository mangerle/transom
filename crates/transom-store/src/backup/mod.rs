//! 会话目录的备份文件读写：文件形态、钥匙串口令出入口与格式探测。
//!
//! 加解密（Argon2id 派生 + XChaCha20-Poly1305 信封）位于
//! [`transom_core::crypto::backup`]：它同时被云同步复用，上提到契约层后
//! 网络层不再依赖本存储层。本文件只负责"文件"：读写、体积上限、原子落盘
//! 与钥匙串口令的导出/导入。
//!
//! 安全设计：
//! - 明文备份默认不含系统钥匙串里的密码/口令，只有用户显式勾选才会写入，
//!   且包含口令时必须加密（见 `BackupService::export` 的入口校验）；
//! - 文件以 0600 权限原子写入，导入前限制文件大小，防止恶意超大文件。

use std::fs;
use std::path::Path;
use std::sync::Arc;
use transom_core::crypto::backup::{
    BackupFormat, MAX_BACKUP_BYTES, decrypt_document, encrypt_document, is_encrypted_envelope,
};
use transom_core::{
    AppError, BackupDocument, BackupExportOptions, BackupExportReport, BackupImportOptions,
    BackupSecret, BackupService, ErrorCode, SecretKey, SecretRestoreReport, SecretStore,
    SessionCatalog, collect_catalog_secrets,
};

/// 会话目录备份服务：文件格式、加密与钥匙串口令的出入口。
///
/// 口令的读写放在这一层，是因为只有这里同时握有"备份内容"与"系统钥匙串"
/// 两个知识面：导出时按会话的认证方式取口令，导入时按"本机已存在则保留"
/// 的规则写回。
pub struct CatalogBackupService {
    secrets: Arc<dyn SecretStore>,
}

impl CatalogBackupService {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self { secrets }
    }
}

impl BackupService for CatalogBackupService {
    fn export(
        &self,
        path: &Path,
        catalog: &SessionCatalog,
        options: &BackupExportOptions,
    ) -> Result<BackupExportReport, AppError> {
        // 口令明文落盘是不可接受的：包含口令就必须加密。UI 会联动开关，
        // 但这里仍要拒绝，防止其它调用方或未来入口绕过界面约束。
        if options.include_secrets && options.passphrase.is_none() {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                "包含会话口令的备份必须加密；请设置备份口令，或取消包含口令",
            ));
        }
        let mut document = BackupDocument::from_catalog(catalog);
        if options.include_secrets {
            document.secrets = Some(collect_catalog_secrets(self.secrets.as_ref(), catalog)?);
        }
        let format = BackupFormat::from_path(path);
        write_backup(
            path,
            &document,
            options
                .passphrase
                .as_ref()
                .map(|passphrase| passphrase.as_str()),
            format,
        )?;
        Ok(BackupExportReport {
            sessions: catalog.sessions.len(),
            folders: catalog.folders.len(),
            secrets: document.secrets.as_ref().map_or(0, |secrets| secrets.len()),
            encrypted: options.passphrase.is_some(),
        })
    }

    fn import(
        &self,
        path: &Path,
        options: &BackupImportOptions,
    ) -> Result<BackupDocument, AppError> {
        read_backup(
            path,
            options
                .passphrase
                .as_ref()
                .map(|passphrase| passphrase.as_str()),
        )
    }

    fn restore_secrets(&self, secrets: &[BackupSecret]) -> Result<SecretRestoreReport, AppError> {
        let mut report = SecretRestoreReport::default();
        for secret in secrets {
            let key = SecretKey::new(secret.owner, secret.kind);
            match self.secrets.get(&key) {
                // 本机已有同名口令：保留现有值，导入备份不改本机正在用的密码。
                Ok(Some(_)) => report.kept += 1,
                Ok(None) => match self.secrets.set(&key, secret.value.as_str()) {
                    Ok(()) => report.restored += 1,
                    Err(error) => {
                        report.failed += 1;
                        log::warn!(
                            "备份口令写入钥匙串失败，归属: {:?}，类型: {:?}，原因: {error}",
                            secret.owner,
                            secret.kind
                        );
                    }
                },
                Err(error) => {
                    return Err(AppError::with_detail(
                        ErrorCode::SecretStoreUnavailable,
                        format!("系统钥匙串不可用，备份口令未能写回：{error}"),
                    ));
                }
            }
        }
        Ok(report)
    }
}

/// 写入备份文件：给了口令就加密，没给就写明文 JSON/YAML。
///
/// 与存储文件共用同一套原子写：唯一临时名 + fsync + rename 覆盖，0600
/// 权限由私有句柄在打开临时文件时收紧；导出后立即断电也不会得到空文件。
///
/// # Errors
/// 序列化/加密、父目录创建、写盘或重命名失败时返回 `AppError`。
pub fn write_backup(
    path: &Path,
    document: &BackupDocument,
    passphrase: Option<&str>,
    format: BackupFormat,
) -> Result<(), AppError> {
    let data = match passphrase {
        Some(passphrase) => encrypt_document(document, passphrase, format)?,
        None => serialize_document(document, format)?,
    };
    crate::file_io::write_atomic(path, &data)
}

/// 读取备份文件：给了口令按加密文件解密，没给按明文文件解析。
///
/// # Errors
/// 文件不存在、超过大小上限、读取失败、口令错误或格式不符时返回 `AppError`。
pub fn read_backup(path: &Path, passphrase: Option<&str>) -> Result<BackupDocument, AppError> {
    let bytes = read_backup_bytes(path)?;
    match passphrase {
        Some(passphrase) => {
            if !is_encrypted_envelope(&bytes) {
                return Err(AppError::with_detail(
                    ErrorCode::BackupNotEncrypted,
                    "该文件不是加密备份，请取消「已加密」勾选后重试",
                ));
            }
            decrypt_document(&bytes, passphrase)
        }
        None => {
            if is_encrypted_envelope(&bytes) {
                return Err(AppError::with_detail(
                    ErrorCode::BackupEncrypted,
                    "该文件是加密备份，请勾选「已加密」并输入口令后重试",
                ));
            }
            let mut document = parse_document(&bytes)?;
            document.normalize();
            Ok(document)
        }
    }
}

/// 读取备份文件字节，并限制体积避免恶意超大文件。
fn read_backup_bytes(path: &Path) -> Result<Vec<u8>, AppError> {
    let metadata = fs::metadata(path).map_err(|error| {
        AppError::with_detail(
            ErrorCode::Io,
            format!("读取备份文件失败，路径: {}，原因: {error}", path.display()),
        )
    })?;
    if metadata.len() > MAX_BACKUP_BYTES {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "备份文件超过 16 MiB 上限，已拒绝导入",
        ));
    }
    fs::read(path).map_err(AppError::from)
}

/// 解析明文备份文档。
fn parse_document(bytes: &[u8]) -> Result<BackupDocument, AppError> {
    serde_json::from_slice(bytes)
        .or_else(|_| serde_yaml_ng::from_slice(bytes))
        .map_err(|_| {
            AppError::with_detail(
                ErrorCode::Configuration,
                "备份文件无法解析，可能不是 Transom 会话备份",
            )
        })
}

/// 序列化明文备份文档。
fn serialize_document(
    document: &BackupDocument,
    format: BackupFormat,
) -> Result<Vec<u8>, AppError> {
    match format {
        BackupFormat::Json => serde_json::to_vec_pretty(document).map_err(|error| {
            AppError::with_detail(ErrorCode::Configuration, format!("序列化备份失败: {error}"))
        }),
        BackupFormat::Yaml => serde_yaml_ng::to_string(document)
            .map(String::into_bytes)
            .map_err(|error| {
                AppError::with_detail(ErrorCode::Configuration, format!("序列化备份失败: {error}"))
            }),
    }
}

#[cfg(test)]
mod tests;
