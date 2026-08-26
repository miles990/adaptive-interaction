//! Single-instance lock: prevents two runtimes from owning the same devices.
//! PID-file based with stale-lock recovery (a dead PID's lock is reclaimed).

use interaction_core::{DomainError, DomainResult};
use std::path::PathBuf;

pub struct InstanceLock {
    path: PathBuf,
    /// Guard: only the process that acquired the lock removes it on drop.
    owned: bool,
}

impl InstanceLock {
    pub fn acquire(path: PathBuf) -> DomainResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DomainError::Storage(format!("create {parent:?}: {e}")))?;
        }
        if let Ok(existing) = std::fs::read_to_string(&path) {
            if let Ok(pid) = existing.trim().parse::<u32>() {
                if pid != std::process::id() && process_alive(pid) {
                    return Err(DomainError::Conflict(format!(
                        "another runtime (pid {pid}) already holds {path:?}; \
                         stop it first or remove a stale lock"
                    )));
                }
                tracing::warn!(pid, "reclaiming stale instance lock");
            }
        }
        std::fs::write(&path, std::process::id().to_string())
            .map_err(|e| DomainError::Storage(format!("write lock {path:?}: {e}")))?;
        Ok(Self { path, owned: true })
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        if self.owned {
            let _ = std::fs::remove_file(&self.path);
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
