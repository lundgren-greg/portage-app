//! Provider and location rows.

use crate::db::{map_sql, now_rfc3339};
use crate::types::{LocationKind, ProviderKind};
use crate::Catalog;
use portage_core::Error;

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
}
