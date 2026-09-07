//! Plan tables ship in `0002_plans_journal.sql`. Write API is PRs 10–13.

use crate::Catalog;

impl Catalog {
    /// True when the 0002 `plans` table exists.
    pub fn plans_schema_ready(&self) -> Result<bool, portage_core::Error> {
        self.table_exists("plans")
    }
}
