//! Volume identity and capacity.
//!
//! Identity is the volume serial, not the drive letter (K24). Windows uses
//! `GetVolumeInformationW`, `GetDriveTypeW`, `IOCTL_STORAGE_QUERY_PROPERTY`
//! (bus type) and `GetDiskFreeSpaceExW`. Linux/macOS use `st_dev` and
//! `statvfs`.

use std::io;
use std::path::{Path, PathBuf};

use super::pathcmp::{is_within, path_str, HOST_CASE_INSENSITIVE};

/// `GetDriveTypeW` classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveType {
    /// Internal disk.
    Fixed,
    /// USB stick / SD card / some external disks.
    Removable,
    /// Network share.
    Remote,
    /// Optical.
    CdRom,
    /// RAM disk.
    RamDisk,
    /// Anything else.
    Unknown,
}

impl DriveType {
    /// Map the raw `GetDriveTypeW` value.
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            2 => Self::Removable,
            3 => Self::Fixed,
            4 => Self::Remote,
            5 => Self::CdRom,
            6 => Self::RamDisk,
            _ => Self::Unknown,
        }
    }

    /// Stored string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Removable => "removable",
            Self::Remote => "remote",
            Self::CdRom => "cdrom",
            Self::RamDisk => "ramdisk",
            Self::Unknown => "unknown",
        }
    }
}

/// Map a raw `STORAGE_BUS_TYPE` to a short name.
pub fn bus_name(raw: u32) -> &'static str {
    match raw {
        1 => "scsi",
        2 => "atapi",
        3 => "ata",
        4 => "1394",
        7 => "usb",
        8 => "raid",
        9 => "iscsi",
        10 => "sas",
        11 => "sata",
        12 => "sd",
        13 => "mmc",
        14 | 15 => "virtual",
        16 => "spaces",
        17 => "nvme",
        19 => "ufs",
        _ => "other",
    }
}

/// What we know about the volume holding a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeInfo {
    /// Mount root (`E:\`, `/`, `/media/usb`).
    pub mount: PathBuf,
    /// Volume serial (`1A2B-3C4D` on Windows, `dev:<hex>` elsewhere).
    pub serial: String,
    /// Volume label (may be empty).
    pub label: String,
    /// Filesystem name (`NTFS`, `exFAT`, …; empty if unknown).
    pub fs_name: String,
    /// Drive type.
    pub drive_type: DriveType,
    /// Bus (`usb`, `nvme`, `sata`, …) when the OS reports it.
    pub bus: Option<String>,
}

impl VolumeInfo {
    /// Removable media or attached over USB / SD.
    pub fn is_removable(&self) -> bool {
        self.drive_type == DriveType::Removable
            || matches!(self.bus.as_deref(), Some("usb" | "sd" | "mmc" | "1394"))
    }

    /// Why this whole volume is a cloud overlay (DriveFS / Cloud Filter), if
    /// it is one.
    pub fn cloud_volume_reason(&self) -> Option<String> {
        let hay = format!("{} {}", self.label, self.fs_name).to_lowercase();
        for needle in ["google drive", "drivefs", "cloudfilter", "cloud filter"] {
            if hay.contains(needle) {
                return Some(format!(
                    "volume {} looks like a DriveFS / Cloud Filter mount (label {:?}, fs {:?})",
                    self.mount.display(),
                    self.label,
                    self.fs_name
                ));
            }
        }
        None
    }
}

/// Free / total bytes on a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity {
    /// Total bytes, if known.
    pub total_bytes: Option<u64>,
    /// Used bytes (`total - free_on_disk`).
    pub used_bytes: u64,
    /// Bytes available to this user. Authoritative for planning.
    pub free_bytes: u64,
    /// Cloud quota (always `None` for local volumes).
    pub quota_bytes: Option<u64>,
}

/// Given the recorded serial and the currently mounted volumes, find where the
/// volume is now mounted. Drive-letter changes (`E:` → `F:`) are resolved here.
pub fn find_mount_for_serial<'a>(
    serial: &str,
    mounted: &'a [VolumeInfo],
) -> Option<&'a VolumeInfo> {
    mounted
        .iter()
        .find(|v| v.serial.eq_ignore_ascii_case(serial))
}

/// Rebase `root` from `old_mount` onto `new_mount` (`E:\Media` → `F:\Media`).
/// Returns `None` if `root` is not under `old_mount`.
pub fn rebase_root(root: &Path, old_mount: &Path, new_mount: &Path) -> Option<PathBuf> {
    let root_s = path_str(root);
    let old_s = path_str(old_mount);
    if !is_within(&root_s, &old_s, HOST_CASE_INSENSITIVE) {
        return None;
    }
    // Drop the mount prefix by length (it may differ only in case).
    let prefix_len = old_s.trim_end_matches(['\\', '/']).len();
    let rel = root_s.get(prefix_len..)?.trim_start_matches(['\\', '/']);
    Some(new_mount.join(rel))
}

