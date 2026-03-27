//! A lock held across processes while the credential file is rewritten.
//!
//! Two micro processes can want to write a credential at the same moment: a session
//! refreshing a token that is about to expire while `micro auth login` stores a new one.
//! Each would otherwise write a whole map built from the read it did on startup, and
//! whichever finished last would erase the other's work without either noticing.
//!
//! The lock is a file created exclusively beside the credential file. Creating it is the
//! atomic step, so exactly one process holds it; dropping the guard removes it. A lock
//! left behind by a process that died is broken once it is old enough to be certain
//! nobody is still working under it.

use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

/// How long a lock may stand before it is taken to belong to a process that died.
const STALE_AFTER: Duration = Duration::from_secs(30);
/// How long to wait for the holder before giving up and saying so.
const WAIT_LIMIT: Duration = Duration::from_secs(10);
/// How long to wait between attempts.
const RETRY_DELAY: Duration = Duration::from_millis(25);

/// A held lock. Dropping it lets the next process in.
pub struct FileLock {
    path: PathBuf,
}

impl FileLock {
    /// Take the lock guarding `target`, waiting for whoever holds it.
    pub fn acquire(target: &Path) -> io::Result<Self> {
        let path = lock_path(target);
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }

        let started = Instant::now();
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(FileLock { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    // Someone holds it. Break it if it is old enough to be abandoned,
                    // otherwise wait and look again.
                    if broke_stale_lock(&path) {
                        continue;
                    }
                    if started.elapsed() >= WAIT_LIMIT {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!(
                                "{} is being written by another micro and did not free up",
                                target.display()
                            ),
                        ));
                    }
                    sleep(RETRY_DELAY);
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Remove a lock old enough that whoever made it is gone. Says whether it removed one.
fn broke_stale_lock(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        // It went away between the failed create and this look, which is the holder
        // releasing it. Try again.
        return true;
    };
    let held_for = metadata
        .modified()
        .ok()
        .and_then(|written| SystemTime::now().duration_since(written).ok());
    match held_for {
        Some(held_for) if held_for >= STALE_AFTER => fs::remove_file(path).is_ok(),
        _ => false,
    }
}

fn lock_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "auth.json".to_string());
    let directory = target.parent().unwrap_or_else(|| Path::new("."));
    directory.join(format!(".{name}.lock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-lockfile-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("auth.json")
    }

    #[test]
    fn the_lock_is_released_when_it_is_dropped() {
        let target = scratch("release");
        let held = FileLock::acquire(&target).expect("first take");
        assert!(lock_path(&target).exists());
        drop(held);
        assert!(!lock_path(&target).exists());
        FileLock::acquire(&target).expect("and it can be taken again");
    }

    #[test]
    fn a_second_holder_waits_rather_than_walking_in() {
        let target = scratch("contended");
        let held = FileLock::acquire(&target).expect("first take");

        // The holder frees it shortly; the waiter should get it rather than fail.
        let releasing = std::thread::spawn(move || {
            sleep(Duration::from_millis(120));
            drop(held);
        });

        let waited = Instant::now();
        let second = FileLock::acquire(&target).expect("the waiter gets in");
        assert!(waited.elapsed() >= Duration::from_millis(100), "it waited");
        drop(second);
        releasing.join().unwrap();
    }

    #[test]
    fn a_lock_left_by_a_dead_process_is_broken() {
        let target = scratch("stale");
        let path = lock_path(&target);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "").unwrap();

        // Backdate it well past the point where a holder could still be working.
        let old = SystemTime::now() - STALE_AFTER - Duration::from_secs(5);
        fs::File::open(&path)
            .unwrap()
            .set_modified(old)
            .expect("backdate the lock");

        FileLock::acquire(&target).expect("the stale lock is broken rather than waited on");
    }
}
