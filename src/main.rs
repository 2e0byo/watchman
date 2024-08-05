use clap::Parser;
use inotify::{EventMask, Inotify, WatchMask};
use std::collections::HashMap;
use std::process::Command;
use std::time::Instant;
use tokio::{
    sync::mpsc::{channel, Receiver, Sender},
    task::{self, JoinHandle},
    time::{sleep, Duration},
};
use tokio_stream::StreamExt;

#[derive(Debug, PartialEq)]
enum State {
    InUse,
    NotInUse,
}

#[derive(Debug)]
struct Event {
    timestamp: Instant,
    state: State,
    path: String,
}

impl Event {
    fn new(state: State, path: &String) -> Event {
        Event {
            timestamp: Instant::now(),
            state,
            path: path.to_string(),
        }
    }
}

async fn filter(count: u8, delay_ms: u64, tx: Sender<u8>) {
    sleep(Duration::from_millis(delay_ms)).await;
    tx.send(count)
        .await
        .expect("Unable to send state for processing.");
}

async fn update(mut rx: Receiver<u8>, cmd: String, on: String, off: String) {
    let mut current_state = State::NotInUse;
    let cmd = cmd.clone();
    let on = on.clone();
    let off = off.clone();
    while let Some(state) = rx.recv().await.map(|count| {
        if count > 0 {
            State::InUse
        } else {
            State::NotInUse
        }
    }) {
        if state != current_state {
            current_state = state;
            let arg = match current_state {
                State::InUse => &on,
                State::NotInUse => &off,
            };
            let res = Command::new(&cmd).arg(arg).spawn();
            match res {
                Ok(_) => (),
                Err(e) => println!(
                    "Failed to start process \"{cmd} {args}\": {error:?}",
                    cmd = cmd,
                    args = arg,
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
    /// Command to call; will receive state as first arg.
    cmd: String,

    /// Files to watch
    paths: Vec<String>,

    /// Filter delay in ms.
    #[arg(short, long, default_value_t = 500)]
    delay: u64,

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
    let mut path_map = HashMap::new();
    let mut count_map: HashMap<String, u8> = HashMap::new();

    let inotify = Inotify::init().expect("Error initialising inotify");
    for path in args.paths.iter() {
        let wd = inotify
            .watches()
            .add(path.clone(), WatchMask::OPEN | WatchMask::CLOSE)
            .unwrap_or_else(|_| panic!("Failed to add watch for {}", path));
        path_map.insert(wd, path);
    }
    let buffer = [0; 1024];
    let stream = inotify
        .into_event_stream(buffer)
        .expect("Failed to start stream.")
        .filter(|msg| msg.is_ok())
        .map(|msg| msg.unwrap())
        .map(|msg| {
            let state = match msg.mask {
                EventMask::OPEN => State::InUse,
                EventMask::CLOSE_NOWRITE | EventMask::CLOSE_WRITE => State::NotInUse,
                other => panic!("Unknown event: {:?}", other),
            };
            let path = path_map.get(&msg.wd).unwrap();
            Event::new(state, path)
        });

    for path in args.paths.iter() {
        count_map.insert(path.to_string(), 0);
    }

    tokio::pin!(stream);

    let (tx, rx) = channel::<u8>(32);
    tokio::spawn(update(rx, args.cmd, args.on, args.off));

    let mut fut: Option<JoinHandle<()>> = None;

    while let Some(event) = stream.next().await {
        let current = count_map.get(&event.path).unwrap();
        let count = match event.state {
            State::InUse => current.to_owned().saturating_add(1),
            State::NotInUse => current.saturating_sub(1),
        };
        count_map.insert(event.path.clone(), count);
        match fut {
            None => (),
            Some(fut) => fut.abort(),
        };
        fut = Some(task::spawn(filter(count, args.delay, tx.clone())));
    }
}
