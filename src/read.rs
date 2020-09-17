// Copyright 2020 Éloïs SANCHEZ
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::*;

pub struct ReadFuture<'a, T: ?Sized> {
    rwlock: &'a RwLock<T>,
    lock_acquired: bool,
}

impl<'a, T: ?Sized> From<&'a RwLock<T>> for ReadFuture<'a, T> {
    fn from(rwlock: &'a RwLock<T>) -> Self {
        Self {
            rwlock,
            lock_acquired: false,
        }
    }
}

impl<'a, T: ?Sized> Future for ReadFuture<'a, T> {
    type Output = ReadGuard<'a, T>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if self.try_acquire_reader() {
            self.lock_acquired = true;
            Poll::Ready(ReadGuard(self.rwlock))
        } else {
            self.rwlock.store_waker(cx.waker());
            Poll::Pending
        }
    }
}

impl<'a, T: ?Sized> ReadFuture<'a, T> {
    fn try_acquire_reader(&self) -> bool {
        let state_ = self.rwlock.state.load(Ordering::Relaxed);
        if state_ & WRITER_FLAG == 0 {
            let new_state = state_
                .checked_add(ONE_READER)
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

pub struct ReadGuard<'a, T: ?Sized>(pub(crate) &'a RwLock<T>);

impl<'a, T: ?Sized> std::ops::Deref for ReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.0.data.get() }
    }
}

impl<'a, T: ?Sized> Drop for ReadGuard<'a, T> {
    fn drop(&mut self) {
        self.0.unlock_reader()
    }
}
