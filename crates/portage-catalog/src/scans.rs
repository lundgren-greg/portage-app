//! Scan sessions (`scans` + `scan_cursors`).

use crate::db::{map_sql, now_rfc3339};
use crate::types::ScanStatus;
use crate::Catalog;
use portage_core::Error;

impl Catalog {
    /// Start a scan for `provider_id`. Returns the new scan id.
    pub fn start_scan(&self, provider_id: &str) -> Result<i64, Error> {
        let started = now_rfc3339()?;
        self.conn()
            .execute(
                "INSERT INTO scans (provider_id, started_at, status)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![provider_id, started, ScanStatus::Running.as_str()],
            )
            .map_err(map_sql)?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Finish a scan with a terminal status and files-seen count.
    pub fn finish_scan(
        &self,
        scan_id: i64,
        files_seen: i64,
        status: ScanStatus,
    ) -> Result<(), Error> {
        if status == ScanStatus::Running {
            return Err(Error::Catalog(
                "finish_scan requires a terminal status (ok|error)".into(),
            ));
        }
        let finished = now_rfc3339()?;
        let n = self
            .conn()
            .execute(
                "UPDATE scans
                 SET finished_at = ?1, files_seen = ?2, status = ?3
                 WHERE id = ?4",
                rusqlite::params![finished, files_seen, status.as_str(), scan_id],
            )
            .map_err(map_sql)?;
        if n != 1 {
            return Err(Error::Catalog(format!("scan {scan_id} not found")));
        }
        Ok(())
    }
}
