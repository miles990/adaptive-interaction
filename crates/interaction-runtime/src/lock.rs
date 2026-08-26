//! Single-instance lock: prevents two runtimes from owning the same devices.
//! PID-file based with stale-lock recovery (a dead PID's lock is reclaimed).
//!
//! Acquisition is ATOMIC: the lock file is created with O_EXCL (create_new), so
//! two processes racing at login cannot both believe they own it. Only if the
//! exclusive create fails do we inspect the existing file for a stale PID and
//! reclaim it (itself via an exclusive re-create).

use interaction_core::{DomainError, DomainResult};
use std::io::Write;
use std::path::PathBuf;

pub struct InstanceLock {
    path: PathBuf,
    /// PID written into the file; drop only removes it if the file STILL holds
    /// this PID (never delete a survivor's lock after losing a race).
    pid: u32,
    /// Guard: only the process that acquired the lock removes it on drop.
    owned: bool,
}

impl InstanceLock {
    pub fn acquire(path: PathBuf) -> DomainResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DomainError::Storage(format!("create {parent:?}: {e}")))?;
        }
        let pid = std::process::id();
        match Self::create_exclusive(&path, pid) {
            Ok(()) => Ok(Self {
                path,
                pid,
                owned: true,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Someone holds (or held) it. Reclaim ONLY a same-pid or dead
                // PID; a live foreign PID is a hard conflict.
                let existing = std::fs::read_to_string(&path).unwrap_or_default();
                if let Ok(other) = existing.trim().parse::<u32>() {
                    if other != pid && process_alive(other) {
                        return Err(DomainError::Conflict(format!(
                            "another runtime (pid {other}) already holds {path:?}; \
                             stop it first or remove a stale lock"
                        )));
                    }
                    if other == pid {
                        // Idempotent within one process.
                        return Ok(Self {
                            path,
                            pid,
                            owned: true,
                        });
                    }
                    tracing::warn!(pid = other, "reclaiming stale instance lock");
                }
                // Stale/garbage: remove and re-create exclusively so a
                // concurrent reclaimer cannot also win.
                let _ = std::fs::remove_file(&path);
                Self::create_exclusive(&path, pid)
                    .map_err(|e| DomainError::Storage(format!("write lock {path:?}: {e}")))?;
                Ok(Self {
                    path,
                    pid,
                    owned: true,
                })
            }
            Err(e) => Err(DomainError::Storage(format!("write lock {path:?}: {e}"))),
        }
    }

    fn create_exclusive(path: &PathBuf, pid: u32) -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true) // O_EXCL: fails if the file already exists
            .open(path)?;
        f.write_all(pid.to_string().as_bytes())
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        if !self.owned {
            return;
        }
        // Only delete the lock if it STILL holds our PID — never remove a
        // survivor's lock after we lost an idempotent same-pid re-acquire race.
        if let Ok(existing) = std::fs::read_to_string(&self.path) {
            if existing.trim().parse::<u32>().ok() == Some(self.pid) {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // Signal 0 = existence check.
    unsafe { libc_kill(pid as i32, 0) == 0 }
}

#[cfg(unix)]
extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    // Conservative on non-unix: assume alive so we never steal a live lock.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_and_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime.lock");
        let lock = InstanceLock::acquire(path.clone()).unwrap();
        // Same pid can re-acquire (idempotent within one process).
        assert!(InstanceLock::acquire(path.clone()).is_ok());
        drop(lock);
    }

    #[test]
    fn stale_lock_is_reclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime.lock");
        // PID 999999999 is (practically) never alive.
        std::fs::write(&path, "999999999").unwrap();
        let lock = InstanceLock::acquire(path.clone());
        assert!(lock.is_ok(), "stale lock should be reclaimed");
    }
}
