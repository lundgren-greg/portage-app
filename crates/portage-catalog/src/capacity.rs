//! Capacity snapshots.

use crate::db::{map_sql, now_rfc3339};
use crate::types::CapacitySnapshot;
use crate::Catalog;
use portage_core::Error;
use rusqlite::OptionalExtension;

impl Catalog {
    /// Insert a capacity snapshot. `measured_at` is now (UTC).
    pub fn insert_capacity(&self, snap: &CapacitySnapshot) -> Result<i64, Error> {
        let measured = now_rfc3339()?;
        self.conn()
            .execute(
                "INSERT INTO capacity_snapshots
                   (location_id, total_bytes, used_bytes, free_bytes, quota_bytes, measured_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    snap.location_id,
                    snap.total_bytes,
                    snap.used_bytes,
                    snap.free_bytes,
                    snap.quota_bytes,
                    measured
                ],
            )
            .map_err(map_sql)?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Latest snapshot for a location.
    pub fn latest_capacity(&self, location_id: &str) -> Result<Option<CapacitySnapshot>, Error> {
        self.conn()
            .query_row(
                "SELECT location_id, total_bytes, used_bytes, free_bytes, quota_bytes
                 FROM capacity_snapshots
                 WHERE location_id = ?1
                 ORDER BY id DESC
                 LIMIT 1",
                [location_id],
                |row| {
                    Ok(CapacitySnapshot {
                        location_id: row.get(0)?,
                        total_bytes: row.get(1)?,
                        used_bytes: row.get(2)?,
                        free_bytes: row.get(3)?,
                        quota_bytes: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(map_sql)
    }
}
