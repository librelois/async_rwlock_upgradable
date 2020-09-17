// Copyright 2020 Éloïs SANCHEZ
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! Asynchronous RwLock implementation with the capability to atomically upgrade a reader to a writer.
//! The algorithm used is very strongly inspired from `parking-lot` but completely redesigned to be efficient in an asynchronous context.
//! This implementation it not vulnerable to write starvation, readers will block if there is a pending writer.

#![deny(
    clippy::unwrap_used,
    missing_copy_implementations,
    trivial_casts,
    trivial_numeric_casts,
    unstable_features,
    unused_import_braces,
    unused_qualifications
)]

mod read;
mod upgradable_read;
mod upgrade;
mod write;

pub use read::ReadGuard;
pub use upgradable_read::UpgradableReadGuard;
pub use write::WriteGuard;

use read::ReadFuture;
use std::{
    cell::UnsafeCell, future::Future, ptr::null_mut, sync::atomic::AtomicPtr,
    sync::atomic::AtomicUsize, sync::atomic::Ordering, task::Poll, task::Waker,
};
use upgradable_read::UpgradableReadFuture;
use write::WriteFuture;

// A reader is holding an upgradable lock. The reader count must be non-zero and
// WRITER_FLAG must not be set.
const UPGRADABLE_FLAG: usize = 0b0001;
// If the reader count is zero: a writer is currently holding an exclusive lock.
// Otherwise: a writer is waiting for the remaining readers to exit the lock.
const WRITER_FLAG: usize = 0b0010;
// Mask of bits used to count readers.
const READERS_MASK: usize = !0b0011;
// Base unit for counting readers.
const ONE_READER: usize = 0b0100;

/// A reader-writer lock
///
/// This type of lock allows a number of readers or at most one writer at any
/// point in time. The write portion of this lock typically allows modification
/// of the underlying data (exclusive access) and the read portion of this lock
/// typically allows for read-only access (shared access).
///
/// The type parameter `T` represents the data that this lock protects. It is
/// required that `T` satisfies `Send` to be shared across threads and `Sync` to
/// allow concurrent access through readers. The RAII guards returned from the
/// locking methods implement `Deref` (and `DerefMut` for the `write` methods)
/// to allow access to the contained of the lock.
pub struct RwLock<T: ?Sized> {
    pub(crate) state: AtomicUsize,
    waker: AtomicPtr<Waker>,
    data: UnsafeCell<T>,
}

impl<T> RwLock<T> {
    /// Creates a new instance of an `RwLock<T>` which is unlocked.
    pub const fn new(data: T) -> Self {
        RwLock {
            state: AtomicUsize::new(0),
            waker: AtomicPtr::new(null_mut()),
            data: UnsafeCell::new(data),
        }
    }

    /// Consumes this `RwLock`, returning the underlying data.
    #[inline]
    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }
}

impl<T: ?Sized> RwLock<T> {
    /// Return a future that attempts to acquire this RwLock with shared read access.
    ///
    /// The future will be pending until there are no more writers which hold the lock.
    /// There may be other readers currently inside the lock when the future acquired the lock.
    ///
    /// Returns an RAII guard which will release this shared access once it is dropped.
    #[inline]
    pub fn read(&self) -> ReadFuture<'_, T> {
        ReadFuture::from(self)
    }
    /// Return a future that attempts to locks this RwLock with upgradable read access.
    ///
    /// The future will be pending until there are no more writers or other upgradable reads which hold the lock.
    /// There may be other readers currently inside the lock when the future acquired the lock.
    ///
    /// Note that attempts to recursively acquire an upgradable lock on a RwLock when the current task already holds one may result in a deadlock.
    ///
    /// Returns an RAII guard which will release this shared access once it is dropped.
    #[inline]
    pub fn upgradable_read(&self) -> UpgradableReadFuture<'_, T> {
        UpgradableReadFuture::from(self)
    }
    /// Return a future that attempts to lock this `RwLock` with exclusive write access.
    ///
    /// The future will be pending while other writers or other readers
    /// currently have access to the lock.
    ///
    /// Returns an RAII guard which will drop the write access of this `RwLock`
    /// when dropped.
    #[inline]
    pub fn write(&self) -> WriteFuture<'_, T> {
        WriteFuture::from(self)
    }
    #[inline]
    pub(crate) fn store_waker(&self, waker: &Waker) {
        let _ = self.waker.compare_exchange_weak(
            null_mut(),
            Box::into_raw(Box::new(waker.clone())),
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
    }
    #[inline]
    pub(crate) fn try_wake(&self, waker_ptr: *mut Waker) {
        let waker_ptr = self.waker.swap(waker_ptr, Ordering::AcqRel);

        if !waker_ptr.is_null() {
            unsafe { Box::from_raw(waker_ptr).wake() }
        }
    }
    #[inline]
    pub(crate) fn unlock_reader(&self) {
        let state_ = self.state.fetch_sub(ONE_READER, Ordering::Release);
        if state_ & READERS_MASK == 0 {
            self.try_wake(null_mut())
        }
    }
    #[inline]
    pub(crate) fn unlock_upgradable_reader(&self) {
        self.state
            .fetch_sub(ONE_READER | UPGRADABLE_FLAG, Ordering::Release);
        self.try_wake(null_mut())
    }
    #[inline]
    pub(crate) fn unlock_writer(&self) {
        self.state.fetch_sub(WRITER_FLAG, Ordering::Release);
        self.try_wake(null_mut())
    }
}

