//! Windows 私有文件安全描述符与创建逻辑。
//!
//! 设计原理（Why）：Windows 没有 mode 位；只依赖用户目录继承 ACL 无法保证
//! 敏感文件在所有部署方式下都不可被其它本地用户读取。这里构造受保护 DACL，
//! 只授权当前用户、SYSTEM 与内置管理员，并在 `CreateFileW`/`CreateDirectoryW`
//! 创建对象时直接传入安全描述符，避免“先创建再补权限”的暴露窗口。

use std::fs::File;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SetEntriesInAclW,
    SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_IS_WELL_KNOWN_GROUP,
};
use windows::Win32::Security::{
    ACL, CopySid, CreateWellKnownSid, DACL_SECURITY_INFORMATION, GetLengthSid, GetTokenInformation,
    InitializeSecurityDescriptor, NO_INHERITANCE, OBJECT_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
    SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SECURITY_MAX_SID_SIZE,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT, SetSecurityDescriptorControl, SetSecurityDescriptorDacl,
    TOKEN_QUERY, TOKEN_USER, TokenUser, WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use windows::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_ALWAYS,
};
use windows::Win32::System::SystemServices::SECURITY_DESCRIPTOR_REVISION;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{BOOL, Error, PCWSTR, PWSTR};

const PRIVATE_PRINCIPALS: usize = 3;

/// 已构造的私有安全描述符与 ACL；ACL 由 `SetEntriesInAclW` 分配，需要显式释放。
pub(crate) struct PrivateSecurity {
    descriptor: Box<SECURITY_DESCRIPTOR>,
    acl: *mut ACL,
    attributes: SECURITY_ATTRIBUTES,
}

impl PrivateSecurity {
    pub(crate) fn new(inherit_for_directory: bool) -> io::Result<Self> {
        let user_sid = current_user_sid()?;
        let system_sid = well_known_sid(WinLocalSystemSid)?;
        let administrators_sid = well_known_sid(WinBuiltinAdministratorsSid)?;
        let inheritance = if inherit_for_directory {
            SUB_CONTAINERS_AND_OBJECTS_INHERIT
        } else {
            NO_INHERITANCE
        };
        let entries = [
            explicit_access(&user_sid, TRUSTEE_IS_USER, inheritance),
            explicit_access(&system_sid, TRUSTEE_IS_WELL_KNOWN_GROUP, inheritance),
            explicit_access(
                &administrators_sid,
                TRUSTEE_IS_WELL_KNOWN_GROUP,
                inheritance,
            ),
        ];
        debug_assert_eq!(entries.len(), PRIVATE_PRINCIPALS);

        let mut acl = std::ptr::null_mut();
        let error = unsafe { SetEntriesInAclW(Some(&entries), None, &mut acl) };
        if let Err(error) = win32_result(error, "构造私有 DACL") {
            if !acl.is_null() {
                unsafe {
                    LocalFree(Some(HLOCAL(acl.cast())));
                }
            }
            return Err(error);
        }
        if acl.is_null() {
            return Err(io::Error::other("构造私有 DACL 返回空指针"));
        }

        let mut descriptor = Box::new(unsafe { zeroed::<SECURITY_DESCRIPTOR>() });
        let descriptor_ptr =
            PSECURITY_DESCRIPTOR((&mut *descriptor as *mut SECURITY_DESCRIPTOR).cast());
        if let Err(error) = unsafe {
            InitializeSecurityDescriptor(descriptor_ptr, SECURITY_DESCRIPTOR_REVISION)
                .and_then(|()| SetSecurityDescriptorDacl(descriptor_ptr, true, Some(acl), false))
                .and_then(|()| {
                    SetSecurityDescriptorControl(
                        descriptor_ptr,
                        SE_DACL_PROTECTED,
                        SE_DACL_PROTECTED,
                    )
                })
        } {
            unsafe {
                LocalFree(Some(HLOCAL(acl.cast())));
            }
            return Err(io_error(error));
        }

        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor_ptr.0,
            bInheritHandle: BOOL(0),
        };
        Ok(Self {
            descriptor,
            acl,
            attributes,
        })
    }

    pub(crate) fn attributes_ptr(&self) -> *const SECURITY_ATTRIBUTES {
        let _ = &self.descriptor;
        &self.attributes
    }
}

impl Drop for PrivateSecurity {
    fn drop(&mut self) {
        unsafe {
            LocalFree(Some(HLOCAL(self.acl.cast())));
        }
    }
}

