use std::ffi::{CStr, CString};
use std::time::Duration;

use pcsc::{Attribute, Card, Context, Protocols, Scope, ShareMode, State, MAX_BUFFER_SIZE};

use crate::apdu::ApduResponse;
use crate::error::{Error, Result};
use crate::transport::Transport;

/// A PC/SC context. Create one of these, list readers, then [`connect`].
pub struct PcscContext {
    inner: Context,
}

impl PcscContext {
    pub fn establish() -> Result<Self> {
        Ok(Self {
            inner: Context::establish(Scope::User)?,
        })
    }

    pub fn readers(&self) -> Result<Vec<String>> {
        let mut buf = [0u8; 4096];
        let names = self.inner.list_readers(&mut buf)?;
        let mut out = Vec::new();
        for name in names {
            out.push(c_to_string(name)?);
        }
        Ok(out)
    }

    /// Connect to a reader. If `reader` is `None`, the first reader is used.
    pub fn connect(&self, reader: Option<&str>) -> Result<PcscCard> {
        let readers = self.readers()?;
        if readers.is_empty() {
            return Err(Error::NoReaders);
        }
        let name = match reader {
            Some(wanted) => readers
                .iter()
                .find(|r| r.as_str() == wanted || r.contains(wanted))
                .cloned()
                .ok_or_else(|| Error::ReaderNotFound(wanted.to_string()))?,
            None => readers.into_iter().next().unwrap(),
        };
        self.connect_named(&name)
    }

    fn connect_named(&self, name: &str) -> Result<PcscCard> {
        let cname =
            CString::new(name).map_err(|_| Error::Other("reader name contains a NUL".into()))?;
        let card = self
            .inner
            .connect(&cname, ShareMode::Shared, Protocols::ANY)?;
        let atr = read_atr(&card)?;
        Ok(PcscCard {
            reader: name.to_string(),
            atr,
            card,
        })
    }

    /// Block until a card is present in `reader` (or any reader if `None`).
    /// Returns the reader name that showed a card.
    pub fn wait_for_card(&self, reader: Option<&str>, timeout: Duration) -> Result<String> {
        let readers = self.readers()?;
        if readers.is_empty() {
            return Err(Error::NoReaders);
        }
        let watched: Vec<String> = match reader {
            Some(wanted) => {
                let found = readers
                    .iter()
                    .find(|r| r.as_str() == wanted || r.contains(wanted))
                    .cloned()
                    .ok_or_else(|| Error::ReaderNotFound(wanted.to_string()))?;
                vec![found]
            }
            None => readers,
        };

        let mut states: Vec<pcsc::ReaderState> = Vec::with_capacity(watched.len());
        for n in &watched {
            let cname = CString::new(n.as_str())
                .map_err(|_| Error::Other("reader name contains a NUL".into()))?;
            states.push(pcsc::ReaderState::new(cname, State::UNAWARE));
        }

        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::NoCard);
            }
            match self.inner.get_status_change(Some(remaining), &mut states) {
                Ok(()) => {}
                Err(pcsc::Error::Timeout) => return Err(Error::NoCard),
                Err(other) => return Err(other.into()),
            }
            for state in &states {
                let ev = state.event_state();
                if ev.contains(State::PRESENT) && !ev.contains(State::MUTE) {
                    return c_to_string(state.name());
                }
            }
            for state in &mut states {
                state.sync_current_state();
            }
        }
    }
}

/// A shared connection to a card in one reader.
pub struct PcscCard {
    reader: String,
    atr: Vec<u8>,
    card: Card,
}

impl PcscCard {
    pub fn reader_name(&self) -> &str {
        &self.reader
    }
}

impl Transport for PcscCard {
    fn reader_name(&self) -> &str {
        &self.reader
    }

    fn atr(&self) -> &[u8] {
        &self.atr
    }

    fn transmit(&mut self, command: &[u8]) -> Result<ApduResponse> {
        let mut buf = [0u8; MAX_BUFFER_SIZE];
        let reply = self.card.transmit(command, &mut buf)?;
        ApduResponse::from_bytes(reply)
    }
}

fn read_atr(card: &Card) -> Result<Vec<u8>> {
    match card.status2_owned() {
        Ok(status) => Ok(status.atr().to_vec()),
        Err(_) => match card.get_attribute_owned(Attribute::AtrString) {
            Ok(bytes) => Ok(bytes),
            Err(err) => Err(err.into()),
        },
    }
}

