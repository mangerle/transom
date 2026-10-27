use serde::{Serialize, de::DeserializeOwned};
use std::fs;
use std::io::Write;
use std::path::Path;
use transom_core::{AppError, ErrorCode, private_fs};

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, AppError> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|error| {
            AppError::with_detail(
                ErrorCode::Configuration,
                format!(
                    "配置文件内容损坏，无法解析，路径: {}，原因: {error}",
                    path.display()
                ),
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::with_detail(
            ErrorCode::Io,
            format!("读取配置文件失败，路径: {}，原因: {error}", path.display()),
        )),
    }
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| {
        AppError::with_detail(ErrorCode::Configuration, "存储路径非法：没有父目录可写")
    })?;
    private_fs::ensure_private_dir(parent).map_err(AppError::from)?;

    let data = serde_json::to_vec_pretty(value)
        .map_err(|error| AppError::with_detail(ErrorCode::Configuration, error.to_string()))?;
    let temp = path.with_extension("json.tmp");
    let mut temp_file = private_fs::open_private_truncate(&temp).map_err(AppError::from)?;
    temp_file.write_all(&data).map_err(AppError::from)?;
    temp_file.flush().map_err(AppError::from)?;
    // Windows 上必须先关闭句柄才能 rename，显式 drop 让两个平台行为一致。
    drop(temp_file);
    #[cfg(windows)]
    {
        let _ = fs::remove_file(path);
    }
    fs::rename(&temp, path).map_err(AppError::from)
}