/// 创建私有目录；已存在的目录保持原 ACL，不静默改写用户已有配置。
pub(super) fn ensure_private_dir(path: &Path) -> io::Result<()> {
    if path.exists() {
        return path
            .is_dir()
            .then_some(())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotADirectory, "私有路径不是目录"));
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        ensure_private_dir(parent)?;
    }
    let security = PrivateSecurity::new(true)?;
    let path = wide_path(path);
    match unsafe { CreateDirectoryW(PCWSTR(path.as_ptr()), Some(security.attributes_ptr())) } {
        Ok(()) => Ok(()),
        Err(_error) if path_exists_after_race(&wide_to_path(&path)) => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}

/// 打开追加写私有文件；创建时直接使用私有安全描述符。
pub(super) fn open_private_append(path: &Path) -> io::Result<File> {
    open_private(path, OPEN_ALWAYS, "打开私有追加文件")
}

/// 打开截断私有文件；创建时直接使用私有安全描述符。
pub(super) fn open_private_truncate(path: &Path) -> io::Result<File> {
    open_private(path, CREATE_ALWAYS, "打开私有截断文件")
}

/// 对已打开文件重新应用私有 DACL，兜底旧版本文件或继承来的宽权限。
pub(super) fn restrict_file_permissions(file: &File) -> io::Result<()> {
    let security = PrivateSecurity::new(false)?;
    let information = OBJECT_SECURITY_INFORMATION(
        DACL_SECURITY_INFORMATION.0 | PROTECTED_DACL_SECURITY_INFORMATION.0,
    );
    let error = unsafe {
        SetSecurityInfo(
            HANDLE(file.as_raw_handle().cast()),
            SE_FILE_OBJECT,
            information,
            None,
            None,
            Some(security.acl),
            None,
        )
    };
    win32_result(error, "收紧私有文件 DACL")
}

fn open_private(
    path: &Path,
    disposition: windows::Win32::Storage::FileSystem::FILE_CREATION_DISPOSITION,
    action: &str,
) -> io::Result<File> {
    let security = PrivateSecurity::new(false)?;
    let wide = wide_path(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            Some(security.attributes_ptr()),
            disposition,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|error| contextual_error(action, path, error))?;
    let file = unsafe { File::from_raw_handle(handle.0) };
    restrict_file_permissions(&file).map_err(|error| contextual_error(action, path, error))?;
    Ok(file)
}

fn current_user_sid() -> io::Result<Vec<u32>> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.map_err(io_error)?;
    let token = TokenHandle(token);
    let mut size = 0u32;
    let _ = unsafe { GetTokenInformation(token.0, TokenUser, None, 0, &mut size) };
    if size == 0 {
        return Err(io::Error::other("读取当前用户令牌长度失败"));
    }
    let mut buffer = vec![0u32; (size as usize).div_ceil(size_of::<u32>())];
    unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            size,
            &mut size,
        )
    }
    .map_err(io_error)?;
    let token_user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    copy_sid(token_user.User.Sid)
}

fn well_known_sid(kind: windows::Win32::Security::WELL_KNOWN_SID_TYPE) -> io::Result<Vec<u32>> {
    let mut sid = vec![0u32; (SECURITY_MAX_SID_SIZE as usize).div_ceil(size_of::<u32>())];
    let mut size = (sid.len() * size_of::<u32>()) as u32;
    unsafe { CreateWellKnownSid(kind, None, Some(PSID(sid.as_mut_ptr().cast())), &mut size) }
        .map_err(io_error)?;
    sid.truncate((size as usize).div_ceil(size_of::<u32>()));
    Ok(sid)
}

fn copy_sid(source: PSID) -> io::Result<Vec<u32>> {
    let length = unsafe { GetLengthSid(source) };
    if length == 0 {
        return Err(io::Error::other("读取 SID 长度失败"));
    }
    let mut sid = vec![0u32; (length as usize).div_ceil(size_of::<u32>())];
    unsafe { CopySid(length, PSID(sid.as_mut_ptr().cast()), source) }.map_err(io_error)?;
    Ok(sid)
}

fn explicit_access(
    sid: &[u32],
    trustee_type: windows::Win32::Security::Authorization::TRUSTEE_TYPE,
    inheritance: windows::Win32::Security::ACE_FLAGS,
) -> EXPLICIT_ACCESS_W {
    EXPLICIT_ACCESS_W {
        grfAccessPermissions: windows::Win32::Storage::FileSystem::FILE_ALL_ACCESS.0,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: inheritance,
        Trustee: windows::Win32::Security::Authorization::TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: trustee_type,
            ptstrName: PWSTR(sid.as_ptr() as *mut u16),
        },
    }
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn wide_to_path(wide: &[u16]) -> std::path::PathBuf {
    let end = wide
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(wide.len());
    std::ffi::OsString::from_wide(&wide[..end]).into()
}

