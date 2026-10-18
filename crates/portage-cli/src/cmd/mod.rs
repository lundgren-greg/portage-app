//! CLI subcommands. PR 1 ships `init`; PR 1.5 adds `status`; PR 3 adds
//! `doctor`; PR 4 adds `provider add local`, `index`, and `capacity`.

pub mod capacity;
pub mod doctor;
pub mod index;
pub mod init;
pub mod provider;
pub mod status;
