//! Blob rows. A proto-blob has `content_id` NULL until hashed.

use crate::db::map_sql;
use crate::types::BlobRow;
use crate::Catalog;
use portage_core::ids::{BlobId, ContentId};
use portage_core::Error;
use rusqlite::OptionalExtension;

impl Catalog {
    /// Lookup a blob by its BLAKE3 identity.
    pub fn blob_by_content_id(&self, id: &ContentId) -> Result<Option<BlobRow>, Error> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id, content_id, size, mime FROM blobs WHERE content_id = ?1")
            .map_err(map_sql)?;
        stmt.query_row([id.to_string()], map_blob_row)
            .optional()
            .map_err(map_sql)
    }

    /// Attach a BLAKE3 identity to a proto-blob.
    pub fn set_content_id(&self, blob_id: BlobId, content_id: &ContentId) -> Result<(), Error> {
        let n = self
            .conn()
            .execute(
                "UPDATE blobs SET content_id = ?1 WHERE id = ?2",
                rusqlite::params![content_id.to_string(), blob_id.0],
            )
            .map_err(map_sql)?;
        if n != 1 {
            return Err(Error::Catalog(format!("blob {} not found", blob_id.0)));
        }
        Ok(())
    }
}

pub(crate) fn insert_proto(
    tx: &rusqlite::Transaction<'_>,
    size: i64,
    mime: Option<&str>,
) -> Result<BlobRow, Error> {
    tx.execute(
        "INSERT INTO blobs (content_id, size, mime) VALUES (NULL, ?1, ?2)",
        rusqlite::params![size, mime],
    )
    .map_err(map_sql)?;
    let id = BlobId(tx.last_insert_rowid());
    Ok(BlobRow {
        id,
        content_id: None,
        size,
        mime: mime.map(str::to_string),
    })
}

fn map_blob_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BlobRow> {
    let raw: Option<String> = row.get(1)?;
    let content_id = match raw {
        None => None,
        Some(s) => Some(s.parse::<ContentId>().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?),
    };
    Ok(BlobRow {
        id: BlobId(row.get(0)?),
        content_id,
        size: row.get(2)?,
        mime: row.get(3)?,
    })
}
