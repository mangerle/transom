use serde::{Serialize, de::DeserializeOwned};
use std::fs;
use std::path::Path;
use transom_core::{AppError, ErrorCode};

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, AppError> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|error| {
            AppError::with_detail(
                ErrorCode::Configuration,
                format!("invalid config file {}: {error}", path.display()),
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::with_detail(
            ErrorCode::Io,
            format!("failed to read {}: {error}", path.display()),
        )),
    }
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::with_detail(ErrorCode::Configuration, "invalid store path"))?;
    fs::create_dir_all(parent).map_err(AppError::from)?;

    let data = serde_json::to_vec_pretty(value)
        .map_err(|error| AppError::with_detail(ErrorCode::Configuration, error.to_string()))?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, data).map_err(AppError::from)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&temp, fs::Permissions::from_mode(0o600));
    }
    #[cfg(windows)]
    {
        let _ = fs::remove_file(path);
    }
    fs::rename(&temp, path).map_err(AppError::from)
}
