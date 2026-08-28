//! Open, migrate, and hold a catalog connection plus its `portage.lock`.

use std::path::Path;

use portage_core::{DataPaths, Error};
use rusqlite::{Connection, OptionalExtension};

use crate::lock::{CatalogLock, LockMode};

/// Highest `PRAGMA user_version` this binary can open.
pub const CURRENT_SCHEMA_VERSION: i32 = 2;

const MIGRATION_0001: &str = include_str!("../../../migrations/0001_init.sql");
const MIGRATION_0002: &str = include_str!("../../../migrations/0002_plans_journal.sql");

/// Open catalog with counts used by `doctor` and tests.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CatalogCounts {
    /// `files` rows.
    pub files: i64,
    /// `blobs` rows.
    pub blobs: i64,
    /// `replicas` rows.
    pub replicas: i64,
}

/// Result of `PRAGMA integrity_check` + `PRAGMA foreign_key_check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityReport {
    /// `ok` or the first integrity_check message.
    pub integrity: String,
    /// Number of foreign-key violations.
    pub foreign_key_violations: usize,
}

impl IntegrityReport {
    /// True when both checks are clean.
    pub fn is_ok(&self) -> bool {
        self.integrity == "ok" && self.foreign_key_violations == 0
    }
}

/// An open catalog. The lock is held for the lifetime of this value.
pub struct Catalog {
    conn: Connection,
    _lock: CatalogLock,
    paths: DataPaths,
}

