//! UI state. The worker owns PC/SC; this module is plain data the view renders.

use cr_autofill::credential::Credential;
use crlib::{CardInfo, ReaderSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    NoReader,
    WaitingForCard,
    Reading,
    Done,
    Unsupported(String),
    Error(String),
}

impl Status {
    pub fn label(&self) -> &str {
        match self {
            Status::NoReader => "No reader",
            Status::WaitingForCard => "Waiting for card",
            Status::Reading => "Reading",
            Status::Done => "Ready",
            Status::Unsupported(_) => "Unsupported card",
            Status::Error(_) => "Error",
        }
    }
}

/// One successful probe of the card currently in the reader.
#[derive(Debug, Clone)]
pub struct CardRead {
    pub reader: String,
    pub infos: Vec<CardInfo>,
    pub notes: Vec<String>,
    pub credential: Option<Credential>,
}

#[derive(Debug, Clone)]
pub enum Event {
    ReadersChanged(Vec<String>),
    ReaderState(Option<ReaderSnapshot>),
    CardInserted(String),
    CardRemoved,
    ReadComplete(CardRead),
    CardUnsupported { reader: String, detail: String },
    ReadError(String),
}

#[derive(Debug)]
pub struct AppState {
    pub readers: Vec<String>,
    pub selected: Option<usize>,
    pub status: Status,
    pub result: Option<CardRead>,
    pub reader_flags: Option<ReaderSnapshot>,
    pub log: Vec<String>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            readers: Vec::new(),
            selected: None,
            status: Status::WaitingForCard,
            result: None,
            reader_flags: None,
            log: vec!["looking for readers".into()],
        }
    }

    pub fn push_log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        const CAP: usize = 200;
        if self.log.len() > CAP {
            let drop_n = self.log.len() - CAP;
            self.log.drain(0..drop_n);
        }
    }

    pub fn selected_name(&self) -> Option<&str> {
        self.selected
            .and_then(|index| self.readers.get(index))
            .map(String::as_str)
    }

    /// Keep the previously selected reader name when the list is replaced.
    pub fn set_readers(&mut self, readers: Vec<String>) {
        let previous = self.selected_name().map(str::to_string);
        self.readers = readers;
        self.selected = previous
            .as_deref()
            .and_then(|name| self.readers.iter().position(|reader| reader == name))
            .or_else(|| (!self.readers.is_empty()).then_some(0));
        if self.readers.is_empty() {
            self.status = Status::NoReader;
            self.result = None;
        } else if matches!(self.status, Status::NoReader | Status::WaitingForCard) {
            self.status = Status::WaitingForCard;
        }
    }
}
