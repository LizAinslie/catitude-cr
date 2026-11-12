//! EMV contact/contactless reader, limited to *safe* fields.
//!
//! The only payment-related facts this module will ever put on a [`CardInfo`]
//! are:
//!
//! * card scheme (Visa, Mastercard, …) derived from the Application Identifier
//! * a **masked** PAN (last 4 digits only, the rest replaced with `*`)
//! * application expiry (`YYMMDD` tag `5F24`, formatted `MM/YY`)
//! * application label / preferred name (tags `50` / `9F12`)
//!
//! It does **not** extract, store, or return:
//!
//! * a full Primary Account Number (tag `5A`)
//! * Track 1 / Track 2 equivalent data (tags `57`, `56`, `9F6B`, …)
//! * the cardholder name (tag `5F20`)
//! * a CVV / CVC (those values are not on the chip in any case)
//!
//! Sensitive tags are redacted from the structural dump as well.
//!
//! # Extension point
//!
//! crlib will not grow a full-PAN or track-data parser. If you are writing
//! a downstream crate and you have a lawful need to surface additional EMV
//! fields, implement your own [`crate::CardApp`] (you can reuse
//! [`select_ppse`], [`scheme_from_aid`], and the APDU/TLV helpers) and
//! register it. Do not patch this file to weaken the masking.

use crate::apdu::Apdu;
use crate::error::{Error, Result};
use crate::hexutil;
use crate::info::CardInfo;
use crate::registry::CardApp;
use crate::tlv::{self, Tlv};
use crate::transport::{transceive, transceive_ok, Transport};

/// PPSE (contactless) and PSE (contact) directory names.
const PPSE: &[u8] = b"2PAY.SYS.DDF01";
const PSE: &[u8] = b"1PAY.SYS.DDF01";

/// Well-known payment AIDs, used both as a registry hint and as a fallback
/// when the PPSE/PSE is missing.
const KNOWN_AIDS: &[&[u8]] = &[
    &[0xA0, 0x00, 0x00, 0x00, 0x03, 0x10, 0x10], // Visa credit/debit
    &[0xA0, 0x00, 0x00, 0x00, 0x03, 0x20, 0x10], // Visa Electron
    &[0xA0, 0x00, 0x00, 0x00, 0x04, 0x10, 0x10], // Mastercard
    &[0xA0, 0x00, 0x00, 0x00, 0x04, 0x30, 0x60], // Maestro
    &[0xA0, 0x00, 0x00, 0x00, 0x25, 0x01, 0x07], // American Express
    &[0xA0, 0x00, 0x00, 0x00, 0x65, 0x10, 0x10], // JCB
    &[0xA0, 0x00, 0x00, 0x01, 0x52, 0x30, 0x10], // Discover
    &[0xA0, 0x00, 0x00, 0x03, 0x33, 0x01, 0x01], // UnionPay
    &[0xA0, 0x00, 0x00, 0x02, 0x77, 0x10, 0x10], // Interac
    &[0xA0, 0x00, 0x00, 0x00, 0x42, 0x10, 0x10], // CB (Cartes Bancaires)
];

/// Tags that must never appear (even in hex) in a field or a dump line.
const REDACTED_TAGS: &[&[u8]] = &[
    &[0x5A],       // Application PAN
    &[0x57],       // Track 2 Equivalent Data
    &[0x56],       // Track 1 Equivalent Data
    &[0x5F, 0x20], // Cardholder name
    &[0x5F, 0x21], // (retired) track 1, alternative
    &[0x9F, 0x1F], // Track 1 discretionary data
    &[0x9F, 0x20], // Track 2 discretionary data
    &[0x9F, 0x6B], // Track 2 data (contactless)
    &[0x9F, 0x62], // PCI-sensitive card data (varies)
];

pub struct EmvApp;

impl CardApp for EmvApp {
    fn name(&self) -> &'static str {
        "emv"
    }

    fn aids(&self) -> &'static [&'static [u8]] {
        KNOWN_AIDS
    }

    fn try_read(&self, card: &mut dyn Transport) -> Result<CardInfo> {
        let directory = select_directory(card);
        let mut candidates: Vec<Vec<u8>> = Vec::new();
        if let Some(ref fci) = directory {
            for aid in tlv::find_all(fci, &[0x4F]) {
                candidates.push(aid.value.clone());
            }
        }
        if candidates.is_empty() {
            for aid in KNOWN_AIDS {
                candidates.push(aid.to_vec());
            }
        }

        let mut last_err: Option<Error> = None;
        for aid in candidates {
            match read_application(card, &aid) {
                Ok(info) => return Ok(info),
                Err(Error::NotApplicable) | Err(Error::Status { .. }) => continue,
                Err(err) => last_err = Some(err),
            }
        }
        Err(last_err.unwrap_or(Error::NotApplicable))
    }

    // Intentionally a no-op. See the module-level docs for why, and for how
    // a downstream crate can add its own CardApp instead of extending this
    // one.
}

