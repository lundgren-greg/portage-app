//! File insert + lookup. Byte files get a proto-blob on insert.

use crate::blobs;
use crate::db::map_sql;
use crate::replicas;
use crate::types::{BlobRow, FileKind, FileRow, Hydration, ReplicaRow, ReplicaState};
use crate::Catalog;
use portage_core::Error;
use rusqlite::OptionalExtension;

/// A file to insert.
#[derive(Debug, Clone)]
pub struct NewFile {
    /// Owning location.
    pub location_id: String,
    /// Parent directory file id.
    pub parent_id: Option<i64>,
    /// Provider-relative path, no `..`.
    pub path: String,
    /// Basename.
    pub name: String,
    /// Byte / directory / shortcut.
    pub kind: FileKind,
    /// Set iff `kind == Shortcut`.
    pub shortcut_target_ref: Option<String>,
    /// Size in bytes.
    pub size: Option<i64>,
    /// Last-write UTC (RFC3339).
    pub mtime_utc: Option<String>,
    /// NTFS file id (local).
    pub ntfs_file_id: Option<String>,
    /// Volume serial (local).
    pub volume_serial: Option<String>,
    /// MIME if known.
    pub mime: Option<String>,
    /// Hydration / overlay state.
    pub hydration: Hydration,
    /// Provider item id.
    pub remote_ref: Option<String>,
    /// Scan that produced this row.
    pub last_scan_id: Option<i64>,
    /// Optional provider checksums used to attach to an existing blob.
    pub checksums: Vec<NewChecksum>,
}

/// A provider checksum binding (schema in 0001; write API for later PRs).
#[derive(Debug, Clone)]
pub struct NewChecksum {
    /// Provider id.
    pub provider_id: String,
    /// Provider item id.
    pub remote_ref: String,
    /// md5 / sha1 / sha256 / quickxor.
    pub algo: String,
    /// Hex digest.
    pub hex: String,
    /// Size the provider reported.
    pub size: i64,
}

/// Result of inserting a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertedFile {
    /// The `files` row.
    pub file: FileRow,
    /// Proto-blob for byte files.
    pub blob: Option<BlobRow>,
    /// Replica for the file: `Suspect` for fully-local files, `Partial` for
    /// placeholders (not a last-copy), `None` for directories and shortcuts.
    pub replica: Option<ReplicaRow>,
}

impl Catalog {
    /// Insert one file. Byte files get a proto-blob (`content_id` NULL).
    /// Placeholder byte files get a proto-blob and a `Partial` replica (not last-copy).
    pub fn insert_file(&self, new: &NewFile) -> Result<InsertedFile, Error> {
        let tx = self.conn().unchecked_transaction().map_err(map_sql)?;
        let inserted = insert_file_in(&tx, new)?;
        tx.commit().map_err(map_sql)?;
        Ok(inserted)
    }

    /// Insert many files in one transaction.
    pub fn insert_files(&self, files: &[NewFile]) -> Result<Vec<InsertedFile>, Error> {
        let tx = self.conn().unchecked_transaction().map_err(map_sql)?;
        let mut out = Vec::with_capacity(files.len());
        for f in files {
            out.push(insert_file_in(&tx, f)?);
        }
        tx.commit().map_err(map_sql)?;
        Ok(out)
    }

    /// Lookup by `(location_id, path)`.
    pub fn file_by_path(&self, location_id: &str, path: &str) -> Result<Option<FileRow>, Error> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT id, location_id, parent_id, path, name, kind, shortcut_target_ref,
                        size, mtime_utc, ntfs_file_id, volume_serial, mime, hydration,
                        remote_ref, last_scan_id
                 FROM files WHERE location_id = ?1 AND path = ?2",
            )
            .map_err(map_sql)?;
        stmt.query_row(rusqlite::params![location_id, path], map_file_row)
            .optional()
            .map_err(map_sql)
    }

    /// Files whose blob has this `ContentId`.
    pub fn files_by_content_id(
        &self,
        id: &portage_core::ids::ContentId,
    ) -> Result<Vec<FileRow>, Error> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT f.id, f.location_id, f.parent_id, f.path, f.name, f.kind,
                        f.shortcut_target_ref, f.size, f.mtime_utc, f.ntfs_file_id,
                        f.volume_serial, f.mime, f.hydration, f.remote_ref, f.last_scan_id
                 FROM files f
                 JOIN replicas r ON r.file_id = f.id
                 JOIN blobs b ON b.id = r.blob_id
                 WHERE b.content_id = ?1",
            )
            .map_err(map_sql)?;
        let rows = stmt
            .query_map([id.to_string()], map_file_row)
            .map_err(map_sql)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(map_sql)?);
        }
        Ok(out)
    }

    /// Insert a provider checksum binding.
    pub fn insert_checksum(&self, row: &NewChecksum) -> Result<(), Error> {
        self.conn()
            .execute(
                "INSERT INTO provider_checksums
                   (provider_id, remote_ref, algo, hex, size, blob_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL)
                 ON CONFLICT(provider_id, remote_ref, algo) DO UPDATE SET
                   hex = excluded.hex,
                   size = excluded.size",
                rusqlite::params![row.provider_id, row.remote_ref, row.algo, row.hex, row.size],
            )
            .map_err(map_sql)?;
        Ok(())
    }
}

