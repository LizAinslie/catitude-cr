//! Extensible PC/SC smart-card reader framework.
//!
//! `crlib` talks to a card through a [`Transport`], parses APDUs and
//! BER-TLV, and runs a registry of [`CardApp`] implementations. Built-in
//! apps identify the ATR, dump DESFire / Calypso / ISO 7816 *structure*,
//! and read EMV credentials (scheme, masked PAN, expiry, application label,
//! cardholder).
//!
//! The EMV structural dump is intentionally **unredacted**: it includes the
//! full PAN, Track 1 / Track 2 equivalent data, and the cardholder name.
//! This is what local wallet and export tools consume. The high-level `pan`
//! field stays masked for inspector views; use the raw dump or
//! `cr-autofill` when you need the full value.
//!
//! A CVV / CVC is not present on the chip. Any save-and-replay flow must ask
//! the user for it separately.
//!
//! The framework is meant to be extended: implement [`CardApp`] for a
//! transit system or a PIV badge, then [`Registry::register`] it.

pub mod apdu;
pub mod apps;
pub mod atr;
pub mod error;
pub mod hexutil;
pub mod info;
pub mod pcsc_transport;
pub mod registry;
pub mod tlv;
pub mod transport;

pub use apdu::{Apdu, ApduResponse};
pub use apps::{scheme_from_aid, select_ppse, AtrApp, CalypsoApp, DesfireApp, EmvApp, Iso7816App};
pub use atr::{Atr, Convention};
pub use error::{Error, Result};
pub use info::{view_rows, CardInfo, DumpLine, Field, RowKind, ViewRow};
pub use pcsc_transport::{
    CardSnapshot, CardSnapshotDetailed, CardWatch, PcscCard, PcscContext, ReaderSnapshot,
    ReaderStateFlags,
};
pub use registry::{CardApp, ProbeResult, Registry};
pub use tlv::Tlv;
pub use transport::{transceive, transceive_ok, ScriptedTransport, Transport};

/// Convenience: open the default PC/SC reader and run the standard registry.
pub fn read_default() -> Result<Vec<CardInfo>> {
    let ctx = PcscContext::establish()?;
    let mut card = ctx.connect(None)?;
    Ok(Registry::standard().read_all(&mut card))
}
