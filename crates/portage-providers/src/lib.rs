//! Storage providers for Portage. PR 4: local volumes (fixed and removable).
//!
//! Google Drive and OneDrive arrive in PRs 6–8. No provider exposes share-link
//! or public-ACL APIs (K13).

pub mod local;
pub mod registry;
pub mod traits;

pub use local::{HostOverlays, LocalConfig, LocalProvider, Role};
pub use traits::{Capacity, ListItem, ObjectMeta, Provider};