/// SELECT the PPSE (preferred) or the PSE and parse the FCI template.
pub fn select_ppse(card: &mut dyn Transport) -> Result<Vec<Tlv>> {
    select_directory(card).ok_or(Error::NotApplicable)
}

fn select_directory(card: &mut dyn Transport) -> Option<Vec<Tlv>> {
    for name in [PPSE, PSE] {
        if let Ok(resp) = transceive_ok(card, &Apdu::select_by_name(name)) {
            if let Ok(tlv) = tlv::parse_all(&resp.data) {
                return Some(tlv);
            }
        }
    }
    None
}

fn read_application(card: &mut dyn Transport, aid: &[u8]) -> Result<CardInfo> {
    let fci_resp = transceive_ok(card, &Apdu::select_by_name(aid))?;
    let fci = tlv::parse_all(&fci_resp.data).unwrap_or_default();

    let mut info = CardInfo::new("emv");
    info.push("aid", hexutil::encode(aid));
    info.push("scheme", scheme_from_aid(aid).unwrap_or("unknown"));

    if let Some(label) = tlv::find(&fci, &[0x50])
        .and_then(|t| hexutil::ascii_if_printable(&t.value))
        .or_else(|| {
            tlv::find(&fci, &[0x9F, 0x12]).and_then(|t| hexutil::ascii_if_printable(&t.value))
        })
    {
        info.push("application_label", label);
    }

    dump_redacted(&mut info, "fci", &fci);

    let records = collect_records(card, &fci);

    // Safe field extraction happens here and only here.
    if let Some(masked) = records.iter().find_map(|t| {
        t.find(&[0x5A])
            .and_then(|pan| mask_pan_from_bcd(&pan.value))
    }) {
        info.push("pan", masked);
    }

    if let Some(exp) = records
        .iter()
        .find_map(|t| t.find(&[0x5F, 0x24]).and_then(|e| format_expiry(&e.value)))
    {
        info.push("expiry", exp);
    }

    // If the label wasn't in the FCI, it may appear in a record.
    if info.field("application_label").is_none() {
        if let Some(label) = records.iter().find_map(|t| {
            t.find(&[0x50])
                .and_then(|v| hexutil::ascii_if_printable(&v.value))
                .or_else(|| {
                    t.find(&[0x9F, 0x12])
                        .and_then(|v| hexutil::ascii_if_printable(&v.value))
                })
        }) {
            info.push("application_label", label);
        }
    }

    for (i, rec) in records.iter().enumerate() {
        dump_redacted(&mut info, format!("record[{i}]"), std::slice::from_ref(rec));
    }

    // A card that selected an AID but produced no safe fields at all is still
    // an EMV card — the scheme/AID themselves are useful.
    Ok(info)
}

fn collect_records(card: &mut dyn Transport, fci: &[Tlv]) -> Vec<Tlv> {
    let mut records = Vec::new();

    // Prefer the AFL from GET PROCESSING OPTIONS when we can issue it.
    if let Some(afl) = try_gpo(card, fci) {
        for (sfi, first, last) in parse_afl(&afl) {
            for recno in first..=last {
                if let Ok(resp) = transceive_ok(card, &Apdu::read_record(sfi, recno)) {
                    if let Ok(mut nodes) = tlv::parse_all(&resp.data) {
                        records.append(&mut nodes);
                    }
                }
            }
        }
        return records;
    }

    // Fallback: walk a conservative range of SFIs and record numbers.
    for sfi in 1..=10u8 {
        for recno in 1..=10u8 {
            match transceive(card, &Apdu::read_record(sfi, recno)) {
                Ok(resp) if resp.is_ok() => {
                    if let Ok(mut nodes) = tlv::parse_all(&resp.data) {
                        records.append(&mut nodes);
                    }
                }
                Ok(resp) if resp.sw() == 0x6A83 || resp.sw() == 0x6A82 => {
                    // record / file not found: stop this SFI
                    if resp.sw() == 0x6A82 {
                        break;
                    }
                    break;
                }
                _ => break,
            }
        }
    }
    records
}