/// Probe the volume that holds `path` (must exist).
pub fn probe(path: &Path) -> io::Result<VolumeInfo> {
    imp::probe(path)
}

/// Every currently mounted volume (Windows: drive letters). Used to follow a
/// removable disk to a new letter. Empty off Windows.
pub fn mounted_volumes() -> Vec<VolumeInfo> {
    imp::mounted_volumes()
}

/// Capacity of the volume mounted at `mount`.
pub fn capacity(mount: &Path) -> io::Result<Capacity> {
    imp::capacity(mount)
}

#[cfg(windows)]
mod imp {
    use super::*;

    use crate::local::win::{to_wide, Handle};

    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
        GetVolumePathNameW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Ioctl::{
        PropertyStandardQuery, StorageDeviceProperty, IOCTL_STORAGE_QUERY_PROPERTY,
        STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    pub fn probe(path: &Path) -> io::Result<VolumeInfo> {
        let wide = to_wide(path.as_os_str());
        let mut buf = [0u16; 1024];
        let len = buf.len() as u32;
        // SAFETY: `wide` is NUL-terminated; `buf` length is passed.
        let ok = unsafe { GetVolumePathNameW(wide.as_ptr(), buf.as_mut_ptr(), len) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let mount = PathBuf::from(from_wide(&buf));
        probe_mount(&mount)
    }

    fn probe_mount(mount: &Path) -> io::Result<VolumeInfo> {
        let wide = to_wide(mount.as_os_str());
        let mut label = [0u16; 261];
        let mut fs = [0u16; 261];
        let mut serial: u32 = 0;
        // SAFETY: all out-pointers are valid for the lengths passed.
        let ok = unsafe {
            GetVolumeInformationW(
                wide.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                &mut serial,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: NUL-terminated wide string.
        let drive_type = DriveType::from_raw(unsafe { GetDriveTypeW(wide.as_ptr()) });
        Ok(VolumeInfo {
            mount: mount.to_path_buf(),
            serial: format!("{:04X}-{:04X}", serial >> 16, serial & 0xFFFF),
            label: from_wide(&label),
            fs_name: from_wide(&fs),
            drive_type,
            bus: bus_type(mount),
        })
    }

    /// Bus type via `IOCTL_STORAGE_QUERY_PROPERTY` on `\\.\X:`. The volume
    /// device is opened with zero access (metadata only). Best-effort.
    fn bus_type(mount: &Path) -> Option<String> {
        let s = path_str(mount);
        let letter = s.strip_suffix('\\').unwrap_or(&s);
        if letter.len() != 2 || !letter.ends_with(':') {
            return None;
        }
        let device = to_wide(std::ffi::OsStr::new(&format!(r"\\.\{letter}")));
        // SAFETY: valid NUL-terminated path; zero desired access.
        let raw = unsafe {
            CreateFileW(
                device.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        let handle = Handle::new(raw)?;
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        let mut out = [0u8; 1024];
        let mut returned: u32 = 0;
        // SAFETY: in/out buffers are valid for the sizes passed.
        let ok = unsafe {
            DeviceIoControl(
                handle.raw(),
                IOCTL_STORAGE_QUERY_PROPERTY,
                &query as *const _ as *const _,
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                out.as_mut_ptr() as *mut _,
                out.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || (returned as usize) < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
            return None;
        }
        let ptr = out.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR;
        // SAFETY: the kernel filled at least one descriptor; read unaligned.
        let desc = unsafe { std::ptr::read_unaligned(ptr) };
        Some(bus_name(desc.BusType as u32).to_string())
    }

    pub fn mounted_volumes() -> Vec<VolumeInfo> {
        // SAFETY: no arguments.
        let mask = unsafe { GetLogicalDrives() };
        (0..26u32)
            .filter(|i| mask & (1 << i) != 0)
            .filter_map(|i| {
                let root = PathBuf::from(format!("{}:\\", (b'A' + i as u8) as char));
                probe_mount(&root).ok()
            })
            .collect()
    }

    pub fn capacity(mount: &Path) -> io::Result<Capacity> {
        let wide = to_wide(mount.as_os_str());
        let path = wide.as_ptr();
        let (mut avail, mut total, mut total_free) = (0u64, 0u64, 0u64);
        // SAFETY: `path` is NUL-terminated; out-pointers are valid u64s.
        let ok = unsafe { GetDiskFreeSpaceExW(path, &mut avail, &mut total, &mut total_free) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Capacity {
            total_bytes: Some(total),
            used_bytes: total.saturating_sub(total_free),
            free_bytes: avail,
            quota_bytes: None,
        })
    }

    fn from_wide(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    pub fn probe(path: &Path) -> io::Result<VolumeInfo> {
        let canonical = path.canonicalize()?;
        let dev = std::fs::metadata(&canonical)?.dev();
        // Walk up while still on the same device to find the mount root.
        let mut mount = canonical.clone();
        while let Some(parent) = mount.parent() {
            match std::fs::metadata(parent) {
                Ok(m) if m.dev() == dev => mount = parent.to_path_buf(),
                _ => break,
            }
        }
        Ok(VolumeInfo {
            mount,
            serial: format!("dev:{dev:x}"),
            label: String::new(),
            fs_name: String::new(),
            drive_type: DriveType::Fixed,
            bus: None,
        })
    }

    pub fn mounted_volumes() -> Vec<VolumeInfo> {
        Vec::new()
    }

    pub fn capacity(mount: &Path) -> io::Result<Capacity> {
        let stats = fs2::statvfs(mount)?;
        Ok(Capacity {
            total_bytes: Some(stats.total_space()),
            used_bytes: stats.total_space().saturating_sub(stats.free_space()),
            free_bytes: stats.available_space(),
            quota_bytes: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vol(mount: &str, serial: &str, label: &str, fs: &str, dt: DriveType) -> VolumeInfo {
        VolumeInfo {
            mount: PathBuf::from(mount),
            serial: serial.into(),
            label: label.into(),
            fs_name: fs.into(),
            drive_type: dt,
            bus: None,
        }
    }

    #[test]
    fn drive_type_and_bus_mapping() {
        assert_eq!(DriveType::from_raw(2), DriveType::Removable);
        assert_eq!(DriveType::from_raw(3), DriveType::Fixed);
        assert_eq!(DriveType::from_raw(4), DriveType::Remote);
        assert_eq!(DriveType::from_raw(5), DriveType::CdRom);
        assert_eq!(DriveType::from_raw(6), DriveType::RamDisk);
        assert_eq!(DriveType::from_raw(0), DriveType::Unknown);
        assert_eq!(DriveType::Removable.as_str(), "removable");
        assert_eq!(bus_name(7), "usb");
        assert_eq!(bus_name(17), "nvme");
        assert_eq!(bus_name(11), "sata");
        assert_eq!(bus_name(999), "other");
    }

    #[test]
    fn removable_flag_from_drive_type_or_bus() {
        let mut v = vol("E:\\", "1", "", "exFAT", DriveType::Removable);
        assert!(v.is_removable());
        v.drive_type = DriveType::Fixed;
        assert!(!v.is_removable());
        v.bus = Some("usb".into());
        assert!(v.is_removable(), "USB SSD reports Fixed but is external");
        v.bus = Some("nvme".into());
        assert!(!v.is_removable());
    }

    #[test]
    fn drivefs_volume_is_detected_by_label_or_fs() {
        let by_label = vol("G:\\", "1", "Google Drive", "FAT32", DriveType::Fixed);
        assert!(by_label.cloud_volume_reason().is_some());
        let by_fs = vol("G:\\", "1", "", "DriveFS", DriveType::Fixed);
        assert!(by_fs.cloud_volume_reason().is_some());
        let plain = vol("D:\\", "1", "Data", "NTFS", DriveType::Fixed);
        assert!(plain.cloud_volume_reason().is_none());
    }

    #[test]
    fn serial_lookup_follows_letter_change() {
        let mounted = [
            vol("C:\\", "AAAA-0001", "", "NTFS", DriveType::Fixed),
            vol("F:\\", "BBBB-0002", "ext", "exFAT", DriveType::Removable),
        ];
        let found = find_mount_for_serial("bbbb-0002", &mounted).unwrap();
        assert_eq!(found.mount, PathBuf::from("F:\\"));
        assert!(find_mount_for_serial("CCCC-0003", &mounted).is_none());
    }

    #[test]
    fn rebase_root_onto_new_mount() {
        let old = Path::new("/media/old");
        let new = Path::new("/media/new");
        assert_eq!(
            rebase_root(Path::new("/media/old/clips"), old, new),
            Some(PathBuf::from("/media/new/clips"))
        );
        assert_eq!(
            rebase_root(Path::new("/media/old"), old, new),
            Some(PathBuf::from("/media/new"))
        );
        assert_eq!(rebase_root(Path::new("/other"), old, new), None);
    }

    #[test]
    fn probe_and_capacity_of_temp_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let info = probe(tmp.path()).unwrap();
        assert!(!info.serial.is_empty());
        assert!(info.cloud_volume_reason().is_none());
        // Same serial on a second probe (identity is stable).
        assert_eq!(probe(tmp.path()).unwrap().serial, info.serial);
        let cap = capacity(&info.mount).unwrap();
        assert!(cap.total_bytes.unwrap() > 0);
        assert!(cap.free_bytes <= cap.total_bytes.unwrap());
        assert_eq!(cap.quota_bytes, None);
    }
}
