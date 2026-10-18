//! `overlay_roots`: cloud sync roots the local walker must never descend (K6).

use crate::db::map_sql;
use crate::Catalog;
use portage_core::Error;

/// An `overlay_roots` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayRootRow {
    /// Absolute path of the overlay root.
    pub path: String,
    /// Owning provider (`overlay:onedrive` until the cloud provider is added).
    pub provider_id: String,
    /// Detector name (`onedrive_userfolder`, `drivefs_mount`, …).
    pub detector: String,
}

impl Catalog {
    /// Insert or update an overlay root. Re-detection refreshes the detector
    /// but never downgrades a configured cloud provider id back to
    /// `overlay:*`.
    pub fn upsert_overlay_root(
        &self,
        path: &str,
        provider_id: &str,
        detector: &str,
    ) -> Result<(), Error> {
        self.conn()
            .execute(
                "INSERT INTO overlay_roots (path, provider_id, detector)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(path) DO UPDATE SET
                   detector = excluded.detector,
                   provider_id = CASE
                     WHEN overlay_roots.provider_id LIKE 'overlay:%' THEN excluded.provider_id
                     ELSE overlay_roots.provider_id
                   END",
                rusqlite::params![path, provider_id, detector],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// All registered overlay roots, ordered by path.
    pub fn overlay_roots(&self) -> Result<Vec<OverlayRootRow>, Error> {
        let mut stmt = self
            .conn()
            .prepare("SELECT path, provider_id, detector FROM overlay_roots ORDER BY path")
            .map_err(map_sql)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(OverlayRootRow {
                    path: row.get(0)?,
                    provider_id: row.get(1)?,
                    detector: row.get(2)?,
                })
            })
            .map_err(map_sql)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_sql)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Catalog, LockMode};

    #[test]
    fn upsert_is_idempotent_and_keeps_configured_provider() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cat = Catalog::open(tmp.path(), LockMode::Exclusive).unwrap();
        assert!(cat.overlay_roots().unwrap().is_empty());

        let od = r"C:\Users\example\OneDrive";
        let det = "onedrive_userfolder";
        cat.upsert_overlay_root(od, "overlay:onedrive", det)
            .unwrap();
        cat.upsert_overlay_root(od, "overlay:onedrive", det)
            .unwrap();
        assert_eq!(cat.overlay_roots().unwrap().len(), 1);

        // A later PR binds the configured provider; re-detection must not undo it.
        cat.upsert_overlay_root(od, "onedrive", det).unwrap();
        cat.upsert_overlay_root(od, "overlay:onedrive", det)
            .unwrap();
        let rows = cat.overlay_roots().unwrap();
        assert_eq!(rows[0].provider_id, "onedrive");
        assert_eq!(rows[0].detector, "onedrive_userfolder");
    }
}
