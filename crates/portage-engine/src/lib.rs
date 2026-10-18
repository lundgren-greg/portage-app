//! Index, policy, planner, and executor for Portage.
//!
//! PR 4: local metadata index ([`index`]). Hashing is PR 5; policy, planner,
//! and executor follow the PR Plan in `docs/design.md`.

pub mod index;

pub use portage_core as core;
