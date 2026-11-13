//! OS-level autofill provider backed by a PC/SC smart card.
//!
//! `cr-autofill` reads a payment card (or any card exposing EMV records) over
//! PC/SC, extracts the credential, and types it into the focused field. It
//! talks to the card directly over APDUs — there is no vendor middleware or
//! reader firmware in the loop to bypass.

pub mod credential;
pub mod typer;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