fn try_gpo(card: &mut dyn Transport, fci: &[Tlv]) -> Option<Vec<u8>> {
    let pdol_len = tlv::find(fci, &[0x9F, 0x38])
        .map(|t| pdol_zero_length(&t.value))
        .unwrap_or(0);
    let pdol = vec![0u8; pdol_len];
    let resp = transceive_ok(card, &Apdu::get_processing_options(&pdol)).ok()?;
    let nodes = tlv::parse_all(&resp.data).ok()?;
    if let Some(afl) = tlv::find(&nodes, &[0x94]) {
        return Some(afl.value.clone());
    }
    // Format 1 GPO response: tag 80, AIP (2 bytes) + AFL.
    if let Some(fmt1) = tlv::find(&nodes, &[0x80]) {
        if fmt1.value.len() > 2 {
            return Some(fmt1.value[2..].to_vec());
        }
    }
    // Some cards return the AFL with no wrapper.
    if resp.data.len() > 2 && tlv::find(&nodes, &[0x94]).is_none() {
        // already handled
    }
    None
}

/// Sum the lengths requested by a PDOL list (tag+length pairs, no values).
fn pdol_zero_length(pdol: &[u8]) -> usize {
    let mut i = 0;
    let mut total = 0usize;
    while i < pdol.len() {
        let mut tlen = 1;
        if pdol[i] & 0x1F == 0x1F {
            tlen = 2;
            while i + tlen - 1 < pdol.len() && pdol[i + tlen - 1] & 0x80 != 0 {
                tlen += 1;
            }
        }
        i += tlen;
        if i >= pdol.len() {
            break;
        }
        total += pdol[i] as usize;
        i += 1;
    }
    total
}

/// AFL is a list of 4-byte entries: (SFI << 3), first record, last record, offline count.
fn parse_afl(afl: &[u8]) -> Vec<(u8, u8, u8)> {
    afl.chunks_exact(4)
        .map(|c| {
            let sfi = c[0] >> 3;
            (sfi, c[1], c[2])
        })
        .filter(|(sfi, first, last)| *sfi > 0 && *first > 0 && *last >= *first)
        .collect()
}

/// Map an AID (or its RID prefix) to a scheme name.
pub fn scheme_from_aid(aid: &[u8]) -> Option<&'static str> {
    let rid = if aid.len() >= 5 { &aid[..5] } else { aid };
    Some(match rid {
        [0xA0, 0x00, 0x00, 0x00, 0x03] => "Visa",
        [0xA0, 0x00, 0x00, 0x00, 0x04] => "Mastercard",
        [0xA0, 0x00, 0x00, 0x00, 0x25] => "American Express",
        [0xA0, 0x00, 0x00, 0x00, 0x65] => "JCB",
        [0xA0, 0x00, 0x00, 0x01, 0x52] => "Discover",
        [0xA0, 0x00, 0x00, 0x03, 0x33] => "UnionPay",
        [0xA0, 0x00, 0x00, 0x02, 0x77] => "Interac",
        [0xA0, 0x00, 0x00, 0x00, 0x42] => "Cartes Bancaires",
        [0xA0, 0x00, 0x00, 0x00, 0x29] => "LINK",
        [0xA0, 0x00, 0x00, 0x01, 0x21] => "Dankort",
        [0xA0, 0x00, 0x00, 0x03, 0x71] => "EFTPOS (AU)",
        _ => return None,
    })
}

/// Mask a packed-BCD PAN: last 4 digits visible, everything else `*`.
///
/// The full digit string is never formatted and is cleared before return.
pub fn mask_pan_from_bcd(value: &[u8]) -> Option<String> {
    let mut digits: Vec<u8> = Vec::with_capacity(value.len() * 2);
    'bytes: for byte in value {
        for nibble in [byte >> 4, byte & 0x0F] {
            if nibble <= 9 {
                digits.push(nibble);
            } else if nibble == 0x0F {
                break 'bytes;
            } else {
                return None;
            }
        }
    }
    let n = digits.len();
    // ISO/IEC 7812 PANs are 12–19 digits. Anything shorter is not a PAN.
    if !(12..=19).contains(&n) {
        digits.clear();
        return None;
    }
    let last4 = format!(
        "{}{}{}{}",
        digits[n - 4],
        digits[n - 3],
        digits[n - 2],
        digits[n - 1]
    );
    digits.fill(0);
    digits.clear();
    let mut masked = String::with_capacity(n);
    for _ in 0..(n - 4) {
        masked.push('*');
    }
    masked.push_str(&last4);
    Some(masked)
}

fn format_expiry(value: &[u8]) -> Option<String> {
    // Tag 5F24 is YYMMDD in packed BCD, 3 bytes.
    if value.len() < 2 {
        return None;
    }
    let yy = bcd_byte(value[0])?;
    let mm = bcd_byte(value[1])?;
    if !(1..=12).contains(&mm) {
        return None;
    }
    Some(format!("{mm:02}/{yy:02}"))
}

fn bcd_byte(b: u8) -> Option<u8> {
    let hi = b >> 4;
    let lo = b & 0x0F;
    if hi > 9 || lo > 9 {
        None
    } else {
        Some(hi * 10 + lo)
    }
}

