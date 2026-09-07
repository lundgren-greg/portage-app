//! Data-dir layout. The catalog path is derived here so every crate agrees.
//!
//! Full YAML policy / engine config parsing is PR 9. This module only
//! answers "where do the catalog and lock live under a data dir?"

use std::path::{Path, PathBuf};

/// Filename of the SQLite catalog inside the data dir.
pub const CATALOG_FILENAME: &str = "catalog.sqlite";
/// Filename of the inter-process lock inside the data dir.
pub const LOCK_FILENAME: &str = "portage.lock";

/// Resolved paths under a Portage data directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPaths {
    data_dir: PathBuf,
}

impl DataPaths {
    /// Treat `data_dir` as the Portage data directory.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    /// Root of the data directory.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// `%data_dir%/catalog.sqlite`
    pub fn catalog(&self) -> PathBuf {
        self.data_dir.join(CATALOG_FILENAME)
    }

    /// `%data_dir%/portage.lock`
    pub fn lock_file(&self) -> PathBuf {
        self.data_dir.join(LOCK_FILENAME)
    }

    /// `%data_dir%/logs`
    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    /// `%data_dir%/metrics`
    pub fn metrics_dir(&self) -> PathBuf {
        self.data_dir.join("metrics")
    }

    /// `%data_dir%/metrics/portage.prom`
    pub fn metrics_snapshot(&self) -> PathBuf {
        self.metrics_dir().join("portage.prom")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_and_lock_sit_directly_under_data_dir() {
        let paths = DataPaths::new(PathBuf::from("D:\\PortageData"));
        assert_eq!(
            paths.catalog(),
            PathBuf::from("D:\\PortageData").join("catalog.sqlite")
        );
        assert_eq!(
            paths.lock_file(),
            PathBuf::from("D:\\PortageData").join("portage.lock")
        );
        assert_eq!(
            paths.metrics_snapshot(),
            PathBuf::from("D:\\PortageData")
                .join("metrics")
                .join("portage.prom")
        );
    }
}
