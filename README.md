# Async in depth Tokio

The project's purpose is to better understand the intricasies behind tokio's async runtime, by creating a very simple KLVStream.

It was created after reading the [tokio tutorial](https://tokio.rs/tokio/tutorial) and more specifically the chapter [Async in depth](https://tokio.rs/tokio/tutorial/async).

## Use case

The use-case is to create a stream of [KLV](https://en.wikipedia.org/wiki/KLV) items retrieved from a source which sends the data partially at a time.

In our example we assume that the key-field will always be one byte, followed by the length field which again would be one byte followed by the value.
The sender to the stream will send the item again and again into the stream in chunks of `STEP` = 4 bytes.

## Steps

### 1. No Waker/Notifier

Cargo.toml dependencies
```toml
[dependencies]
futures = "0.3.28"
tokio = { version = "1.28.0", features = ["full"] }
```

main.rs
```rust
use futures::stream::{Stream, StreamExt};
use std::{sync::mpsc::{self, Receiver, Sender}, task::{Poll, Context}, pin::Pin, thread};

/// The stream contains
/// * a receiver `rx`, with which it populates its internal buffer `data`,
/// * a sender `tx`, which a "user" copies to send data to the stream,
/// * `count`, which tracks the number of times poll_next is called,
/// * `data`, the buffer to store the received data.
pub struct SimpleKLVStream {
    pub rx: Receiver<Vec<u8>>,
    pub tx: Sender<Vec<u8>>,
    pub count: usize,
    pub data: Vec<u8>
}

impl SimpleKLVStream {
    // Get the value-length (2nd byte) and add 2 to return the size of the current KLV item stored in data.
    pub fn next_item_len(&self) -> Option<usize> {
        self.data.iter().nth(1).map(|val_len| *val_len as usize + 2)
    }
}

/// impl Stream for SimpleKLVStream.
/// This will enable to use `stream.next` async method on our SimpleKLVStream, as long as we import StreamExt.
impl Stream for SimpleKLVStream {
    type Item = Vec<u8>;

    /// The first approach does nothing with the Context parameter.
    /// This as correctly stated by the tokio tutorial, [Async in depth](https://tokio.rs/tokio/tutorial/async),
    /// will hang the thread as poll_next will only be called once (when await is called).
    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.count += 1;
        // Print how many times poll_next is called.
        println!("poll_next triggered {} times", this.count);
        // First append any new_data that where sent from a tx copy to the internal buffer.
        while let Ok(mut new_data) = this.rx.try_recv() {
            this.data.append(&mut new_data);
        }

        match this.next_item_len() {
            // If we have the length-byte, and we also have the necessary value-bytes after, return the item.
            Some(len) if len >= this.data.len() => {
                let item = this.data.drain(..len).collect();
                Poll::Ready(Some(item))
            }
            // Else simply return Poll::Pending
            _ => Poll::Pending
        }
    }
}

#[tokio::main]
async fn main() {
    // Create a SimpleKLVStream object
    let (tx, rx) = mpsc::channel();
    let mut stream = SimpleKLVStream {
        rx,
        tx,
        count: 0,
        data: vec![]
    };

    // Spawn a thread which will send 4 bytes every 1 second in the stream.
    let tx = stream.tx.clone();
    tokio::spawn(async move {
        let mut count = 0;
        loop {
            count += 1;
            println!("count: {count}: sending 4 bytes");
            let data = vec![0x06, 10, 0x01, 0x01];
            tx.send(data).unwrap();
            thread::sleep(std::time::Duration::from_secs(1));
        }
    });

    // While stream.next() does not return None await for the next item and print it.
    while let Some(item) = stream.next().await {
        println!("item: {:?}", item);
    }
}
```

#### Output

```
poll_next triggered 1 times
count: 1: sending 4 bytes
count: 2: sending 4 bytes
count: 3: sending 4 bytes
count: 4: sending 4 bytes
count: 5: sending 4 bytes
count: 6: sending 4 bytes
count: 7: sending 4 bytes
count: 8: sending 4 bytes
count: 9: sending 4 bytes
...
```

#### Conclusion

As correctly stated by the tokio tutorial, [Async in depth](https://tokio.rs/tokio/tutorial/async),
not using a waker or notifier, will hang the thread as `poll_next()` will only be called once (when await is called).

Notice that `poll_next()` is called once and no more.

### 2. Waker wake after amount of time


Cargo.toml dependencies
```toml
[dependencies]
futures = "0.3.28"
tokio = { version = "1.28.0", features = ["full"] }
```

main.rs
```rust
use futures::stream::{Stream, StreamExt};
use std::{sync::mpsc::{self, Receiver, Sender}, task::{Poll, Context}, pin::Pin, thread, time::Duration};

pub struct SimpleKLVStream {
    pub rx: Receiver<Vec<u8>>,
    pub tx: Sender<Vec<u8>>,
    pub count: usize,
    pub data: Vec<u8>
}

impl SimpleKLVStream {
    pub fn next_item_len(&self) -> Option<usize> {
        self.data.iter().nth(1).map(|val_len| *val_len as usize + 2)
    }
}

impl Stream for SimpleKLVStream {
    type Item = Vec<u8>;

    /// This time we will wake the cx.waker() after 2 seconds before returning Poll::Pending.
    /// Notice that we are waking the Waker in the same thread.
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.count += 1;
        println!("poll_next triggered {} times", this.count);
        while let Ok(mut new_data) = this.rx.try_recv() {
            this.data.append(&mut new_data);
        }
        match this.next_item_len() {
            Some(len) if len <= this.data.len() => {
                let item = this.data.drain(..len).collect();
                Poll::Ready(Some(item))
            }
            _ => {
                thread::sleep(Duration::from_secs(2));
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}

// Better practice
const STEP: usize = 4;
const ITEM: [u8; 12] = [0xff, 10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

#[tokio::main]
async fn main() {
    let (tx, rx) = mpsc::channel();
    let mut stream = SimpleKLVStream {
        rx,
        tx,
        count: 0,
        data: vec![]
    };

    let tx = stream.tx.clone();
    tokio::spawn(async move {
        let mut count = 0;
        loop {
            let index = (count*STEP) % ITEM.len();
            count += 1;
            println!("count: {count}: sending 4 bytes");
            let data = ITEM[index..index+STEP].to_vec();
            tx.send(data).unwrap();
            thread::sleep(Duration::from_secs(1));
        }
    });

    while let Some(item) = stream.next().await {
        println!("got item: {:?}", item);
    }
}
```

#### Output

```
poll_next triggered 1 times
count: 1: sending 4 bytes
count: 2: sending 4 bytes
count: 3: sending 4 bytes
poll_next triggered 2 times
got item: [255, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
poll_next triggered 3 times
count: 4: sending 4 bytes
poll_next triggered 4 times
count: 5: sending 4 bytes
count: 6: sending 4 bytes
poll_next triggered 5 times
got item: [255, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
poll_next triggered 6 times
count: 7: sending 4 bytes
count: 8: sending 4 bytes
poll_next triggered 7 times
count: 9: sending 4 bytes
count: 10: sending 4 bytes
poll_next triggered 8 times
got item: [255, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
poll_next triggered 9 times
count: 11: sending 4 bytes
count: 12: sending 4 bytes
poll_next triggered 10 times
got item: [255, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
poll_next triggered 11 times
count: 13: sending 4 bytes
...
```

#### Single thread?

But what happens if we run this program on a single thread?

New Cargo.toml dependencies
```toml
[dependencies]
futures = "0.3.28"
tokio = { version = "1.28.0", features = ["rt", "macros"] }
```

main.rs
```rust
...

#[tokio::main(flavor = "current_thread")]
async fn main() {
...
```

#### New output
```
poll_next triggered 1 times
count: 1: sending 4 bytes
count: 2: sending 4 bytes
count: 3: sending 4 bytes
count: 4: sending 4 bytes
count: 5: sending 4 bytes
count: 6: sending 4 bytes
count: 7: sending 4 bytes
count: 8: sending 4 bytes
count: 9: sending 4 bytes
```

Notice that after `poll_next` 2 seconds pass before the other tokio task starts sending bytes.

#### Conclusion

We can see now that the multi-threaded version of the stream works.
Every 2 seconds, `poll_next()` is called and any new data are added to the buffer.
When the buffer contains the necessary amount of bytes a new KLV item is returned.

But using one thread it fails; calling `thread::sleep` blocks the thread and we have no practical async functionality.

### 3. tokio::time::sleep()

#### Documentation:

> Equivalent to sleep_until(Instant::now() + duration). An asynchronous analog to std::thread::sleep.
> No work is performed while awaiting on the sleep future to complete

Cargo.toml dependencies
```toml
[dependencies]
futures = "0.3.28"
tokio = { version = "1.28.0", features = ["rt", "macros", "time"] }
```

main.rs
```rust
use futures::stream::{Stream, StreamExt};
use std::{sync::mpsc::{self, Receiver, Sender}, task::{Poll, Context}, pin::Pin, time::Duration};

pub struct SimpleKLVStream {
    pub rx: Receiver<Vec<u8>>,
    pub tx: Sender<Vec<u8>>,
    pub count: usize,
    pub data: Vec<u8>
}

impl SimpleKLVStream {
    pub fn next_item_len(&self) -> Option<usize> {
        self.data.iter().nth(1).map(|val_len| *val_len as usize + 2)
    }
}

impl Stream for SimpleKLVStream {
    type Item = Vec<u8>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.count += 1;
        println!("poll_next triggered {} times", this.count);
        while let Ok(mut new_data) = this.rx.try_recv() {
            this.data.append(&mut new_data);
        }
        match this.next_item_len() {
            Some(len) if len <= this.data.len() => {
                let item = this.data.drain(..len).collect();
                Poll::Ready(Some(item))
            }
            _ => {
                // Notice we have to clone the waker in order to use it in a new task
                let waker = cx.waker().clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    waker.wake();
                });
                Poll::Pending
            }
        }
    }
}

const STEP: usize = 4;
const ITEM: [u8; 12] = [0xff, 10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (tx, rx) = mpsc::channel();
    let mut stream = SimpleKLVStream {
        rx,
        tx,
        count: 0,
        data: vec![]
    };

    let tx = stream.tx.clone();
    tokio::spawn(async move {
        let mut count = 0;
        loop {
            let index = (count*STEP) % ITEM.len();
            count += 1;
            println!("count: {count}: sending {STEP} bytes");
            let data = ITEM[index..index+STEP].to_vec();
            tx.send(data).unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    while let Some(item) = stream.next().await {
        println!("got item: {:?}", item);
    }
}
```

#### Output

```
poll_next triggered 1 times
count: 1: sending 4 bytes
count: 2: sending 4 bytes
poll_next triggered 2 times
count: 3: sending 4 bytes
count: 4: sending 4 bytes
poll_next triggered 3 times
got item: [255, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
poll_next triggered 4 times
count: 5: sending 4 bytes
count: 6: sending 4 bytes
poll_next triggered 5 times
got item: [255, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
poll_next triggered 6 times
count: 7: sending 4 bytes
...
```

#### Conclusion

Do not block a task using std's synchronization primitives.
Notice that we have to clone the context's waker before using it in a `tokio::spawn()`

