//! PC/SC on a dedicated thread.
//!
//! The UI thread only sends [`Cmd`] and receives [`Event`]. `crlib` and the
//! PC/SC context stay here, so a transceive never blocks a frame.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use crlib::{CardWatch, PcscContext, Registry};

use crate::model::{CardRead, Event};

const POLL: Duration = Duration::from_millis(400);

#[derive(Debug)]
pub enum Cmd {
    /// Watch this reader and read it if a card is already there.
    Select(String),
    /// Drop the cached read and ask PC/SC for the reader list again.
    Refresh,
    Shutdown,
}

pub fn spawn(cmd_rx: Receiver<Cmd>, tx: smol::channel::Sender<Event>) {
    thread::Builder::new()
        .name("crgui-pcsc".into())
        .spawn(move || run(cmd_rx, tx))
        .expect("spawn pcsc worker");
}

fn run(cmd_rx: Receiver<Cmd>, tx: smol::channel::Sender<Event>) {
    let ctx = match PcscContext::establish() {
        Ok(ctx) => ctx,
        Err(err) => {
            let _ = tx.send_blocking(Event::ReadError(explain(err)));
            return;
        }
    };

    let mut watch = ctx.card_watch();
    let mut wanted: Option<String> = None;
    let mut chosen_by_user = false;
    let mut last_readers: Option<Vec<String>> = None;
    // Reader we already parsed for the current insertion. Cleared on removal,
    // reader change, or an explicit rescan.
    let mut held: Option<String> = None;

    loop {
        match drain(
            &cmd_rx,
            &mut wanted,
            &mut chosen_by_user,
            &mut held,
            &mut watch,
            &ctx,
        ) {
            Drain::Shutdown => return,
            Drain::Continue => {}
        }

        let snapshot = match ctx.poll_cards(&mut watch, wanted.as_deref(), POLL) {
            Ok(snapshot) => snapshot,
            Err(err) => {
                if tx.send_blocking(Event::ReadError(explain(err))).is_err() {
                    return;
                }
                watch = ctx.card_watch();
                held = None;
                thread::sleep(POLL);
                continue;
            }
        };

        if last_readers.as_ref() != Some(&snapshot.readers) {
            last_readers = Some(snapshot.readers.clone());
            if !emit(&tx, Event::ReadersChanged(snapshot.readers.clone())) {
                return;
            }
            // `present` was computed for the watch we just polled. If the
            // selection changed, that answer is for a different reader set,
            // so poll again before reading or reporting a removal.
            let mut repoll = false;
            if !chosen_by_user {
                let next = snapshot.readers.first().cloned();
                if wanted != next {
                    wanted = next;
                    watch = ctx.card_watch();
                    repoll = true;
                }
            } else if let Some(name) = &wanted {
                if !snapshot.readers.iter().any(|reader| reader == name) {
                    chosen_by_user = false;
                    wanted = snapshot.readers.first().cloned();
                    held = None;
                    watch = ctx.card_watch();
                    repoll = true;
                }
            }
            if repoll {
                continue;
            }
        }

        match snapshot.present {
            Some(name) if held.as_ref() != Some(&name) => {
                if !emit(&tx, Event::CardInserted(name.clone())) {
                    return;
                }
                match read_card(&ctx, &name) {
                    Ok(read) => {
                        held = Some(name);
                        if !emit(&tx, Event::ReadComplete(read)) {
                            return;
                        }
                    }
                    Err(err) => {
                        // Remember the insertion so a dead card does not
                        // retry on every poll. Rescan or removal clears it.
                        held = Some(name);
                        if !emit(&tx, Event::ReadError(err)) {
                            return;
                        }
                    }
                }
            }
            None if held.is_some() => {
                held = None;
                if !emit(&tx, Event::CardRemoved) {
                    return;
                }
            }
            _ => {}
        }
    }
}

enum Drain {
    Continue,
    Shutdown,
}

fn drain(
    cmd_rx: &Receiver<Cmd>,
    wanted: &mut Option<String>,
    chosen_by_user: &mut bool,
    held: &mut Option<String>,
    watch: &mut CardWatch,
    ctx: &PcscContext,
) -> Drain {
    loop {
        match cmd_rx.try_recv() {
            Ok(Cmd::Shutdown) | Err(TryRecvError::Disconnected) => return Drain::Shutdown,
            Err(TryRecvError::Empty) => return Drain::Continue,
            Ok(Cmd::Select(name)) => {
                *wanted = Some(name);
                *chosen_by_user = true;
                *held = None;
                *watch = ctx.card_watch();
            }
            Ok(Cmd::Refresh) => {
                *held = None;
                *watch = ctx.card_watch();
            }
        }
    }
}

fn read_card(ctx: &PcscContext, reader: &str) -> Result<CardRead, String> {
    let mut card = ctx.connect(Some(reader)).map_err(explain)?;
    let probed = Registry::standard().probe(&mut card);
    let mut read = CardRead {
        reader: reader.to_string(),
        infos: Vec::new(),
        notes: Vec::new(),
    };
    for probe in probed {
        match probe.result {
            Ok(info) => read.infos.push(info),
            Err(err) => read.notes.push(format!("[{}] {err}", probe.app)),
        }
    }
    if read.infos.is_empty() && read.notes.is_empty() {
        read.notes
            .push("no built-in app recognized this card".into());
    }
    Ok(read)
}

fn explain(err: crlib::Error) -> String {
    match &err {
        crlib::Error::NoReaders | crlib::Error::Pcsc(_) | crlib::Error::NoCard => {
            format!("{err}. Is pcscd running?")
        }
        _ => err.to_string(),
    }
}

fn emit(tx: &smol::channel::Sender<Event>, event: Event) -> bool {
    tx.send_blocking(event).is_ok()
}