fn c_to_string(name: &CStr) -> Result<String> {
    name.to_str()
        .map(|s| s.to_string())
        .map_err(|_| Error::Other("reader name is not valid UTF-8".into()))
}

/// A responsive card: present, powered, and not mute.
fn card_is_live(state: State) -> bool {
    state.contains(State::PRESENT)
        && !state.contains(State::MUTE)
        && !state.contains(State::EMPTY)
        && !state.contains(State::UNAVAILABLE)
}

/// Remembers PC/SC reader states between [`PcscContext::poll_cards`] calls.
///
/// One worker thread should own both the context and this watch. The watch
/// is what lets a caller block in `get_status_change` instead of spinning.
pub struct CardWatch {
    names: Vec<String>,
    states: Vec<pcsc::ReaderState>,
}

impl CardWatch {
    fn new() -> Self {
        Self {
            names: Vec::new(),
            states: Vec::new(),
        }
    }
}

/// Readers visible right now, and which watched reader holds a live card.
pub struct CardSnapshot {
    pub readers: Vec<String>,
    /// Watched reader that currently has a responsive card.
    pub present: Option<String>,
}

impl PcscContext {
    /// Start a presence watch. Pair it with [`Self::poll_cards`].
    pub fn card_watch(&self) -> CardWatch {
        CardWatch::new()
    }

    /// Block until a watched reader's card presence changes, or `timeout` elapses.
    ///
    /// `reader` limits the watch to one reader (exact name, or a substring,
    /// same rule as [`Self::connect`]). `None` watches every reader and
    /// reports the first one with a live card.
    ///
    /// When nothing is plugged in, this still waits out `timeout` so a caller
    /// can poll a command channel at that cadence without spinning.
    /// The reader list is refreshed on every call.
    pub fn poll_cards(
        &self,
        watch: &mut CardWatch,
        reader: Option<&str>,
        timeout: Duration,
    ) -> Result<CardSnapshot> {
        let readers = match self.readers() {
            Ok(readers) => readers,
            Err(Error::NoReaders) => Vec::new(),
            Err(err) => return Err(err),
        };
        let wanted = watched_names(&readers, reader);
        if watch.names != wanted {
            let states = wanted
                .iter()
                .map(|name| {
                    CString::new(name.as_str())
                        .map(|cname| pcsc::ReaderState::new(cname, State::UNAWARE))
                        .map_err(|_| Error::Other("reader name contains a NUL".into()))
                })
                .collect::<Result<Vec<_>>>()?;
            watch.names.clone_from(&wanted);
            watch.states = states;
        }

        if watch.states.is_empty() {
            std::thread::sleep(timeout);
            return Ok(CardSnapshot {
                readers,
                present: None,
            });
        }

        let changed = match self
            .inner
            .get_status_change(Some(timeout), &mut watch.states)
        {
            Ok(()) => true,
            Err(pcsc::Error::Timeout) => false,
            Err(err) => return Err(err.into()),
        };

        let present = watch.states.iter().find_map(|state| {
            if !card_is_live(state.event_state()) {
                return None;
            }
            c_to_string(state.name()).ok()
        });

        if changed {
            for state in &mut watch.states {
                state.sync_current_state();
            }
        }

        Ok(CardSnapshot { readers, present })
    }
}

fn watched_names(readers: &[String], reader: Option<&str>) -> Vec<String> {
    match reader {
        Some(wanted) => readers
            .iter()
            .filter(|name| name.as_str() == wanted || name.contains(wanted))
            .cloned()
            .collect(),
        None => readers.to_vec(),
    }
}

#[cfg(test)]
mod watch_tests {
    use super::card_is_live;
    use pcsc::State;

    #[test]
    fn live_card_requires_presence_without_mute_or_empty() {
        assert!(card_is_live(State::PRESENT));
        assert!(!card_is_live(State::EMPTY));
        assert!(!card_is_live(State::UNAVAILABLE));
        assert!(!card_is_live(State::PRESENT | State::MUTE));
        assert!(!card_is_live(State::PRESENT | State::EMPTY));
        assert!(card_is_live(State::PRESENT | State::CHANGED));
    }
}
