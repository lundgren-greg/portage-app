//! Placeholder detection from metadata attributes only (K6).
//!
//! Never opens the file. On Windows the attributes come from the directory
//! enumeration (`FindNextFileW` data) that `std::fs::DirEntry::metadata`
//! already holds. On Linux/macOS every file is `LocalFull`.

use std::fs::Metadata;

use portage_catalog::Hydration;

/// `FILE_ATTRIBUTE_REPARSE_POINT`
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
/// `FILE_ATTRIBUTE_OFFLINE`
pub const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
/// `FILE_ATTRIBUTE_RECALL_ON_OPEN`
pub const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
/// `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`
pub const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

/// Attributes that mean "bytes are not (all) on this disk".
const RECALL_MASK: u32 =
    FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_OFFLINE;

/// Classify Windows file attributes.
///
/// Placeholder if any recall/offline bit is set, or if the entry is a reparse
/// point that is not a symlink/junction (Cloud Filter `IO_REPARSE_TAG_CLOUD*`
/// files carry such a reparse point). Unknown reparse points are treated as
/// placeholders: opening one is never worth the risk of a hydrate.
pub fn classify_attributes(attributes: u32, is_symlink: bool) -> Hydration {
    if attributes & RECALL_MASK != 0 {
        return Hydration::Placeholder;
    }
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && !is_symlink {
        return Hydration::Placeholder;
    }
    Hydration::LocalFull
}

/// Raw attribute bits for `meta` (0 off Windows).
pub fn attributes(meta: &Metadata) -> u32 {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes()
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        0
    }
}

/// Hydration of an entry from metadata already in hand. Always `LocalFull`
/// off Windows.
pub fn hydration_of(meta: &Metadata) -> Hydration {
    if cfg!(windows) {
        classify_attributes(attributes(meta), meta.file_type().is_symlink())
    } else {
        Hydration::LocalFull
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_files_are_local_full() {
        assert_eq!(classify_attributes(0, false), Hydration::LocalFull);
        // ARCHIVE | READONLY
        assert_eq!(classify_attributes(0x20 | 0x01, false), Hydration::LocalFull);
    }

    #[test]
    fn recall_and_offline_bits_are_placeholders() {
        for bits in [
            FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
            FILE_ATTRIBUTE_RECALL_ON_OPEN,
            FILE_ATTRIBUTE_OFFLINE,
            FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | 0x20,
        ] {
            assert_eq!(classify_attributes(bits, false), Hydration::Placeholder);
            // Even if the OS also reports a link, recall bits win.
            assert_eq!(classify_attributes(bits, true), Hydration::Placeholder);
        }
    }

    #[test]
    fn reparse_points_other_than_links_are_placeholders() {
        assert_eq!(
            classify_attributes(FILE_ATTRIBUTE_REPARSE_POINT, false),
            Hydration::Placeholder
        );
        assert_eq!(
            classify_attributes(FILE_ATTRIBUTE_REPARSE_POINT, true),
            Hydration::LocalFull
        );
    }

    #[test]
    fn real_temp_file_is_local_full() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let meta = std::fs::symlink_metadata(tmp.path()).unwrap();
        assert_eq!(hydration_of(&meta), Hydration::LocalFull);
    }
}
