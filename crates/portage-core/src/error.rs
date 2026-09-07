//! Portage error type. Exit-code mapping lives in the CLI.

use std::path::PathBuf;

/// Errors shared across Portage crates.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The catalog lock is held by another process.
    #[error("catalog locked by pid {pid}")]
    CatalogLocked {
        /// Process id of the lock holder, if known.
        pid: u32,
    },

    /// A chosen `data_dir` is unsafe (overlay, Cloud Filter, or low free space).
    #[error("unsafe data_dir {}: {reason}", dir.display())]
    UnsafeDataDir {
        /// The rejected directory.
        dir: PathBuf,
        /// Why it was rejected.
        reason: String,
    },

    /// Wrapper for I/O errors with the path that failed.
    #[error("io error at {}: {source}", path.display())]
    Io {
        /// The path being accessed.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A string failed to parse as a typed id (e.g. `b3:<64 hex>`).
    #[error("invalid {what}: {input}")]
    InvalidId {
        /// What was being parsed.
        what: &'static str,
        /// The offending input (truncated by the caller if huge).
        input: String,
    },

    /// A hash algorithm was requested that this build cannot compute yet.
    #[error("unsupported hash algorithm: {0}")]
    UnsupportedHashAlgo(String),

    /// A candidate path escapes its root (traversal, ADS, or symlink escape).
    #[error("path {} escapes root {}: {reason}", candidate.display(), root.display())]
    PathEscape {
        /// The registered root.
        root: PathBuf,
        /// The rejected candidate.
        candidate: PathBuf,
        /// Why it was rejected.
        reason: String,
    },

    /// A safety invariant was violated; the current plan must stop.
    #[error("invariant violated: {0}")]
    Invariant(String),

    /// SQLite catalog error (constraint, I/O inside sqlite, unexpected row).
    #[error("catalog: {0}")]
    Catalog(String),

    /// The on-disk catalog was migrated by a newer Portage than this binary.
    #[error("catalog schema version {found} is newer than this build (supports {supported})")]
    CatalogTooNew {
        /// `PRAGMA user_version` read from the file.
        found: i32,
        /// Highest version this binary knows how to open.
        supported: i32,
    },
}
