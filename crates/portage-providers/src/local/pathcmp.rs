//! Pure path comparison used by the overlay detectors.
//!
//! Works on strings so Windows-shaped paths (`C:\Users\example\OneDrive`)
//! can be unit-tested on Linux CI. Production passes
//! [`HOST_CASE_INSENSITIVE`].

use std::path::{Path, PathBuf};

/// Default case rule for the host OS (NTFS/ReFS compare case-insensitively).
pub const HOST_CASE_INSENSITIVE: bool = cfg!(windows);

/// Split a path into comparable components. Accepts `\` and `/`, strips
/// `\\?\` / `\\?\UNC\` verbatim prefixes, drops empty and `.` parts, and
/// folds `..` into its parent in absolute paths. `..` never climbs above a
/// drive (`C:`), a UNC share (`\\server\share`), or `/`.
fn components(path: &str, case_insensitive: bool) -> Vec<String> {
    let trimmed = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.to_string()
    };
    let unc = trimmed.starts_with(r"\\") || trimmed.starts_with("//");
    let drive = {
        let b = trimmed.as_bytes();
        b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
    };
    let mut out: Vec<String> = Vec::new();
    if unc {
        out.push(String::from(r"\\"));
    } else if trimmed.starts_with('/') {
        out.push(String::from("/"));
    }
    // What `..` may not pop: `\\` + server + share, the drive, or `/`.
    let floor = match (unc, drive) {
        (true, _) => 3,
        (false, true) => 1,
        (false, false) => out.len(),
    };
    let absolute = unc || drive || !out.is_empty();
    for p in trimmed.split(['\\', '/']) {
        if p.is_empty() || p == "." {
            continue;
        }
        if p == ".." && absolute {
            if out.len() > floor {
                out.pop();
            }
            continue;
        }
        out.push(if case_insensitive {
            p.to_lowercase()
        } else {
            p.to_string()
        });
    }
    out
}

/// Absolute `path` with `.` / `..` folded (see [`components`]); relative
/// paths are returned unchanged. Detector roots are stored in this form so
/// they compare like the canonical roots `provider add local` records.
pub fn normalize(path: &str) -> String {
    let parts = components(path, false);
    match parts.first().map(String::as_str) {
        Some(r"\\") => format!(r"\\{}", parts[1..].join(r"\")),
        Some("/") => format!("/{}", parts[1..].join("/")),
        Some(d) if d.len() == 2 && d.ends_with(':') => {
            if parts.len() == 1 {
                format!(r"{d}\")
            } else {
                parts.join(r"\")
            }
        }
        _ => path.to_string(),
    }
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
    fn parent_components_fold_but_never_cross_drive_or_share() {
        let od = r"C:\Users\example\OneDrive";
        let dotted = r"C:\Users\example\AppData\Local\..\..\OneDrive";
        assert!(is_within(&format!(r"{dotted}\x"), od, true));
        assert!(is_within(r"C:\Users\example\OneDrive\x", dotted, true));
        assert!(same_path(r"C:\..\..\Users", r"C:\Users", true));
        assert!(!is_within(r"\\nas\share\..\other\x", r"\\nas\other", true));
        assert!(is_within(r"\\nas\share\..\..\x", r"\\nas\share", true));
        assert!(same_path("/a/b/../c", "/a/c", false));
        assert!(same_path("/../a", "/a", false));
        // Relative paths keep `..`: there is nothing to fold it into.
        assert!(!same_path("../a", "a", false));
        assert_eq!(normalize(dotted), od);
        assert_eq!(normalize("G:"), r"G:\");
        assert_eq!(normalize(r"\\nas\s\a\..\b"), r"\\nas\s\b");
        assert_eq!(normalize("/a/./b/../c"), "/a/c");
        assert_eq!(normalize("rel/../x"), "rel/../x");
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
