//! Provider and location rows.

use crate::db::{map_sql, now_rfc3339};
use crate::types::{LocationKind, ProviderKind};
use crate::Catalog;
use portage_core::Error;
use rusqlite::OptionalExtension;

/// A `providers` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRow {
    /// Provider id (`local-d`, `ext-media`, …).
    pub id: String,
    /// Provider kind.
    pub kind: ProviderKind,
    /// Account, if any.
    pub account: Option<String>,
    /// Provider-specific JSON config (local: root, roles, volume serial, …).
    pub config_json: String,
}

impl Catalog {
    /// Insert or replace a provider.
    pub fn upsert_provider(
        &self,
        id: &str,
        kind: ProviderKind,
        account: Option<&str>,
    ) -> Result<(), Error> {
        let created = now_rfc3339()?;
        self.conn()
            .execute(
                "INSERT INTO providers (id, kind, account, config_json, created_at)
                 VALUES (?1, ?2, ?3, '{}', ?4)
                 ON CONFLICT(id) DO UPDATE SET
                   kind = excluded.kind,
                   account = excluded.account",
                rusqlite::params![id, kind.as_str(), account, created],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Insert a location. Re-insert of the same `(provider_id, root)` is a no-op
    /// on the unique key only when `id` matches; otherwise it is an error.
    pub fn upsert_location(
        &self,
        id: &str,
        provider_id: &str,
        kind: LocationKind,
        label: Option<&str>,
        root: Option<&str>,
    ) -> Result<(), Error> {
        self.conn()
            .execute(
                "INSERT INTO locations (id, provider_id, kind, label, root)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                   label = excluded.label,
                   root = excluded.root",
                rusqlite::params![id, provider_id, kind.as_str(), label, root],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Insert or replace a provider including its JSON config.
    pub fn upsert_provider_config(
        &self,
        id: &str,
        kind: ProviderKind,
        account: Option<&str>,
        config_json: &str,
    ) -> Result<(), Error> {
        let created = now_rfc3339()?;
        self.conn()
            .execute(
                "INSERT INTO providers (id, kind, account, config_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                   kind = excluded.kind,
                   account = excluded.account,
                   config_json = excluded.config_json",
                rusqlite::params![id, kind.as_str(), account, config_json, created],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// One provider by id.
    pub fn provider(&self, id: &str) -> Result<Option<ProviderRow>, Error> {
        self.conn()
            .query_row(
                "SELECT id, kind, account, config_json FROM providers WHERE id = ?1",
                [id],
                map_provider_row,
            )
            .optional()
            .map_err(map_sql)
    }

    /// Every provider, ordered by id.
    pub fn providers(&self) -> Result<Vec<ProviderRow>, Error> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id, kind, account, config_json FROM providers ORDER BY id")
            .map_err(map_sql)?;
        let rows = stmt.query_map([], map_provider_row).map_err(map_sql)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_sql)
    }
}

fn map_provider_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderRow> {
    let kind: String = row.get(1)?;
    Ok(ProviderRow {
        id: row.get(0)?,
        kind: kind.parse().map_err(|e: Error| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?,
        account: row.get(2)?,
        config_json: row.get(3)?,
    })
}
