use clap::Parser;
use inotify::{EventMask, Inotify, WatchMask};
use std::process::Command;
use tokio::{
    sync::mpsc::{channel, Receiver, Sender},
    task::{self, JoinHandle},
    time::{sleep, Duration},
};
use tokio_stream::StreamExt;

fn setup(paths: &[&str]) -> Inotify {
    let inotify = Inotify::init().expect("Error initialising inotify");
    for path in paths {
        inotify
            .watches()
            .add(path, WatchMask::OPEN | WatchMask::CLOSE)
            .expect(&format!("Failed to add watch for {}", path));
    }
    inotify
}

async fn filter(state: bool, delay_ms: u64, tx: Sender<bool>) {
    sleep(Duration::from_millis(delay_ms)).await;
    tx.send(state)
        .await
        .expect("Unable to send state for processing.");
}

async fn update(mut rx: Receiver<bool>, cmd: String, on: String, off: String) {
    let mut state = false;
    let cmd = cmd.clone();
    let on = on.clone();
    let off = off.clone();
    while let Some(msg) = rx.recv().await {
        if msg != state {
            state = msg;
            let res = Command::new(&cmd)
                .arg(match state {
                    true => &on,
                    false => &off,
                })
                .spawn();
            match res {
                Ok(_) => (),
                Err(e) => println!(
                    "Failed to start process \"{cmd} {args}\": {error:?}",
                    cmd = cmd,
                    args = state,
                    error = e
                ),
            }
        }
    }
}

/// Debounced inotify watcher
#[derive(Parser, Debug)]
#[command(author, version, about, long_about=None)]
struct Args {
    /// Filter delay in ms.
    #[arg(short, long, default_value_t = 500)]
    delay: u64,

    /// Command to call; will receive state as first arg.
    cmd: String,

    /// String to pass when resource in use.
    #[arg(long, default_value = "on")]
    on: String,

    /// String to pass when resource not in use.
    #[arg(long, default_value = "off")]
    off: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = Args::parse();

    let inotify = setup(&["/dev/video0"]);
    let buffer = [0; 1024];
    let stream = inotify
        .into_event_stream(buffer)
        .expect("Failed to start stream.")
        .filter(|msg| match msg {
            Ok(_) => true,
            _ => false,
        })
        .map(|msg| match msg.unwrap().mask {
            EventMask::OPEN => true,
            EventMask::CLOSE_NOWRITE | EventMask::CLOSE_WRITE => false,
            other => panic!("Unknown event: {:?}", other),
        });

    tokio::pin!(stream);

    let (tx, rx) = channel::<bool>(32);
    tokio::spawn(update(rx, args.cmd, args.on, args.off));

    let mut fut: Option<JoinHandle<()>> = None;

    while let Some(state) = stream.next().await {
        match fut {
            None => (),
            Some(fut) => fut.abort(),
        };
        fut = Some(task::spawn(filter(state, args.delay, tx.clone())));
    }
}
