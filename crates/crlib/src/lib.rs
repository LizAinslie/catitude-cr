//! Extensible PC/SC smart-card reader framework.
//!
//! `crlib` talks to a card through a [`Transport`], parses APDUs and
//! BER-TLV, and runs a registry of [`CardApp`] implementations. Built-in
//! apps identify the ATR, dump DESFire / Calypso / ISO 7816 *structure*,
//! and read a small, deliberately limited set of EMV fields (scheme, masked
//! PAN, expiry, application label).
//!
//! The framework is meant to be extended: implement [`CardApp`] for a
//! transit system or a PIV badge, then [`Registry::register`] it.
//!
//! Built-in parsers never return a full payment-card PAN, a cardholder name
//! taken from track data, or a CVV. See `apps::emv` for the policy and the
//! extension point.

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
pub use pcsc_transport::{CardSnapshot, CardWatch, PcscCard, PcscContext};
pub use registry::{CardApp, ProbeResult, Registry};
pub use tlv::Tlv;
pub use transport::{transceive, transceive_ok, ScriptedTransport, Transport};

/// Convenience: open the default PC/SC reader and run the standard registry.
pub fn read_default() -> Result<Vec<CardInfo>> {
    let ctx = PcscContext::establish()?;
    let mut card = ctx.connect(None)?;
    Ok(Registry::standard().read_all(&mut card))
}
