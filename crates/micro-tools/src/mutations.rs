//! One writer at a time, per file.
//!
//! The calls in one answer run together, so two of them can reach the same file at once.
//! Each reads the whole file, changes its part, and writes the whole file back, so
//! whichever writes last erases the other's work — and both report success, because
//! neither did anything wrong on its own.
//!
//! A lock per path serializes them. Two edits to different files still run together,
//! which is the point of running them together at all.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;

/// The locks handed out so far, one per path.
///
/// Held for the process rather than per tool, because two tools rooted at the same
/// workspace are still two ways to reach one file.
fn locks() -> &'static Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(Default::default)
}

/// Wait for this file to be nobody else's, and hold it until the guard is dropped.
pub async fn hold(path: &Path) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = {
        let mut held = locks().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Arc::clone(held.entry(path.to_path_buf()).or_default())
    };
    lock.lock_owned().await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two writers on one path take turns, so the second reads what the first wrote
    /// rather than the state they both started from.
    #[tokio::test]
    async fn one_writer_at_a_time_per_file() {
        let path = std::env::temp_dir().join("micro-mutations-shared.txt");
        let counter = Arc::new(tokio::sync::Mutex::new(0u32));
        let mut running = Vec::new();

        for _ in 0..8 {
            let path = path.clone();
            let counter = Arc::clone(&counter);
            running.push(tokio::spawn(async move {
                let _held = hold(&path).await;
                // Read, pause, write: the pause is where a lost update would happen.
                let seen = *counter.lock().await;
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                *counter.lock().await = seen + 1;
            }));
        }

        for task in running {
            task.await.unwrap();
        }
        assert_eq!(*counter.lock().await, 8, "no write was lost");
    }

    /// Different files are not held up by each other.
    #[tokio::test]
    async fn different_files_do_not_wait_on_each_other() {
        let first = hold(Path::new("/tmp/micro-mutations-a")).await;
        // Would deadlock if the lock were shared rather than per path.
        let second = hold(Path::new("/tmp/micro-mutations-b")).await;
        drop((first, second));
    }
}