fn insert_file_in(tx: &rusqlite::Transaction<'_>, new: &NewFile) -> Result<InsertedFile, Error> {
    validate_catalog_path(&new.path)?;
    tx.execute(
        "INSERT INTO files (
            location_id, parent_id, path, name, kind, shortcut_target_ref, is_dir,
            size, mtime_utc, ntfs_file_id, volume_serial, mime, hydration,
            remote_ref, last_scan_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            new.location_id,
            new.parent_id,
            new.path,
            new.name,
            new.kind.as_str(),
            new.shortcut_target_ref,
            new.kind.is_dir() as i64,
            new.size,
            new.mtime_utc,
            new.ntfs_file_id,
            new.volume_serial,
            new.mime,
            new.hydration.as_str(),
            new.remote_ref,
            new.last_scan_id,
        ],
    )
    .map_err(map_sql)?;
    let file_id = tx.last_insert_rowid();

    let blob = if new.kind == FileKind::Byte {
        match find_blob_by_checksums(tx, &new.checksums)? {
            Some(existing) => Some(existing),
            None => {
                let size = new.size.ok_or_else(|| {
                    portage_core::Error::Invariant(
                        "byte file must have a size before inserting a proto-blob".into(),
                    )
                })?;
                Some(blobs::insert_proto(tx, size, new.mime.as_deref())?)
            }
        }
    } else {
        None
    };
    if let Some(b) = blob.as_ref() {
        store_checksums(tx, &new.checksums, b.id)?;
    }

    let replica = match (blob.as_ref(), new.hydration) {
        (Some(b), Hydration::Placeholder) => {
            // Placeholder: the file exists locally as an OS stub (not fully
            // downloaded). Insert a Partial replica to keep the file↔blob
            // association intact without counting as a last-copy.
            Some(replicas::insert(tx, b.id, file_id, ReplicaState::Partial)?)
        }
        (Some(b), _) => Some(replicas::insert(tx, b.id, file_id, ReplicaState::Suspect)?),
        (None, _) => None,
    };

    let file = FileRow {
        id: file_id,
        location_id: new.location_id.clone(),
        parent_id: new.parent_id,
        path: new.path.clone(),
        name: new.name.clone(),
        kind: new.kind,
        shortcut_target_ref: new.shortcut_target_ref.clone(),
        size: new.size,
        mtime_utc: new.mtime_utc.clone(),
        ntfs_file_id: new.ntfs_file_id.clone(),
        volume_serial: new.volume_serial.clone(),
        mime: new.mime.clone(),
        hydration: new.hydration,
        remote_ref: new.remote_ref.clone(),
        last_scan_id: new.last_scan_id,
    };
    Ok(InsertedFile {
        file,
        blob,
        replica,
    })
}

fn map_file_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileRow> {
    let kind: String = row.get(5)?;
    let hydration: String = row.get(12)?;
    Ok(FileRow {
        id: row.get(0)?,
        location_id: row.get(1)?,
        parent_id: row.get(2)?,
        path: row.get(3)?,
        name: row.get(4)?,
        kind: kind.parse().map_err(|e: portage_core::Error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?,
        shortcut_target_ref: row.get(6)?,
        size: row.get(7)?,
        mtime_utc: row.get(8)?,
        ntfs_file_id: row.get(9)?,
        volume_serial: row.get(10)?,
        mime: row.get(11)?,
        hydration: hydration.parse().map_err(|e: portage_core::Error| {
            rusqlite::Error::FromSqlConversionFailure(
                12,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?,
        remote_ref: row.get(13)?,
        last_scan_id: row.get(14)?,
    })
}

fn validate_catalog_path(path: &str) -> Result<(), Error> {
    if path.is_empty() || path.contains('\0') {
        return Err(Error::Invariant(format!("illegal catalog path: {path:?}")));
    }
    if path.split(['/', '\\']).any(|part| part == "..") {
        return Err(Error::Invariant(format!(
            "catalog path must not contain '..': {path}"
        )));
    }
    Ok(())
}

fn find_blob_by_checksums(
    tx: &rusqlite::Transaction<'_>,
    checksums: &[NewChecksum],
) -> Result<Option<crate::types::BlobRow>, Error> {
    for checksum in checksums {
        let found = tx
            .query_row(
                "SELECT b.id, b.content_id, b.size, b.mime
                 FROM provider_checksums c
                 JOIN blobs b ON b.id = c.blob_id
                 WHERE c.algo = ?1 AND c.hex = ?2 AND c.size = ?3 AND c.blob_id IS NOT NULL
                 LIMIT 1",
                rusqlite::params![checksum.algo, checksum.hex, checksum.size],
                |row| {
                    let raw: Option<String> = row.get(1)?;
                    let content_id = match raw {
                        None => None,
                        Some(s) => Some(s.parse().map_err(|e: Error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                1,
                                rusqlite::types::Type::Text,
                                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                            )
                        })?),
                    };
                    Ok(crate::types::BlobRow {
                        id: portage_core::ids::BlobId(row.get(0)?),
                        content_id,
                        size: row.get(2)?,
                        mime: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(map_sql)?;
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

fn store_checksums(
    tx: &rusqlite::Transaction<'_>,
    checksums: &[NewChecksum],
    blob_id: portage_core::ids::BlobId,
) -> Result<(), Error> {
    for checksum in checksums {
        tx.execute(
            "INSERT INTO provider_checksums
                (provider_id, remote_ref, algo, hex, size, blob_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(provider_id, remote_ref, algo) DO UPDATE SET
                hex = excluded.hex,
                size = excluded.size,
                blob_id = excluded.blob_id",
            rusqlite::params![
                checksum.provider_id,
                checksum.remote_ref,
                checksum.algo,
                checksum.hex,
                checksum.size,
                blob_id.0,
            ],
        )
        .map_err(map_sql)?;
    }
    Ok(())
}
