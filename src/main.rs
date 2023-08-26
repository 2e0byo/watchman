use inotify::{EventMask, Inotify, WatchMask};
use tokio::{
    sync::mpsc::{channel, Receiver, Sender},
    task::{self, JoinHandle},
    time::{sleep, Duration},
};
use tokio_stream::StreamExt;

fn setup() -> Inotify {
    let inotify = Inotify::init().expect("Error initialising inotify");
    ["/dev/video0"].map(|p| {
        inotify
            .watches()
            .add(p, WatchMask::OPEN | WatchMask::CLOSE)
            .expect(&format!("Failed to add watch for {}", p));
    });
    inotify
}

async fn filter(state: bool, delay_ms: u64, tx: Sender<bool>) {
    sleep(Duration::from_millis(delay_ms)).await;
    tx.send(state)
        .await
        .expect("Unable to send state for processing.");
}

async fn update(mut rx: Receiver<bool>) {
    let mut state = false;
    while let Some(msg) = rx.recv().await {
        if msg != state {
            state = msg;
            println!("Camera is {}", if state { "on" } else { "off" });
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let delay_ms = 150;
    let inotify = setup();
    let buffer = [0; 1024];
    let stream = inotify
        .into_event_stream(buffer)
        .expect("Failed to start stream.")
        .filter(|msg| match msg {
            Ok(_) => true,
            _ => false,
        })
        .map(|msg| msg.unwrap().mask);

    // Pin memory access so fn can yield safely
    tokio::pin!(stream);

    let (tx, rx) = channel::<bool>(32);
    tokio::spawn(update(rx));

    let mut fut: Option<JoinHandle<()>> = None;

    while let Some(msg) = stream.next().await {
        match fut {
            None => (),
            Some(fut) => fut.abort(),
        };
        let state = match msg {
            EventMask::OPEN => true,
            EventMask::CLOSE_NOWRITE | EventMask::CLOSE_WRITE => false,
            other => panic!("Got other event: {:?}", other),
        };
        fut = Some(task::spawn(filter(state, delay_ms, tx.clone())));
    }
}
