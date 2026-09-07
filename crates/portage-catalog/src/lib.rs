//! SQLite catalog: open/migrate, lock, file/blob/replica queries.
//!
//! Schema lives in repo-root `migrations/`. `PRAGMA user_version` is the
//! catalog version; a newer file than this binary refuses to open.

pub mod blobs;
pub mod capacity;
pub mod db;
pub mod files;
pub mod journal;
pub mod lock;
pub mod plans;
pub mod providers;
pub mod replicas;
pub mod scans;
pub mod types;

pub use db::{Catalog, CatalogCounts, IntegrityReport, CURRENT_SCHEMA_VERSION};
pub use files::{InsertedFile, NewChecksum, NewFile};
pub use lock::{CatalogLock, LockMode};
pub use types::{
    BlobRow, CapacitySnapshot, FileKind, FileRow, Hydration, LocationKind, ProviderKind,
    ReplicaRow, ReplicaState, ScanStatus,
};
