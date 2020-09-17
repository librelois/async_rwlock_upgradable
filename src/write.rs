// Copyright 2020 Éloïs SANCHEZ
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::*;
use read::ReadGuard;

pub struct WriteFuture<'a, T: ?Sized> {
    rwlock: &'a RwLock<T>,
    lock_acquired: bool,
    writer_flag_acquired: bool,
}

impl<'a, T: ?Sized> From<&'a RwLock<T>> for WriteFuture<'a, T> {
    fn from(rwlock: &'a RwLock<T>) -> Self {
        Self {
            rwlock,
            lock_acquired: false,
            writer_flag_acquired: false,
        }
    }
}

impl<'a, T: ?Sized> Future for WriteFuture<'a, T> {
    type Output = WriteGuard<'a, T>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if self.try_acquire_writer() {
            self.lock_acquired = true;
            Poll::Ready(WriteGuard(self.rwlock))
        } else {
            self.rwlock.store_waker(cx.waker());
            Poll::Pending
        }
    }
}

impl<'a, T: ?Sized> WriteFuture<'a, T> {
    fn try_acquire_writer(&mut self) -> bool {
        if self.writer_flag_acquired {
            self.rwlock.state.load(Ordering::Relaxed) & READERS_MASK == 0
        } else if let Err(state_) = self.rwlock.state.compare_exchange_weak(
            0,
            WRITER_FLAG,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            if state_ & (WRITER_FLAG | UPGRADABLE_FLAG) != 0 {
                false
            } else if self
                .rwlock
                .state
                .compare_exchange_weak(
                    state_,
                    state_ | WRITER_FLAG,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                self.writer_flag_acquired = true;
                false
            } else {
                false
            }
        } else {
            true
        }
    }
}

impl<'a, T: ?Sized> Drop for WriteFuture<'a, T> {
    fn drop(&mut self) {
        if !self.lock_acquired && self.writer_flag_acquired {
            self.rwlock.unlock_writer()
        }
    }
}

pub struct WriteGuard<'a, T: ?Sized>(pub(crate) &'a RwLock<T>);

impl<'a, T: ?Sized> WriteGuard<'a, T> {
    /// Atomically downgrades an exclusive lock into a shared lock without allowing any thread to take an exclusive lock in the meantime.
    pub fn downgrade(&mut self) -> ReadGuard<'a, T> {
        self.0.state.store(ONE_READER, Ordering::Release);
        ReadGuard(self.0)
    }
    /// Downgrades an exclusive lock to an upgradable lock.
    pub fn downgrade_to_upgradable(&mut self) -> UpgradableReadGuard<'a, T> {
        self.0
            .state
            .store(ONE_READER | UPGRADABLE_FLAG, Ordering::Release);
        UpgradableReadGuard::from(self.0)
    }
}

impl<'a, T: ?Sized> std::ops::Deref for WriteGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.0.data.get() }
    }
}

impl<'a, T: ?Sized> std::ops::DerefMut for WriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.0.data.get() }
    }
}

impl<'a, T: ?Sized> Drop for WriteGuard<'a, T> {
    fn drop(&mut self) {
        self.0.unlock_writer()
    }
}
