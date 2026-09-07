//! Journal tables ship in `0002_plans_journal.sql`. Write API is PRs 11–13.

use crate::Catalog;

impl Catalog {
    /// True when the 0002 `journal_ops` table exists (includes `we_created`).
    pub fn journal_schema_ready(&self) -> Result<bool, portage_core::Error> {
        self.table_exists("journal_ops")
    }
}