impl Catalog {
    /// Open (or create) the catalog under `data_dir` with `mode`.
    ///
    /// Exclusive may create and migrate. Shared refuses if the file is missing
    /// or `user_version` is behind this binary — migrate under exclusive first.
    pub fn open(data_dir: impl AsRef<Path>, mode: LockMode) -> Result<Self, Error> {
        let paths = DataPaths::new(data_dir.as_ref());

        // Shared open is read-only: refuse before creating any on-disk side effects.
        if mode == LockMode::Shared && !paths.catalog().exists() {
            return Err(Error::Catalog(
                "catalog.sqlite is missing; open exclusive to create and migrate".into(),
            ));
        }

        std::fs::create_dir_all(paths.data_dir()).map_err(|source| Error::Io {
            path: paths.data_dir().to_path_buf(),
            source,
        })?;

        let lock = CatalogLock::acquire(&paths.lock_file(), mode)?;
        let catalog_path = paths.catalog();

        let conn = Connection::open(&catalog_path).map_err(map_sql)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(map_sql)?;
        conn.pragma_update(None, "foreign_keys", true)
            .map_err(map_sql)?;
        conn.pragma_update(None, "busy_timeout", 5000)
            .map_err(map_sql)?;

        let found = user_version(&conn)?;
        if found > CURRENT_SCHEMA_VERSION {
            return Err(Error::CatalogTooNew {
                found,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
        if found < CURRENT_SCHEMA_VERSION {
            if mode != LockMode::Exclusive {
                return Err(Error::Catalog(format!(
                    "catalog schema {found} needs migrate to {CURRENT_SCHEMA_VERSION}; open exclusive"
                )));
            }
            migrate(&conn, found)?;
        }

        tracing::info!(
            path = %catalog_path.display(),
            ?mode,
            version = CURRENT_SCHEMA_VERSION,
            "catalog open"
        );
        Ok(Self {
            conn,
            _lock: lock,
            paths,
        })
    }

    /// Data-dir paths this catalog was opened from.
    pub fn paths(&self) -> &DataPaths {
        &self.paths
    }

    /// `PRAGMA user_version` after open/migrate.
    pub fn schema_version(&self) -> Result<i32, Error> {
        user_version(&self.conn)
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Row counts for doctor / tests.
    pub fn counts(&self) -> Result<CatalogCounts, Error> {
        Ok(CatalogCounts {
            files: count_star(self.conn(), "files")?,
            blobs: count_star(self.conn(), "blobs")?,
            replicas: count_star(self.conn(), "replicas")?,
        })
    }

    /// `PRAGMA integrity_check` plus foreign-key check.
    pub fn integrity(&self) -> Result<IntegrityReport, Error> {
        let integrity: String = self
            .conn()
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(map_sql)?;
        let mut stmt = self
            .conn()
            .prepare("PRAGMA foreign_key_check")
            .map_err(map_sql)?;
        let foreign_key_violations = stmt.query_map([], |_| Ok(())).map_err(map_sql)?.count();
        Ok(IntegrityReport {
            integrity,
            foreign_key_violations,
        })
    }

    /// Whether a user table exists (used to prove 0002 applied).
    pub fn table_exists(&self, name: &str) -> Result<bool, Error> {
        let found: Option<String> = self
            .conn()
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_sql)?;
        Ok(found.is_some())
    }

    /// Checkpoint WAL then copy `catalog.sqlite` to `dest`.
    pub fn checkpoint_and_backup(&self, dest: &Path) -> Result<(), Error> {
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(map_sql)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::copy(self.paths.catalog(), dest).map_err(|source| Error::Io {
            path: dest.to_path_buf(),
            source,
        })?;
        tracing::info!(dest = %dest.display(), "catalog backup written");
        Ok(())
    }

    /// `%data_dir%/catalog-YYYYMMDD.sqlite`, or with `-HHMMSS` if that exists.
    pub fn backup_path_today(&self) -> Result<std::path::PathBuf, Error> {
        let now = time::OffsetDateTime::now_utc();
        let date = format!(
            "{:04}{:02}{:02}",
            now.year(),
            u8::from(now.month()),
            now.day()
        );
        let base = self.paths.data_dir().join(format!("catalog-{date}.sqlite"));
        if !base.exists() {
            return Ok(base);
        }
        let stamp = format!("{:02}{:02}{:02}", now.hour(), now.minute(), now.second());
        Ok(self
            .paths
            .data_dir()
            .join(format!("catalog-{date}-{stamp}.sqlite")))
    }
}

fn user_version(conn: &Connection) -> Result<i32, Error> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(map_sql)
}

fn set_user_version(conn: &Connection, version: i32) -> Result<(), Error> {
    conn.pragma_update(None, "user_version", version)
        .map_err(map_sql)
}

fn migrate(conn: &Connection, from: i32) -> Result<(), Error> {
    if from < 1 {
        conn.execute_batch(MIGRATION_0001).map_err(map_sql)?;
        set_user_version(conn, 1)?;
    }
    if from < 2 {
        conn.execute_batch(MIGRATION_0002).map_err(map_sql)?;
        set_user_version(conn, 2)?;
    }
    Ok(())
}

fn count_star(conn: &Connection, table: &str) -> Result<i64, Error> {
    // Table names are crate-internal, not user input.
    let sql = format!("SELECT COUNT(*) FROM {table}");
    conn.query_row(&sql, [], |row| row.get(0)).map_err(map_sql)
}

pub(crate) fn map_sql(err: rusqlite::Error) -> Error {
    Error::Catalog(err.to_string())
}

pub(crate) fn now_rfc3339() -> Result<String, Error> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| Error::Catalog(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn exclusive_creates_and_migrates_to_v2() {
        let dir = tmp();
        let cat = Catalog::open(dir.path(), LockMode::Exclusive).unwrap();
        assert_eq!(cat.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
        assert!(cat.table_exists("files").unwrap());
        assert!(cat.table_exists("plans").unwrap());
        assert!(cat.table_exists("journal_ops").unwrap());
        assert!(cat.integrity().unwrap().is_ok());
        assert_eq!(cat.counts().unwrap(), CatalogCounts::default());
    }

    #[test]
    fn shared_refuses_missing_catalog() {
        let dir = tmp();
        match Catalog::open(dir.path(), LockMode::Shared) {
            Err(Error::Catalog(msg)) => assert!(msg.contains("missing"), "{msg}"),
            Err(e) => panic!("expected missing catalog, got {e}"),
            Ok(_) => panic!("expected missing catalog, got Ok"),
        }
    }

    #[test]
    fn shared_opens_after_exclusive_migrate() {
        let dir = tmp();
        drop(Catalog::open(dir.path(), LockMode::Exclusive).unwrap());
        let cat = Catalog::open(dir.path(), LockMode::Shared).unwrap();
        assert_eq!(cat.schema_version().unwrap(), 2);
    }

    #[test]
    fn too_new_refuses_to_open() {
        let dir = tmp();
        drop(Catalog::open(dir.path(), LockMode::Exclusive).unwrap());
        {
            let conn = Connection::open(dir.path().join("catalog.sqlite")).unwrap();
            conn.pragma_update(None, "user_version", 99).unwrap();
        }
        match Catalog::open(dir.path(), LockMode::Exclusive) {
            Err(Error::CatalogTooNew {
                found: 99,
                supported: CURRENT_SCHEMA_VERSION,
            }) => {}
            Err(e) => panic!("expected CatalogTooNew, got {e}"),
            Ok(_) => panic!("expected CatalogTooNew, got Ok"),
        }
    }
}
