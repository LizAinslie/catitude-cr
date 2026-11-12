//! Built-in [`CardApp`](crate::CardApp) implementations.

mod atr_id;
mod calypso;
mod desfire;
mod emv;
mod iso7816;

pub use atr_id::AtrApp;
pub use calypso::CalypsoApp;
pub use desfire::DesfireApp;
pub use emv::{scheme_from_aid, select_ppse, EmvApp};
pub use iso7816::Iso7816App;