unsafe impl<T> Send for RwLock<T> where T: Send + ?Sized {}
unsafe impl<T> Sync for RwLock<T> where T: Send + Sync + ?Sized {}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use futures::{StreamExt, TryStreamExt};
    use std::ops::AddAssign;
    use std::sync::Arc;
    use tokio::time::Duration;

    #[tokio::test(core_threads = 12)]
    async fn test_mutex() {
        let c = RwLock::new(0);

        futures::stream::iter(0..10000)
            .for_each_concurrent(None, |_| async {
                let mut co: WriteGuard<i32> = c.write().await;
                *co += 1;
            })
            .await;

        let co = c.write().await;
        assert_eq!(*co, 10000)
    }

    #[tokio::test(core_threads = 12)]
    async fn test_container() {
        let c = RwLock::new(String::from("lol"));

        let mut co: WriteGuard<String> = c.write().await;
        co.add_assign("lol");

        assert_eq!(*co, "lollol");
    }

    #[tokio::test(core_threads = 12)]
    async fn test_timeout() {
        let c = RwLock::new(String::from("lol"));

        let co: WriteGuard<String> = c.write().await;

        futures::stream::iter(0..10000i32)
            .then(|_| tokio::time::timeout(Duration::from_nanos(1), c.write()))
            .try_for_each_concurrent(None, |_c| futures::future::ok(()))
            .await
            .expect_err("timout must be");

        drop(co);

        let mut co: WriteGuard<String> = c.write().await;
        co.add_assign("lol");

        assert_eq!(*co, "lollol");
    }

    #[tokio::test(core_threads = 12)]
    async fn test_concurrent_reading() {
        let c = RwLock::new(String::from("lol"));

        let co: ReadGuard<String> = c.read().await;

        futures::stream::iter(0..10000i32)
            .then(|_| c.read())
            .inspect(|c| assert_eq!(*co, **c))
            .for_each_concurrent(None, |_c| futures::future::ready(()))
            .await;

        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(1), c.write()).await,
            Err(_)
        ));

        let co2: ReadGuard<String> = c.read().await;
        assert_eq!(*co, *co2);
    }

    #[tokio::test(core_threads = 12)]
    async fn test_concurrent_reading_writing() {
        let c = RwLock::new(String::from("lol"));

        let co: ReadGuard<String> = c.read().await;
        let co2: ReadGuard<String> = c.read().await;
        assert_eq!(*co, *co2);

        drop(co);
        drop(co2);

        let mut co: WriteGuard<String> = c.write().await;

        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(1), c.read()).await,
            Err(_)
        ));

        *co += "lol";

        drop(co);

        let co: ReadGuard<String> = c.read().await;
        let co2: ReadGuard<String> = c.read().await;
        assert_eq!(*co, "lollol");
        assert_eq!(*co, *co2);
    }

    #[test]
    fn multithreading_test() {
        let num = 100;
        let mutex = Arc::new(RwLock::new(0));
        let ths: Vec<_> = (0..num)
            .map(|i| {
                let mutex = mutex.clone();
                std::thread::spawn(move || {
                    block_on(async {
                        if i % 2 == 0 {
                            let mut lock = mutex.write().await;
                            *lock += 1;
                            drop(lock)
                        } else {
                            let lock1 = mutex.read().await;
                            let lock2 = mutex.read().await;
                            assert_eq!(*lock1, *lock2);
                            drop(lock1);
                            drop(lock2);
                        }
                    })
                })
            })
            .collect();

        for thread in ths {
            thread.join().expect("unreachable");
        }

        block_on(async {
            let lock = mutex.read().await;
            assert_eq!(num / 2, *lock)
        })
    }
}
