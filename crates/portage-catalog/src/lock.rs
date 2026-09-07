//! Exclusive / shared lock on `%data_dir%/portage.lock`.
//!
//! `portage init` creates the empty file; this module enforces it. Writers
//! (`index`, `plan`, `apply`, `resume`, migrate, backup) take exclusive.
//! Readers (`search`, `list`, `plan show`, cached `capacity`, `doctor`)
//! take shared. A contended acquire is `Error::CatalogLocked { pid }`.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use portage_core::Error;

/// How the catalog is being used this invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    /// Single writer. Blocks shared and exclusive acquirers.
    Exclusive,
    /// Concurrent readers. Blocks exclusive acquirers.
    Shared,
}

/// Held lock. Released on drop.
#[derive(Debug)]
pub struct CatalogLock {
    file: File,
    mode: LockMode,
}

impl CatalogLock {
    /// Open (creating if needed) `path` and try to acquire `mode` immediately.
    ///
    /// `lock_timeout: 0` in the design: fail at once, never wait.
    pub fn acquire(path: &Path, mode: LockMode) -> Result<Self, Error> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?;

        let acquired = match mode {
            LockMode::Exclusive => file.try_lock(),
            LockMode::Shared => file.try_lock_shared(),
        };

        match acquired {
            Ok(()) => {
                if mode == LockMode::Exclusive {
                    write_pid(&mut file, path)?;
                }
                tracing::debug!(
                    path = %path.display(),
                    ?mode,
                    pid = std::process::id(),
                    "catalog lock acquired"
                );
                Ok(Self { file, mode })
            }
            Err(TryLockError::WouldBlock) => {
                let pid = read_pid(&mut file);
                Err(Error::CatalogLocked { pid })
            }
            Err(TryLockError::Error(source)) => Err(Error::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Which mode this guard holds.
    pub fn mode(&self) -> LockMode {
        self.mode
    }
}

impl Drop for CatalogLock {
    fn drop(&mut self) {
        if self.mode == LockMode::Exclusive {
            // Clear PID so a subsequent shared holder doesn't report a stale
            // exclusive-owner PID when a new exclusive acquire blocks.
            let _ = self.file.set_len(0);
        }
        let _ = self.file.unlock();
    }
}

fn write_pid(file: &mut File, path: &Path) -> Result<(), Error> {
    file.set_len(0).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.seek(SeekFrom::Start(0)).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    write!(file, "{}", std::process::id()).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.flush().map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn read_pid(file: &mut File) -> u32 {
    let _ = file.seek(SeekFrom::Start(0));
    let mut buf = String::new();
    if file.read_to_string(&mut buf).is_err() {
        return 0;
    }
    buf.trim().parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn exclusive_blocks_exclusive_and_shared() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("portage.lock");
        let held = CatalogLock::acquire(&path, LockMode::Exclusive).unwrap();
        assert_eq!(held.mode(), LockMode::Exclusive);

        match CatalogLock::acquire(&path, LockMode::Exclusive) {
            Err(Error::CatalogLocked { pid }) => {
                // Windows exclusive locks can hide the pid file; 0 means unknown.
                assert!(pid == 0 || pid == std::process::id(), "pid={pid}");
            }
            other => panic!("expected CatalogLocked, got {other:?}"),
        }
        match CatalogLock::acquire(&path, LockMode::Shared) {
            Err(Error::CatalogLocked { pid }) => {
                assert!(pid == 0 || pid == std::process::id(), "pid={pid}");
            }
            other => panic!("expected CatalogLocked, got {other:?}"),
        }
        drop(held);
        CatalogLock::acquire(&path, LockMode::Exclusive).unwrap();
    }

    #[test]
    fn shared_allows_shared_blocks_exclusive() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("portage.lock");
        let a = CatalogLock::acquire(&path, LockMode::Shared).unwrap();
        let b = CatalogLock::acquire(&path, LockMode::Shared).unwrap();
        assert!(matches!(
            CatalogLock::acquire(&path, LockMode::Exclusive),
            Err(Error::CatalogLocked { .. })
        ));
        drop(a);
        drop(b);
        CatalogLock::acquire(&path, LockMode::Exclusive).unwrap();
    }
}
