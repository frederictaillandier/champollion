//! Keeps the game closed while too many cards are due: playing waits for
//! studying.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use champollion_api::DueCount;

/// Time between two questions to the backend: reviews made on the phone
/// show up this late at most.
const POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Asks the backend, in a thread, how many cards are due.
pub struct DueCards {
    /// `None` until the backend answers, and while it does not.
    pub due: Arc<Mutex<Option<u64>>>,
    handle: thread::JoinHandle<()>,
}

impl DueCards {
    /// `backend_url` is e.g. `http://10.0.0.1:8090`.
    pub fn spawn(backend_url: &str, stop: Arc<AtomicBool>) -> Self {
        let url = format!("{backend_url}/cards/due/count");
        let due = Arc::new(Mutex::new(None));
        let shared = Arc::clone(&due);
        let handle = thread::Builder::new()
            .name("due-cards".into())
            .spawn(move || poll(&url, &shared, &stop))
            .expect("spawn due cards thread");
        Self { due, handle }
    }

    pub fn join(self) {
        let _ = self.handle.join();
    }
}

fn poll(url: &str, due: &Mutex<Option<u64>>, stop: &AtomicBool) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut failing = false;
    while !stop.load(Ordering::Relaxed) {
        let answer = agent
            .get(url)
            .call()
            .and_then(|mut r| r.body_mut().read_json::<DueCount>());
        match answer {
            Ok(count) => {
                if failing {
                    tracing::info!("the backend tells how many cards are due again");
                    failing = false;
                }
                *due.lock().unwrap() = Some(count.due);
            }
            Err(e) => {
                if !failing {
                    tracing::warn!(
                        "could not ask how many cards are due, the game is not locked: {e}"
                    );
                    failing = true;
                }
                *due.lock().unwrap() = None;
            }
        }
        let mut slept = Duration::ZERO;
        while slept < POLL_INTERVAL && !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(500));
            slept += Duration::from_millis(500);
        }
    }
}

/// Locks the game once `lock_at` cards are due, until fewer than
/// `unlock_below` are: unlocking right under `lock_at` would lock it again a
/// few minutes into the game.
pub struct Lock {
    lock_at: u64,
    unlock_below: u64,
    /// Exists while locked, so restarting the daemon does not unlock.
    file: PathBuf,
    locked: bool,
}

impl Lock {
    /// `lock_at` 0 never locks.
    pub fn load(lock_at: u64, unlock_below: u64, file: PathBuf) -> Self {
        let locked = lock_at > 0 && file.exists();
        Self {
            lock_at,
            unlock_below: unlock_below.min(lock_at),
            file,
            locked,
        }
    }

    pub fn unlock_below(&self) -> u64 {
        self.unlock_below
    }

    /// Takes the number of cards due now, if known, and returns whether the
    /// game must not run. When the backend cannot be reached the game is
    /// let run, rather than blocked by a network problem.
    pub fn update(&mut self, due: Option<u64>) -> bool {
        if self.lock_at == 0 {
            return false;
        }
        let Some(due) = due else {
            return false;
        };
        if !self.locked && due >= self.lock_at {
            tracing::info!("{due} cards are due, locking the game");
            self.locked = true;
            let result = self
                .file
                .parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|()| fs::write(&self.file, format!("{due}\n")));
            if let Err(e) = result {
                tracing::warn!("could not write {}: {e}", self.file.display());
            }
        } else if self.locked && due < self.unlock_below {
            tracing::info!("only {due} cards are due, unlocking the game");
            self.locked = false;
            if let Err(e) = fs::remove_file(&self.file) {
                tracing::warn!("could not remove {}: {e}", self.file.display());
            }
        }
        self.locked
    }
}

/// Shows a desktop notification that stays until dismissed.
pub async fn notify(summary: &str, body: &str) -> zbus::Result<()> {
    let connection = zbus::Connection::session().await?;
    let hints: HashMap<&str, zbus::zvariant::Value> =
        HashMap::from([("urgency", zbus::zvariant::Value::U8(2))]);
    connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "Champollion",
                0u32,
                "",
                summary,
                body,
                Vec::<&str>::new(),
                hints,
                -1i32,
            ),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_and_unlocks_with_a_gap() {
        let file = std::env::temp_dir().join(format!("champollion-lock-{}", std::process::id()));
        let mut lock = Lock::load(50, 10, file.clone());
        assert!(!lock.update(Some(49)));
        assert!(lock.update(Some(50)));
        assert!(file.exists());
        // Still locked between the two thresholds, and after a restart.
        assert!(lock.update(Some(20)));
        let mut lock = Lock::load(50, 10, file.clone());
        assert!(lock.update(Some(10)));
        // An unreachable backend does not block the game, nor unlock it.
        assert!(!lock.update(None));
        assert!(lock.update(Some(30)));
        assert!(!lock.update(Some(9)));
        assert!(!file.exists());
        assert!(!lock.update(Some(30)));
    }

    #[test]
    fn never_locks_at_zero() {
        let file = std::env::temp_dir().join(format!("champollion-nolock-{}", std::process::id()));
        let mut lock = Lock::load(0, 10, file.clone());
        assert!(!lock.update(Some(1000)));
        assert!(!file.exists());
    }
}
