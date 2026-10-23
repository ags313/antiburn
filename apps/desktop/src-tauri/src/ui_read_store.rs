//! The Overview's pool of read-only store handles.
//!
//! Each handle owns one SQLite connection behind one mutex, so a single
//! handle runs one read at a time. A pool lets a slow read, such as the
//! allowance scan over raw turn rows, run beside the fast ones.

use std::ops::Deref;
use std::sync::{Condvar, Mutex, MutexGuard};

use crate::store::Store;

/// Number of reader connections the Overview opens.
pub(crate) const UI_READ_POOL_SIZE: usize = 4;

pub(crate) struct UiReadStore {
    idle: Mutex<Vec<Store>>,
    returned: Condvar,
    /// A handle that shares the settings snapshot and other lock-free state
    /// with every pooled handle.
    shared: Store,
}

/// A pooled handle on loan. It returns to the pool on drop.
pub(crate) struct UiReadLease<'a> {
    pool: &'a UiReadStore,
    store: Option<Store>,
}

impl UiReadStore {
    /// Builds a pool of `size` readers from `open`. Stops at the first reader
    /// that cannot open. Falls back to `fallback` as the only handle when no
    /// reader opens, so a failed open still leaves the app usable.
    pub(crate) fn open<E>(
        size: usize,
        fallback: Store,
        mut open: impl FnMut() -> Result<Store, E>,
    ) -> (Self, Option<E>) {
        let mut stores = Vec::with_capacity(size);
        let mut failure = None;
        for _ in 0..size {
            match open() {
                Ok(store) => stores.push(store),
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        if stores.is_empty() {
            stores.push(fallback);
        }
        (Self::from_stores(stores), failure)
    }

    pub(crate) fn from_stores(stores: Vec<Store>) -> Self {
        let shared = stores
            .first()
            .cloned()
            .expect("a read pool holds at least one store");
        Self {
            idle: Mutex::new(stores),
            returned: Condvar::new(),
            shared,
        }
    }

    /// Borrows an idle handle and waits when all handles are in use.
    pub(crate) fn checkout(&self) -> UiReadLease<'_> {
        let mut idle = self.lock_idle();
        loop {
            if let Some(store) = idle.pop() {
                return UiReadLease {
                    pool: self,
                    store: Some(store),
                };
            }
            idle = self
                .returned
                .wait(idle)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// A handle for state that needs no connection, such as the settings
    /// snapshot. It never waits on a read.
    pub(crate) fn shared(&self) -> &Store {
        &self.shared
    }

    fn lock_idle(&self) -> MutexGuard<'_, Vec<Store>> {
        self.idle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Deref for UiReadLease<'_> {
    type Target = Store;

    fn deref(&self) -> &Store {
        self.store.as_ref().expect("a lease holds its store")
    }
}

impl Drop for UiReadLease<'_> {
    fn drop(&mut self) {
        if let Some(store) = self.store.take() {
            self.pool.lock_idle().push(store);
            self.pool.returned.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    fn pool(size: usize) -> (tempfile::TempDir, UiReadStore) {
        let directory = tempfile::tempdir().expect("creates a temp dir");
        let writer = Store::open(directory.path()).expect("opens a store");
        let (pool, failure) = UiReadStore::open(size, writer.clone(), || {
            writer.open_reader(Duration::from_millis(100))
        });
        assert!(failure.is_none(), "every reader opens");
        (directory, pool)
    }

    /// One reader stays inside a long read until the test releases it, so no
    /// timing bound is involved.
    #[test]
    fn a_read_completes_while_another_reader_is_held() {
        let (_directory, pool) = pool(2);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel();

        let pool_ref = &pool;
        thread::scope(|scope| {
            scope.spawn(move || {
                let slow = pool_ref.checkout();
                let _connection = slow.lock();
                ready_tx.send(()).expect("signals the slow read started");
                let _ = release_rx.recv_timeout(Duration::from_secs(5));
            });
            ready_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("the slow reader is held");

            scope.spawn(|| {
                let fast = pool.checkout();
                done_tx
                    .send(fast.settings().is_ok())
                    .expect("reports the fast read");
            });
            let finished = done_rx.recv_timeout(Duration::from_secs(2));
            release_tx.send(()).expect("releases the slow reader");
            assert_eq!(finished, Ok(true), "the fast read waited on the slow one");
        });
    }

    #[test]
    fn checkout_waits_until_a_handle_returns() {
        let (_directory, pool) = pool(1);
        let held = pool.checkout();
        thread::scope(|scope| {
            let waiting = scope.spawn(|| pool.checkout().settings().is_ok());
            thread::sleep(Duration::from_millis(50));
            assert!(!waiting.is_finished(), "the only handle is on loan");
            drop(held);
            assert!(waiting.join().expect("the waiter finishes"));
        });
    }

    #[test]
    fn falls_back_to_the_fallback_store_when_no_reader_opens() {
        let directory = tempfile::tempdir().expect("creates a temp dir");
        let writer = Store::open(directory.path()).expect("opens a store");
        let (pool, failure) = UiReadStore::open(4, writer, || Err::<Store, _>("cannot open"));
        assert_eq!(failure, Some("cannot open"));
        assert!(pool.checkout().settings().is_ok());
    }
}
