// Copyright 2020 Éloïs SANCHEZ
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::*;
use write::WriteGuard;

pub struct UpgradeFuture<'a, T: ?Sized> {
    rwlock: &'a RwLock<T>,
    lock_acquired: bool,
    writer_flag_acquired: bool,
}

impl<'a, T: ?Sized> From<&'a RwLock<T>> for UpgradeFuture<'a, T> {
    fn from(rwlock: &'a RwLock<T>) -> Self {
        Self {
            rwlock,
            lock_acquired: false,
            writer_flag_acquired: false,
        }
    }
}

impl<'a, T: ?Sized> Future for UpgradeFuture<'a, T> {
    type Output = WriteGuard<'a, T>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if self.try_upgrade() {
            self.lock_acquired = true;
            Poll::Ready(WriteGuard(self.rwlock))
        } else {
            self.rwlock.store_waker(cx.waker());
            Poll::Pending
        }
    }
}

impl<'a, T: ?Sized> UpgradeFuture<'a, T> {
    fn try_upgrade(&mut self) -> bool {
        if self.writer_flag_acquired {
            self.rwlock.state.load(Ordering::Relaxed) & READERS_MASK == 0
        } else if let Err(state_) = self.rwlock.state.compare_exchange_weak(
            ONE_READER | UPGRADABLE_FLAG,
            WRITER_FLAG,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            if self
                .rwlock
                .state
                .compare_exchange_weak(
                    state_,
                    state_ - (ONE_READER | UPGRADABLE_FLAG) + WRITER_FLAG,
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

impl<'a, T: ?Sized> Drop for UpgradeFuture<'a, T> {
    fn drop(&mut self) {
        if !self.lock_acquired {
            if self.writer_flag_acquired {
                self.rwlock.unlock_writer()
            } else {
                self.rwlock.unlock_upgradable_reader()
            }
        }
    }
}
