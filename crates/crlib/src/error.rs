use std::fmt;

/// crlib error type.
#[derive(Debug)]
pub enum Error {
    /// The registered app decided this card is not one it handles.
    NotApplicable,
    /// No PC/SC readers are present.
    NoReaders,
    /// The reader is present but no card is seated (or the card is mute).
    NoCard,
    /// The reader or card name was not found.
    ReaderNotFound(String),
    /// APDU response status was not 0x9000 (and was not handled as 61xx/6Cxx).
    Status { sw1: u8, sw2: u8 },
    /// BER-TLV parsing failed.
    Tlv(String),
    /// APDU encoding or protocol error.
    Apdu(String),
    /// Underlying PC/SC / driver error.
    Pcsc(String),
    /// Catch-all for unexpected failures.
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotApplicable => write!(f, "card app does not apply"),
            Error::NoReaders => write!(f, "no PC/SC readers found"),
            Error::NoCard => write!(f, "no card present in the reader"),
            Error::ReaderNotFound(name) => write!(f, "reader not found: {name}"),
            Error::Status { sw1, sw2 } => {
                write!(f, "unexpected APDU status {sw1:02X}{sw2:02X}")
            }
            Error::Tlv(msg) => write!(f, "TLV: {msg}"),
            Error::Apdu(msg) => write!(f, "APDU: {msg}"),
            Error::Pcsc(msg) => write!(f, "PC/SC: {msg}"),
            Error::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<pcsc::Error> for Error {
    fn from(err: pcsc::Error) -> Self {
        match err {
            pcsc::Error::NoReadersAvailable => Error::NoReaders,
            pcsc::Error::NoSmartcard | pcsc::Error::RemovedCard | pcsc::Error::UnpoweredCard => {
                Error::NoCard
            }
            pcsc::Error::UnknownReader => Error::ReaderNotFound(err.to_string()),
            other => Error::Pcsc(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