fn path_exists_after_race(path: &Path) -> bool {
    path.is_dir()
}

fn win32_result(error: WIN32_ERROR, action: &str) -> io::Result<()> {
    if error == ERROR_SUCCESS {
        Ok(())
    } else {
        let source = io::Error::from_raw_os_error(error.0 as i32);
        Err(io::Error::new(
            source.kind(),
            format!("{action}失败，Windows 错误码: {}，原因: {source}", error.0),
        ))
    }
}

fn io_error(error: Error) -> io::Error {
    io::Error::other(format!(
        "Windows API 调用失败，错误码: {:#x}，原因: {}",
        error.code().0,
        error.message()
    ))
}

fn contextual_error(action: &str, path: &Path, error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!(
        "{action}失败，路径: {}，原因: {error}",
        path.display()
    ))
}

struct TokenHandle(HANDLE);

impl Drop for TokenHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
pub(super) fn assert_private_acl(path: &Path, directory: bool) {
    use windows::Win32::Security::Authorization::GetSecurityInfo;
    use windows::Win32::Security::{
        GetSecurityDescriptorControl, GetSecurityDescriptorDacl, SECURITY_DESCRIPTOR_CONTROL,
    };
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, OPEN_EXISTING, READ_CONTROL,
    };

    let wide = wide_path(path);
    let flags = if directory {
        FILE_FLAG_BACKUP_SEMANTICS
    } else {
        FILE_ATTRIBUTE_NORMAL
    };
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            READ_CONTROL.0 | FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
    }
    .expect("测试对象必须可打开以读取安全信息");
    let file = unsafe { File::from_raw_handle(handle.0) };
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let error = unsafe {
        GetSecurityInfo(
            HANDLE(file.as_raw_handle().cast()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            Some(&mut descriptor),
        )
    };
    win32_result(error, "读取私有对象 DACL").expect("必须能读取 DACL");
    let descriptor_guard = LocalDescriptor(descriptor);

    let mut control = 0u16;
    let mut revision = 0u32;
    unsafe { GetSecurityDescriptorControl(descriptor_guard.0, &mut control, &mut revision) }
        .expect("必须能读取安全描述符控制位");
    assert!(
        SECURITY_DESCRIPTOR_CONTROL(control).contains(SE_DACL_PROTECTED),
        "私有对象必须使用受保护 DACL"
    );
    let mut present = BOOL(0);
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut defaulted = BOOL(1);
    unsafe {
        GetSecurityDescriptorDacl(descriptor_guard.0, &mut present, &mut acl, &mut defaulted)
    }
    .expect("必须能读取 DACL 指针");
    assert!(present.as_bool());
    assert!(!defaulted.as_bool());
    assert!(!acl.is_null());
    assert_eq!(
        u32::from(unsafe { (*acl).AceCount }),
        PRIVATE_PRINCIPALS as u32
    );
}

#[cfg(test)]
struct LocalDescriptor(PSECURITY_DESCRIPTOR);

#[cfg(test)]
impl Drop for LocalDescriptor {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            unsafe {
                LocalFree(Some(HLOCAL(self.0.0.cast())));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PRIVATE_PRINCIPALS, PrivateSecurity};
    use windows::Win32::Security::{
        ACL, GetSecurityDescriptorControl, GetSecurityDescriptorDacl, PSECURITY_DESCRIPTOR,
        SE_DACL_PROTECTED, SECURITY_DESCRIPTOR_CONTROL,
    };
    use windows::core::BOOL;

    #[test]
    fn private_descriptor_is_protected_and_has_three_principals() {
        let security = PrivateSecurity::new(true).unwrap();
        let descriptor = PSECURITY_DESCRIPTOR(security.attributes.lpSecurityDescriptor);
        let mut control = 0u16;
        let mut revision = 0u32;
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) }.unwrap();
        assert!(
            SECURITY_DESCRIPTOR_CONTROL(control).contains(SE_DACL_PROTECTED),
            "私有 DACL 必须阻止父目录继承 ACE 混入"
        );

        let mut present = BOOL(0);
        let mut acl: *mut ACL = std::ptr::null_mut();
        let mut defaulted = BOOL(1);
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) }
            .unwrap();
        assert!(present.as_bool());
        assert!(!defaulted.as_bool());
        assert!(!acl.is_null());
        assert_eq!(
            u32::from(unsafe { (*acl).AceCount }),
            PRIVATE_PRINCIPALS as u32
        );
    }
}
