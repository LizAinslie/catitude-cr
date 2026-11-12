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
