use futures::stream::{Stream, StreamExt};
use std::{
    pin::Pin,
    sync::mpsc::{self, Receiver, Sender},
    task::{Context, Poll},
    time::Duration,
};

pub struct SimpleKLVStream {
    pub rx: Receiver<Vec<u8>>,
    pub tx: Sender<Vec<u8>>,
    pub count: usize,
    pub data: Vec<u8>,
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
const ITEM: [u8; 12] = [
    0xff, 10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (tx, rx) = mpsc::channel();
    let mut stream = SimpleKLVStream {
        rx,
        tx,
        count: 0,
        data: vec![],
    };

    let tx = stream.tx.clone();
    tokio::spawn(async move {
        let mut count = 0;
        loop {
            let index = (count * STEP) % ITEM.len();
            count += 1;
            println!("count: {count}: sending {STEP} bytes");
            let data = ITEM[index..index + STEP].to_vec();
            tx.send(data).unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    while let Some(item) = stream.next().await {
        println!("got item: {:?}", item);
    }
}
