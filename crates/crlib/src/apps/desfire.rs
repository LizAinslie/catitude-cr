//! MIFARE DESFire structure dumper.
//!
//! Lists the card's version, application IDs, and — for each application —
//! file IDs and file settings (type and size). It does **not** read file
//! contents: those may hold personal or payment data depending on the
//! application.

use crate::apdu::Apdu;
use crate::error::{Error, Result};
use crate::hexutil;
use crate::info::CardInfo;
use crate::registry::CardApp;
use crate::transport::{transceive, Transport};

pub struct DesfireApp;

impl CardApp for DesfireApp {
    fn name(&self) -> &'static str {
        "desfire"
    }

    fn aids(&self) -> &'static [&'static [u8]] {
        // DESFire is selected by a native command, not an ISO AID, though
        // some cards also expose the ISO AID A000000396.
        &[&[0xA0, 0x00, 0x00, 0x03, 0x96]]
    }

    fn try_read(&self, card: &mut dyn Transport) -> Result<CardInfo> {
        let version = desfire_cmd(card, 0x60, &[])?;
        if version.len() < 7 {
            return Err(Error::NotApplicable);
        }

        let mut info = CardInfo::new(self.name());
        info.push("uid", hexutil::encode_pretty(uid_from_version(&version)));
        if version.len() >= 14 {
            info.push(
                "hardware",
                format!(
                    "vendor {:02X} type {:02X} ver {}.{}",
                    version[0], version[1], version[3], version[4]
                ),
            );
        }
        info.push_raw("get_version", hexutil::encode_pretty(&version));

        match desfire_cmd(card, 0x6A, &[]) {
            Ok(aids) => {
                let list = aids.chunks_exact(3).collect::<Vec<_>>();
                info.push("applications", list.len().to_string());
                for aid in list {
                    let _ = inspect_application(card, aid, &mut info);
                }
            }
            Err(_) => {
                info.push("applications", "unavailable (not a DESFire, or denied)");
            }
        }

        Ok(info)
    }
}

fn uid_from_version(version: &[u8]) -> &[u8] {
    // GetVersion returns hardware (7) + software (7) + UID (7) + production.
    // After 91AF chaining we typically have 28 bytes; UID starts at offset 14.
    if version.len() >= 21 {
        &version[14..21]
    } else {
        &[]
    }
}

fn inspect_application(card: &mut dyn Transport, aid: &[u8], info: &mut CardInfo) -> Result<()> {
    let key = format!("app {}", hexutil::encode(aid));
    if desfire_cmd(card, 0x5A, aid).is_err() {
        info.push_raw(&key, "select failed");
        return Ok(());
    }
    match desfire_cmd(card, 0x6F, &[]) {
        Ok(fids) => {
            let mut desc = format!("{} file(s)", fids.len());
            for fid in &fids {
                if let Ok(settings) = desfire_cmd(card, 0xF5, &[*fid]) {
                    let kind = file_type(settings.first().copied().unwrap_or(0xFF));
                    let size = file_size(&settings);
                    desc.push_str(&format!(" ; {:02X}: {kind} {size}", fid));
                    info.push_raw(
                        format!("{key}.file {:02X}", fid),
                        format!(
                            "{kind} {size}  settings {}",
                            hexutil::encode_pretty(&settings)
                        ),
                    );
                } else {
                    desc.push_str(&format!(" ; {:02X}: (no settings)", fid));
                }
            }
            info.push(key, desc);
        }
        Err(_) => {
            info.push(key, "selected, file list denied");
        }
    }
    Ok(())
}

fn file_type(t: u8) -> &'static str {
    match t {
        0x00 => "standard-data",
        0x01 => "backup-data",
        0x02 => "value",
        0x03 => "linear-record",
        0x04 => "cyclic-record",
        other => {
            let _ = other;
            "unknown"
        }
    }
}

fn file_size(settings: &[u8]) -> String {
    // Layout varies slightly; size is a 3-byte little-endian field after
    // the type / comm / access bytes. Be conservative and only print it
    // when the buffer looks long enough.
    if settings.len() >= 7 {
        let size = settings[4] as u32 | ((settings[5] as u32) << 8) | ((settings[6] as u32) << 16);
        format!("{size} bytes")
    } else {
        "size ?".into()
    }
}

fn desfire_cmd(card: &mut dyn Transport, ins: u8, data: &[u8]) -> Result<Vec<u8>> {
    let resp = transceive(card, &Apdu::desfire(ins, data))?;
    if resp.sw() == 0x9100 || resp.is_ok() || resp.desfire_more() {
        // transceive already drained 91AF additional frames into resp.data
        if resp.sw() == 0x9100 || resp.sw() == 0x9000 {
            return Ok(resp.data);
        }
    }
    // Some stacks return 91xx only, and 91AF was already consumed.
    if resp.sw1 == 0x91 && (resp.sw2 == 0x00 || resp.sw2 == 0xAF) {
        return Ok(resp.data);
    }
    // A completely mute / non-DESFire card typically returns 6E00 or 6D00.
    if matches!(resp.sw(), 0x6E00 | 0x6D00 | 0x6A82 | 0x6A81) {
        return Err(Error::NotApplicable);
    }
    Err(Error::Status {
        sw1: resp.sw1,
        sw2: resp.sw2,
    })
}
