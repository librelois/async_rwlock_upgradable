// Copyright 2020 Éloïs SANCHEZ
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::*;
use read::ReadGuard;
use upgrade::UpgradeFuture;

pub struct UpgradableReadFuture<'a, T: ?Sized> {
    rwlock: &'a RwLock<T>,
    lock_acquired: bool,
}

impl<'a, T: ?Sized> From<&'a RwLock<T>> for UpgradableReadFuture<'a, T> {
    fn from(rwlock: &'a RwLock<T>) -> Self {
        Self {
            rwlock,
            lock_acquired: false,
        }
    }
}

impl<'a, T: ?Sized> Future for UpgradableReadFuture<'a, T> {
    type Output = UpgradableReadGuard<'a, T>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if self.try_acquire_upgradable_reader() {
            self.lock_acquired = true;
            Poll::Ready(UpgradableReadGuard {
                rwlock: self.rwlock,
                upgrade_called: false,
            })
        } else {
            self.rwlock.store_waker(cx.waker());
            Poll::Pending
        }
    }
}

impl<'a, T: ?Sized> UpgradableReadFuture<'a, T> {
    fn try_acquire_upgradable_reader(&self) -> bool {
        let state_ = self.rwlock.state.load(Ordering::Relaxed);
        if state_ & (WRITER_FLAG | UPGRADABLE_FLAG) == 0 {
            let new_state = state_
                .checked_add(ONE_READER | UPGRADABLE_FLAG)
                .expect("RwLock reader count overflow");
            self.rwlock
                .state
                .compare_exchange_weak(state_, new_state, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
        } else {
            false
        }
    }
}

pub struct UpgradableReadGuard<'a, T: ?Sized> {
    rwlock: &'a RwLock<T>,
    upgrade_called: bool,
}

impl<'a, T: ?Sized> From<&'a RwLock<T>> for UpgradableReadGuard<'a, T> {
    fn from(rwlock: &'a RwLock<T>) -> Self {
        Self {
            rwlock,
            upgrade_called: false,
        }
    }
}

impl<'a, T: ?Sized> UpgradableReadGuard<'a, T> {
    /// Downgrades an upgradable lock to a shared lock.
    pub fn downgrade_upgradable(&self) -> ReadGuard<'a, T> {
        self.rwlock
            .state
            .fetch_sub(UPGRADABLE_FLAG, Ordering::Release);
        ReadGuard(self.rwlock)
    }
    /// Upgrades an upgradable lock to an exclusive lock.
    pub fn upgrade(mut self) -> UpgradeFuture<'a, T> {
        self.upgrade_called = true;
        UpgradeFuture::from(self.rwlock)
    }
}

impl<'a, T: ?Sized> std::ops::Deref for UpgradableReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.rwlock.data.get() }
    }
}

impl<'a, T: ?Sized> Drop for UpgradableReadGuard<'a, T> {
    fn drop(&mut self) {
        if !self.upgrade_called {
            self.rwlock.unlock_upgradable_reader();
        }
    }
}
