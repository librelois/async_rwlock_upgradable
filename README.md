# async_rwlock_upgradable

It is a lib which provide asynchronous RwLock implementation with the capability to atomically upgrade a reader to a writer.

The algorithm used is very strongly inspired from `parking-lot` but completely redesigned to be efficient in an asynchronous context.
This implementation it not vulnerable to write starvation, readers will block if there is a pending writer.

It's maybe very efficient because when mutex tries to acquire data unsuccessfully, these returning control to an async runtime back.
This lib built only on atomics and don't use others std synchronous data structures, which make this lib so fast.

## Example

```rust
use async_rwlock_upgradable::RwLock;

#[tokio::main]
async fn main() {
    let mutex = RwLock::new(42);
    let guard = mutex.read().await;
    assert_eq!(*guard, 42);
}
```
