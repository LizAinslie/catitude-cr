//! Calypso (and "1TIC.ICA") structure dumper.
//!
//! Selects the ticketing application and reports which known elementary
//! files are present, with a record count. File *contents* are not read:
//! Calypso environment / holder files can contain personal data.

use crate::apdu::Apdu;
use crate::error::{Error, Result};
use crate::hexutil;
use crate::info::CardInfo;
use crate::registry::CardApp;
use crate::tlv;
use crate::transport::{transceive, transceive_ok, Transport};

/// Common Calypso AIDs. `1TIC.ICA` is the Intercode / French transit one.
const AIDS: &[&[u8]] = &[
    b"1TIC.ICA",
    &[0xA0, 0x00, 0x00, 0x04, 0x04, 0x01, 0x01], // Calypso generic
    &[0xA0, 0x00, 0x00, 0x04, 0x04, 0x01, 0x25],
];

/// Well-known Calypso / Intercode file identifiers. Listed, not read.
const FILES: &[([u8; 2], &str)] = &[
    ([0x20, 0x00], "environment / holder DF"),
    ([0x20, 0x01], "environment / holder EF"),
    ([0x20, 0x10], "environment"),
    ([0x20, 0x20], "events / journal"),
    ([0x20, 0x21], "events special"),
    ([0x20, 0x30], "contracts / subscriptions"),
    ([0x20, 0x31], "contracts list"),
    ([0x20, 0x40], "counters"),
    ([0x20, 0x50], "special events"),
    ([0x20, 0x60], "contracts"),
    ([0x20, 0x61], "contracts (alt)"),
    ([0x20, 0x69], "counters (alt)"),
    ([0x20, 0x6A], "counters (alt 2)"),
    ([0x3F, 0x1C], "extended / ATR file"),
];

pub struct CalypsoApp;

impl CardApp for CalypsoApp {
    fn name(&self) -> &'static str {
        "calypso"
    }

    fn aids(&self) -> &'static [&'static [u8]] {
        AIDS
    }

    fn try_read(&self, card: &mut dyn Transport) -> Result<CardInfo> {
        let mut selected: Option<(Vec<u8>, Vec<u8>)> = None;
        for aid in AIDS {
            if let Ok(resp) = transceive_ok(card, &Apdu::select_by_name(aid)) {
                selected = Some((aid.to_vec(), resp.data));
                break;
            }
        }
        let (aid, fci_bytes) = selected.ok_or(Error::NotApplicable)?;

        let mut info = CardInfo::new(self.name());
        info.push("aid", hexutil::encode(&aid));
        if let Some(ascii) = hexutil::ascii_if_printable(&aid) {
            info.push("aid.ascii", ascii);
        }
        if let Ok(fci) = tlv::parse_all(&fci_bytes) {
            if let Some(label) =
                tlv::find(&fci, &[0x50]).and_then(|t| hexutil::ascii_if_printable(&t.value))
            {
                info.push("application_label", label);
            }
            info.push_raw("fci", hexutil::encode_pretty(&fci_bytes));
        }

        let mut present = 0u32;
        for (fid, name) in FILES {
            match transceive(card, &Apdu::select_by_fid(*fid)) {
                Ok(resp) if resp.is_ok() => {
                    present += 1;
                    let records = count_records(card);
                    info.push(
                        format!("file {}", hexutil::encode(fid)),
                        format!("{name}; {records} readable record(s)"),
                    );
                }
                _ => {}
            }
        }
        info.push("files_present", present.to_string());
        Ok(info)
    }
}

/// Count records by issuing READ RECORD until the card says the record
/// does not exist. The record *body is discarded* — we only want a count.
fn count_records(card: &mut dyn Transport) -> u32 {
    let mut count = 0u32;
    for recno in 1..=16u8 {
        match transceive(card, &Apdu::read_record(0, recno)) {
            Ok(resp) if resp.is_ok() => {
                // Intentionally ignore resp.data.
                let _ = resp.data.len();
                count += 1;
            }
            _ => break,
        }
    }
    count
}
