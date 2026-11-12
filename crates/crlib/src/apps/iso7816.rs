//! Generic ISO 7816 structure dumper.
//!
//! Selects the master file, reports the FCI, and tries a small set of
//! well-known dedicated-file identifiers. It does not walk arbitrary EFs
//! or dump their contents, so it stays useful as a "what's on this card"
//! starting point without slurping application data.

use crate::apdu::Apdu;
use crate::error::{Error, Result};
use crate::hexutil;
use crate::info::CardInfo;
use crate::registry::CardApp;
use crate::tlv::{self, Tlv};
use crate::transport::{transceive, transceive_ok, Transport};

/// Tags that, if they happen to show up in an FCI, must not be dumped.
const REDACTED_TAGS: &[&[u8]] = &[
    &[0x5A],
    &[0x57],
    &[0x56],
    &[0x5F, 0x20],
    &[0x9F, 0x1F],
    &[0x9F, 0x20],
    &[0x9F, 0x6B],
];

const COMMON_DFS: &[([u8; 2], &str)] = &[
    ([0x3F, 0x00], "master file"),
    ([0x7F, 0x10], "DF telecom / EMV"),
    ([0x7F, 0x20], "DF GSM / ICAO DG? (selected, not read)"),
    ([0x7F, 0x21], "DF military / national"),
    ([0x50, 0x15], "DF PKCS#15"),
    ([0x50, 0x32], "DF PKCS#15 (alt)"),
    ([0xA0, 0x00], "DF proprietary A000"),
];

pub struct Iso7816App;

impl CardApp for Iso7816App {
    fn name(&self) -> &'static str {
        "iso7816"
    }

    fn aids(&self) -> &'static [&'static [u8]] {
        &[]
    }

    fn try_read(&self, card: &mut dyn Transport) -> Result<CardInfo> {
        let resp = match transceive_ok(card, &Apdu::select_mf()) {
            Ok(r) => r,
            Err(Error::Status { .. }) => {
                // Some cards want SELECT MF with P2=0x0C (no FCI) or a
                // different P1. Try once more before giving up.
                let alt = Apdu::new(0x00, 0xA4, 0x00, 0x0C).with_data(vec![0x3F, 0x00]);
                transceive(card, &alt).map_err(|_| Error::NotApplicable)?
            }
            Err(err) => return Err(err),
        };

        if !resp.is_ok() && !resp.data.is_empty() && resp.sw() != 0x9000 {
            // If even the retry failed with no useful body, this probably
            // isn't an ISO 7816 file-oriented card (e.g. a raw DESFire).
            if matches!(resp.sw(), 0x6E00 | 0x6D00 | 0x6A81 | 0x6A82) {
                return Err(Error::NotApplicable);
            }
        }

        let mut info = CardInfo::new(self.name());
        info.push("mf_status", format!("{:04X}", resp.sw()));
        if !resp.data.is_empty() {
            match tlv::parse_all(&resp.data) {
                Ok(fci) => {
                    if let Some(df) = tlv::find(&fci, &[0x84]) {
                        info.push("df_name", hexutil::encode(&df.value));
                        if let Some(ascii) = hexutil::ascii_if_printable(&df.value) {
                            info.push("df_name.ascii", ascii);
                        }
                    }
                    dump(&mut info, "fci", &fci);
                }
                Err(_) => {
                    info.push_raw("mf.fci.raw", hexutil::encode_pretty(&resp.data));
                }
            }
        }

        let mut found = 0u32;
        for (fid, name) in COMMON_DFS {
            if *fid == [0x3F, 0x00] {
                continue;
            }
            if let Ok(r) = transceive(card, &Apdu::select_by_fid(*fid)) {
                if r.is_ok() {
                    found += 1;
                    info.push(format!("df {}", hexutil::encode(fid)), *name);
                }
            }
        }
        info.push("known_dfs_present", found.to_string());
        Ok(info)
    }
}

fn dump(info: &mut CardInfo, prefix: &str, nodes: &[Tlv]) {
    tlv::walk(nodes, |tlv, depth| {
        let indent = "  ".repeat(depth);
        let label = format!("{prefix}.{indent}{}", tlv.tag_hex());
        if REDACTED_TAGS.iter().any(|t| *t == tlv.tag.as_slice()) {
            info.push_raw(label, "[redacted]");
        } else if tlv.is_constructed() {
            info.push_raw(label, format!("constructed ({} bytes)", tlv.value.len()));
        } else if let Some(ascii) = hexutil::ascii_if_printable(&tlv.value) {
            info.push_raw(label, ascii);
        } else {
            info.push_raw(label, tlv.value_hex());
        }
    });
}
