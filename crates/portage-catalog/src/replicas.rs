//! Replica rows. Suspect ≠ last-copy.

use crate::db::map_sql;
use crate::types::{ReplicaRow, ReplicaState};
use crate::Catalog;
use portage_core::ids::BlobId;
use portage_core::Error;

impl Catalog {
    /// Replicas attached to a blob.
    pub fn replicas_for_blob(&self, blob_id: BlobId) -> Result<Vec<ReplicaRow>, Error> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id, blob_id, file_id, state FROM replicas WHERE blob_id = ?1")
            .map_err(map_sql)?;
        let rows = stmt
            .query_map([blob_id.0], map_replica_row)
            .map_err(map_sql)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(map_sql)?);
        }
        Ok(out)
    }
}

pub(crate) fn insert(
    tx: &rusqlite::Transaction<'_>,
    blob_id: BlobId,
    file_id: i64,
    state: ReplicaState,
) -> Result<ReplicaRow, Error> {
    tx.execute(
        "INSERT INTO replicas (blob_id, file_id, state) VALUES (?1, ?2, ?3)",
        rusqlite::params![blob_id.0, file_id, state.as_str()],
    )
    .map_err(map_sql)?;
    Ok(ReplicaRow {
        id: tx.last_insert_rowid(),
        blob_id,
        file_id,
        state,
    })
}

fn map_replica_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReplicaRow> {
    let state: String = row.get(3)?;
    Ok(ReplicaRow {
        id: row.get(0)?,
        blob_id: BlobId(row.get(1)?),
        file_id: row.get(2)?,
        state: state.parse().map_err(|e: portage_core::Error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?,
    })
}
