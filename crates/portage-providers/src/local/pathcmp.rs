//! Pure path comparison used by the overlay detectors.
//!
//! Works on strings so Windows-shaped paths (`C:\Users\example\OneDrive`)
//! can be unit-tested on Linux CI. Production passes
//! [`HOST_CASE_INSENSITIVE`].

use std::path::{Path, PathBuf};

/// Default case rule for the host OS (NTFS/ReFS compare case-insensitively).
pub const HOST_CASE_INSENSITIVE: bool = cfg!(windows);

/// Split a path into comparable components. Accepts `\` and `/`, strips
/// `\\?\` / `\\?\UNC\` verbatim prefixes, drops empty and `.` parts.
fn components(path: &str, case_insensitive: bool) -> Vec<String> {
    let trimmed = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.to_string()
    };
    let unc = trimmed.starts_with(r"\\") || trimmed.starts_with("//");
    let mut out: Vec<String> = trimmed
        .split(['\\', '/'])
        .filter(|p| !p.is_empty() && *p != ".")
        .map(|p| {
            if case_insensitive {
                p.to_lowercase()
            } else {
                p.to_string()
            }
        })
        .collect();
    if unc {
        out.insert(0, String::from(r"\\"));
    } else if trimmed.starts_with('/') {
        out.insert(0, String::from("/"));
    }
    out
}

/// True if `child` equals `parent` or lives under it.
pub fn is_within(child: &str, parent: &str, case_insensitive: bool) -> bool {
    let c = components(child, case_insensitive);
    let p = components(parent, case_insensitive);
    !p.is_empty() && c.len() >= p.len() && c[..p.len()] == p[..]
}

/// True if both name the same location.
pub fn same_path(a: &str, b: &str, case_insensitive: bool) -> bool {
    components(a, case_insensitive) == components(b, case_insensitive)
}

/// Lossy string form of a path for comparison.
pub fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Recognise registry data that is an absolute filesystem path:
/// `G:`, `G:\…`, `\\server\share…`. Returns it normalised (`G:` → `G:\`).
pub fn as_absolute_windows_path(data: &str) -> Option<String> {
    let s = data.trim().trim_matches('"');
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        if bytes.len() == 2 {
            return Some(format!("{s}\\"));
        }
        if bytes[2] == b'\\' || bytes[2] == b'/' {
            return Some(s.to_string());
        }
        return None;
    }
    if s.starts_with(r"\\") && s.len() > 2 {
        return Some(s.to_string());
    }
    None
}

/// Strip the `\\?\` verbatim prefix `std::fs::canonicalize` adds on Windows.
pub fn strip_verbatim(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest.to_string());
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_is_component_wise_and_case_insensitive_on_windows_rules() {
        let od = r"C:\Users\example\OneDrive";
        assert!(is_within(r"C:\Users\example\OneDrive", od, true));
        assert!(is_within(
            r"c:\users\EXAMPLE\onedrive\Clips\a.mp4",
            od,
            true
        ));
        assert!(is_within(r"\\?\C:\Users\example\OneDrive\x", od, true));
        assert!(!is_within(r"C:\Users\example\OneDriveBackup", od, true));
        assert!(!is_within(r"C:\Users\example", od, true));
        assert!(!is_within(r"c:\users\example\onedrive", od, false));
        assert!(is_within("/home/example/x", "/home/example", false));
        assert!(!is_within("/home/examplex", "/home/example", false));
        assert!(!is_within("anything", "", false));
    }

    #[test]
    fn same_path_ignores_trailing_separators_and_verbatim_prefix() {
        assert!(same_path(r"G:\", "g:", true));
        assert!(same_path(r"\\?\D:\Clips\", r"D:/Clips", true));
        assert!(!same_path(r"D:\Clips", r"D:\Clips2", true));
        assert!(same_path(r"\\?\UNC\nas\share", r"\\nas\share\", true));
        assert!(!same_path(r"\\nas\share", r"nas\share", true));
    }

    #[test]
    fn absolute_path_recognition() {
        assert_eq!(as_absolute_windows_path("G:").as_deref(), Some(r"G:\"));
        assert_eq!(
            as_absolute_windows_path(r"D:\Drive\Sync").as_deref(),
            Some(r"D:\Drive\Sync")
        );
        assert_eq!(
            as_absolute_windows_path(r#""E:/x""#).as_deref(),
            Some("E:/x")
        );
        assert_eq!(
            as_absolute_windows_path(r"\\nas\share").as_deref(),
            Some(r"\\nas\share")
        );
        for bad in ["", "1", "true", "G:x", r"relative\path", r"\\", "12:00"] {
            assert_eq!(as_absolute_windows_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn strip_verbatim_prefixes() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\x")),
            PathBuf::from(r"C:\x")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\nas\s")),
            PathBuf::from(r"\\nas\s")
        );
        assert_eq!(strip_verbatim(PathBuf::from("/tmp")), PathBuf::from("/tmp"));
    }
}