fn is_redacted(tag: &[u8]) -> bool {
    REDACTED_TAGS.iter().any(|t| *t == tag)
}

fn dump_redacted(info: &mut CardInfo, prefix: impl AsRef<str>, nodes: &[Tlv]) {
    let prefix = prefix.as_ref();
    tlv::walk(nodes, |tlv, depth| {
        let indent = "  ".repeat(depth);
        let label = format!("{prefix}.{indent}{}", tlv.tag_hex());
        if is_redacted(&tlv.tag) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ScriptedTransport;
    use crate::Registry;

    #[test]
    fn mask_keeps_only_last_four() {
        // 5412 3456 7890 1234
        let pan = [0x54, 0x12, 0x34, 0x56, 0x78, 0x90, 0x12, 0x34];
        let masked = mask_pan_from_bcd(&pan).unwrap();
        assert_eq!(masked, "************1234");
        assert!(!masked.contains('5'));
        assert!(!masked.contains("5412"));
        assert_eq!(masked.chars().filter(|c| *c == '*').count(), 12);
    }

    #[test]
    fn mask_handles_odd_nibble_padding() {
        // 15-digit PAN ending in 9999, padded with 0xF
        let pan = [0x37, 0x11, 0x11, 0x11, 0x11, 0x19, 0x99, 0x9F];
        let masked = mask_pan_from_bcd(&pan).unwrap();
        assert!(masked.ends_with("9999"));
        assert!(masked.chars().all(|c| c == '*' || c.is_ascii_digit()));
        assert!(!masked.starts_with('3'));
    }

    #[test]
    fn scheme_lookup() {
        assert_eq!(
            scheme_from_aid(&[0xA0, 0x00, 0x00, 0x00, 0x03, 0x10, 0x10]),
            Some("Visa")
        );
        assert_eq!(
            scheme_from_aid(&[0xA0, 0x00, 0x00, 0x00, 0x04, 0x10, 0x10]),
            Some("Mastercard")
        );
    }

    #[test]
    fn expiry_format() {
        assert_eq!(format_expiry(&[0x27, 0x12, 0x31]).as_deref(), Some("12/27"));
        assert_eq!(format_expiry(&[0x27, 0x13, 0x31]), None);
    }

    #[test]
    fn emv_app_masks_pan_from_a_scripted_card() {
        // SELECT AID -> FCI with label, then READ RECORD SFI 1 rec 1 with
        // a 70 template containing 5A (PAN) and 5F24 (expiry).
        let aid: &[u8] = &[0xA0, 0x00, 0x00, 0x00, 0x03, 0x10, 0x10];
        let mut fci = vec![0x6F, 0x12, 0x84, 0x07];
        fci.extend_from_slice(aid);
        fci.extend_from_slice(&[0xA5, 0x07, 0x50, 0x05, b'V', b'I', b'S', b'A', b' ']);

        // record: 70 0E  5A 08 54 12 34 56 78 90 12 34  5F 24 03 27 12 31
        let rec = vec![
            0x70, 0x10, 0x5A, 0x08, 0x54, 0x12, 0x34, 0x56, 0x78, 0x90, 0x12, 0x34, 0x5F, 0x24,
            0x03, 0x27, 0x12, 0x31,
        ];

        let mut sel = Apdu::select_by_name(aid).to_bytes().unwrap();
        let mut card = ScriptedTransport::new(vec![0x3B, 0x00]);
        // PPSE / PSE fail
        // AID select succeeds
        let mut fci_resp = fci.clone();
        fci_resp.extend_from_slice(&[0x90, 0x00]);
        card = card.on(sel.clone(), fci_resp);

        // GPO will fail (default 6A82)
        // READ RECORD SFI 1 rec 1
        let mut rec_resp = rec.clone();
        rec_resp.extend_from_slice(&[0x90, 0x00]);
        card = card.on(Apdu::read_record(1, 1).to_bytes().unwrap(), rec_resp);

        // also handle known-AID fallback (same select)
        let _ = sel.pop();

        let mut registry = Registry::new();
        registry.register(EmvApp);
        let infos = registry.read_all(&mut card);
        assert_eq!(infos.len(), 1, "expected one EMV result, got {infos:?}");
        let info = &infos[0];
        assert_eq!(info.field("scheme"), Some("Visa"));
        assert_eq!(info.field("application_label"), Some("VISA "));
        assert_eq!(info.field("pan"), Some("************1234"));
        assert_eq!(info.field("expiry"), Some("12/27"));
        // Full PAN must not leak into fields or the dump.
        let rendered = info.to_string();
        assert!(!rendered.contains("5412345678901234"));
        assert!(!rendered.contains("54 12 34 56 78 90 12 34"));
        assert!(rendered.contains("[redacted]"));
    }
}
