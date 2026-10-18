//! Win32 helpers: read-only HKCU registry reader and metadata-only handles.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING, WIN32_FIND_DATAW,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    REG_EXPAND_SZ, REG_SZ,
};

use super::sync_roots::RegistryReader;

/// NUL-terminated UTF-16.
pub(crate) fn to_wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Owned Win32 handle, closed on drop.
pub(crate) struct Handle(HANDLE);

impl Handle {
    pub(crate) fn new(raw: HANDLE) -> Option<Self> {
        if raw == INVALID_HANDLE_VALUE || raw.is_null() {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub(crate) fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: we own a valid handle.
        unsafe { CloseHandle(self.0) };
    }
}

/// NTFS file reference number (`volume serial` scoped), as 16 hex digits.
///
/// Opens with **zero desired access** and `FILE_FLAG_OPEN_REPARSE_POINT`, so
/// no data is read and a Cloud Filter reparse point is not followed. Callers
/// must still only call this for entries already classified `LocalFull`.
pub(crate) fn file_id(path: &Path) -> Option<String> {
    let wide = to_wide(path.as_os_str());
    // SAFETY: NUL-terminated path; zero access; no template.
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    let handle = Handle::new(raw)?;
    // SAFETY: zeroed POD out-struct.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: valid handle and out-pointer.
    if unsafe { GetFileInformationByHandle(handle.raw(), &mut info) } == 0 {
        return None;
    }
    Some(format!(
        "{:08x}{:08x}",
        info.nFileIndexHigh, info.nFileIndexLow
    ))
}

/// `(attributes, file id)` of an already-open file, from the handle itself
/// (so it describes the object being read, not whatever the path names now).
pub(crate) fn handle_identity(file: &std::fs::File) -> Option<(u32, String)> {
    use std::os::windows::io::AsRawHandle;
    // SAFETY: zeroed POD out-struct.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `file` owns a valid handle for the duration of the call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } == 0 {
        return None;
    }
    Some((
        info.dwFileAttributes,
        format!("{:08x}{:08x}", info.nFileIndexHigh, info.nFileIndexLow),
    ))
}

/// The long name on disk of the last component of `path`, from enumerating
/// its parent directory (no handle to the object itself is opened). Win32
/// resolves aliases here, so `name.` or an 8.3 `NAME~1` comes back as the
/// real name. The caller must reject wildcard characters first.
pub(crate) fn on_disk_name(path: &Path) -> std::io::Result<OsString> {
    let wide = to_wide(path.as_os_str());
    // SAFETY: zeroed POD out-struct.
    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    // SAFETY: NUL-terminated path; out-struct valid; no filter or flags.
    let raw = unsafe {
        FindFirstFileExW(
            wide.as_ptr(),
            FindExInfoBasic,
            &mut data as *mut _ as *mut core::ffi::c_void,
            FindExSearchNameMatch,
            std::ptr::null(),
            0,
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: valid search handle from FindFirstFileExW.
    unsafe { FindClose(raw) };
    let len = data
        .cFileName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(data.cFileName.len());
    Ok(OsString::from_wide(&data.cFileName[..len]))
}

/// Read-only `HKEY_CURRENT_USER`.
pub struct WinRegistry;

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: we own an open key.
        unsafe { RegCloseKey(self.0) };
    }
}

fn open(key: &str) -> Option<Key> {
    let wide = to_wide(OsStr::new(key));
    let mut out: HKEY = std::ptr::null_mut();
    // SAFETY: NUL-terminated subkey; out-pointer valid.
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide.as_ptr(), 0, KEY_READ, &mut out) };
    (rc == 0).then_some(Key(out))
}

fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

impl RegistryReader for WinRegistry {
    fn key_exists(&self, key: &str) -> bool {
        open(key).is_some()
    }

    fn subkeys(&self, key: &str) -> Vec<String> {
        let Some(k) = open(key) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for index in 0u32.. {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            // SAFETY: buffers valid for the lengths passed.
            let rc = unsafe {
                RegEnumKeyExW(
                    k.0,
                    index,
                    name.as_mut_ptr(),
                    &mut len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc != 0 {
                break;
            }
            out.push(wide_to_string(&name[..len as usize]));
        }
        out
    }

    fn string_values(&self, key: &str) -> Vec<(String, String)> {
        let Some(k) = open(key) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for index in 0u32.. {
            let mut name = [0u16; 16384];
            let mut name_len = name.len() as u32;
            let mut data = vec![0u16; 8192];
            let mut data_len = (data.len() * 2) as u32;
            let mut kind: u32 = 0;
            // SAFETY: buffers valid for the lengths passed (data in bytes).
            let rc = unsafe {
                RegEnumValueW(
                    k.0,
                    index,
                    name.as_mut_ptr(),
                    &mut name_len,
                    std::ptr::null(),
                    &mut kind,
                    data.as_mut_ptr() as *mut u8,
                    &mut data_len,
                )
            };
            // ERROR_MORE_DATA (234): value too large to be a path; skip it.
            if rc == 234 {
                continue;
            }
            if rc != 0 {
                break;
            }
            if kind == REG_SZ || kind == REG_EXPAND_SZ {
                let chars = (data_len as usize / 2).min(data.len());
                out.push((
                    wide_to_string(&name[..name_len as usize]),
                    wide_to_string(&data[..chars]),
                ));
            }
        }
        out
    }
}
