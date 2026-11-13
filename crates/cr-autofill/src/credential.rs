use crlib::apdu::Apdu;
use crlib::apps::scheme_from_aid;
use crlib::error::{Error, Result};
use crlib::tlv::{self, Tlv};
use crlib::transport::{transceive, transceive_ok, Transport};

const PPSE: &[u8] = b"2PAY.SYS.DDF01";
const PSE: &[u8] = b"1PAY.SYS.DDF01";

const KNOWN_AIDS: &[&[u8]] = &[
    &[0xA0, 0x00, 0x00, 0x00, 0x03, 0x10, 0x10],
    &[0xA0, 0x00, 0x00, 0x00, 0x03, 0x20, 0x10],
    &[0xA0, 0x00, 0x00, 0x00, 0x04, 0x10, 0x10],
    &[0xA0, 0x00, 0x00, 0x00, 0x04, 0x30, 0x60],
    &[0xA0, 0x00, 0x00, 0x00, 0x25, 0x01, 0x07],
    &[0xA0, 0x00, 0x00, 0x00, 0x65, 0x10, 0x10],
    &[0xA0, 0x00, 0x00, 0x01, 0x52, 0x30, 0x10],
    &[0xA0, 0x00, 0x00, 0x03, 0x33, 0x01, 0x01],
    &[0xA0, 0x00, 0x00, 0x02, 0x77, 0x10, 0x10],
    &[0xA0, 0x00, 0x00, 0x00, 0x42, 0x10, 0x10],
];

/// A card credential that can be autofilled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Credential {
    pub scheme: String,
    pub pan: String,
    pub expiry_month: Option<u8>,
    pub expiry_year: Option<u16>,
    pub cardholder: Option<String>,
}

impl Credential {
    /// `MM/YY`, or `None` when the expiry is unknown.
    pub fn expiry(&self) -> Option<String> {
        let month = self.expiry_month?;
        let year = self.expiry_year? % 100;
        Some(format!("{month:02}/{year:02}"))
    }

    pub fn field(&self, name: &str) -> Option<String> {
        match name {
            "pan" => Some(self.pan.clone()),
            "expiry" => self.expiry(),
            "month" => self.expiry_month.map(|m| format!("{m:02}")),
            "year" => self.expiry_year.map(|y| format!("{y:02}")),
            "name" => self.cardholder.clone(),
            _ => None,
        }
    }
}

/// Read a payment credential from a card, or `None` when the card is not a
/// payment card (or does not expose a PAN).
pub fn read_credential(card: &mut dyn Transport) -> Result<Option<Credential>> {
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
            Ok(Some(cred)) => return Ok(Some(cred)),
            Ok(None) => continue,
            Err(Error::NotApplicable) | Err(Error::Status { .. }) => continue,
            Err(err) => last_err = Some(err),
        }
    }
    if let Some(err) = last_err {
        return Err(err);
    }
    Ok(None)
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

fn read_application(card: &mut dyn Transport, aid: &[u8]) -> Result<Option<Credential>> {
    let fci_resp = transceive_ok(card, &Apdu::select_by_name(aid))?;
    let fci = tlv::parse_all(&fci_resp.data).unwrap_or_default();
    let records = collect_records(card, &fci);

    let pan = records
        .iter()
        .find_map(|t| t.find(&[0x5A]).and_then(|v| pan_from_bcd(&v.value)))
        .or_else(|| {
            records
                .iter()
                .find_map(|t| t.find(&[0x57]).and_then(|v| pan_from_track2(&v.value)))
        });

    let Some(pan) = pan else {
        return Ok(None);
    };

    let expiry = records
        .iter()
        .find_map(|t| t.find(&[0x5F, 0x24]).and_then(|v| expiry_from_5f24(&v.value)))
        .or_else(|| {
            records
                .iter()
                .find_map(|t| t.find(&[0x57]).and_then(|v| expiry_from_track2(&v.value)))
        });
    let (month, year) = match expiry {
        Some((m, y)) => (Some(m), Some(y)),
        None => (None, None),
    };

    let cardholder = records
        .iter()
        .find_map(|t| t.find(&[0x5F, 0x20]).and_then(|v| ascii(&v.value)))
        .or_else(|| {
            records
                .iter()
                .find_map(|t| t.find(&[0x56]).and_then(|v| name_from_track1(&v.value)))
        });

    Ok(Some(Credential {
        scheme: scheme_from_aid(aid).unwrap_or("unknown").to_string(),
        pan,
        expiry_month: month,
        expiry_year: year,
        cardholder,
    }))
}

fn collect_records(card: &mut dyn Transport, fci: &[Tlv]) -> Vec<Tlv> {
    let mut records = Vec::new();

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

    for sfi in 1..=10u8 {
        for recno in 1..=10u8 {
            match transceive(card, &Apdu::read_record(sfi, recno)) {
                Ok(resp) if resp.is_ok() => {
                    if let Ok(mut nodes) = tlv::parse_all(&resp.data) {
                        records.append(&mut nodes);
                    }
                }
                Ok(_) => break,
                Err(Error::Status { .. }) => break,
                Err(_) => break,
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
    if let Some(fmt1) = tlv::find(&nodes, &[0x80]) {
        if fmt1.value.len() > 2 {
            return Some(fmt1.value[2..].to_vec());
        }
    }
    None
}

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

fn parse_afl(afl: &[u8]) -> Vec<(u8, u8, u8)> {
    afl.chunks_exact(4)
        .map(|c| (c[0] >> 3, c[1], c[2]))
        .filter(|(sfi, first, last)| *sfi > 0 && *first > 0 && *last >= *first)
        .collect()
}

fn pan_from_bcd(value: &[u8]) -> Option<String> {
    let mut digits = Vec::with_capacity(value.len() * 2);
    'bytes: for byte in value {
        for nibble in [byte >> 4, byte & 0x0F] {
            if nibble <= 9 {
                digits.push(nibble);
            } else {
                break 'bytes;
            }
        }
    }
    let n = digits.len();
    if !(12..=19).contains(&n) {
        return None;
    }
    Some(
        digits
            .into_iter()
            .map(|d| (b'0' + d) as char)
            .collect(),
    )
}

fn pan_from_track2(value: &[u8]) -> Option<String> {
    let end = value
        .iter()
        .position(|&b| b == b'D' || b == b'=')
        .unwrap_or(value.len());
    let pan = &value[..end];
    if pan.len() < 12 || !pan.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(String::from_utf8_lossy(pan).into_owned())
}

fn expiry_from_5f24(value: &[u8]) -> Option<(u8, u16)> {
    if value.len() < 2 {
        return None;
    }
    let yy = bcd_byte(value[0])?;
    let mm = bcd_byte(value[1])?;
    if !(1..=12).contains(&mm) {
        return None;
    }
    Some((mm, yy as u16))
}

fn expiry_from_track2(value: &[u8]) -> Option<(u8, u16)> {
    let sep = value.iter().position(|&b| b == b'D' || b == b'=')?;
    let exp = &value[sep + 1..];
    if exp.len() < 4 {
        return None;
    }
    let yy: u16 = std::str::from_utf8(&exp[0..2]).ok()?.parse().ok()?;
    let mm: u8 = std::str::from_utf8(&exp[2..4]).ok()?.parse().ok()?;
    if !(1..=12).contains(&mm) {
        return None;
    }
    Some((mm, yy))
}

fn name_from_track1(value: &[u8]) -> Option<String> {
    let start = value.iter().position(|&b| b == b'^')?;
    let rest = &value[start + 1..];
    let end = rest.iter().position(|&b| b == b'^')?;
    ascii(&rest[..end])
}

fn ascii(value: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(value).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crlib::ScriptedTransport;

    #[test]
    fn bcd_pan_full() {
        let pan = [0x54, 0x12, 0x34, 0x56, 0x78, 0x90, 0x12, 0x34];
        assert_eq!(pan_from_bcd(&pan).as_deref(), Some("5412345678901234"));
    }

    #[test]
    fn bcd_pan_strips_padding() {
        let pan = [0x37, 0x11, 0x11, 0x11, 0x11, 0x19, 0x99, 0x9F];
        assert_eq!(pan_from_bcd(&pan).as_deref(), Some("371111111119999"));
    }

    #[test]
    fn track2_pan_and_expiry() {
        let t2 = b"5412345678901234D2712101000000000";
        assert_eq!(pan_from_track2(t2).as_deref(), Some("5412345678901234"));
        assert_eq!(expiry_from_track2(t2), Some((12, 27)));
    }

    #[test]
    fn track1_name() {
        let t1 = b"B5412345678901234^DOE/JOHN^271210100000000";
        assert_eq!(name_from_track1(t1).as_deref(), Some("DOE/JOHN"));
    }

    #[test]
    fn reads_full_credential_from_scripted_card() {
        let aid: &[u8] = &[0xA0, 0x00, 0x00, 0x00, 0x03, 0x10, 0x10];
        let mut fci = vec![0x6F, 0x12, 0x84, 0x07];
        fci.extend_from_slice(aid);
        fci.extend_from_slice(&[0xA5, 0x07, 0x50, 0x05, b'V', b'I', b'S', b'A', b' ']);

        let rec = vec![
            0x70, 0x1B, 0x5A, 0x08, 0x54, 0x12, 0x34, 0x56, 0x78, 0x90, 0x12, 0x34, 0x5F, 0x24,
            0x03, 0x27, 0x12, 0x31, 0x5F, 0x20, 0x08, b'D', b'O', b'E', b'/', b'J', b'O', b'H',
            b'N',
        ];

        let mut card = ScriptedTransport::new(vec![0x3B, 0x00]);

        let mut fci_resp = fci.clone();
        fci_resp.extend_from_slice(&[0x90, 0x00]);
        card = card.on(Apdu::select_by_name(aid).to_bytes().unwrap(), fci_resp);

        let mut rec_resp = rec.clone();
        rec_resp.extend_from_slice(&[0x90, 0x00]);
        card = card.on(Apdu::read_record(1, 1).to_bytes().unwrap(), rec_resp);

        let cred = read_credential(&mut card).unwrap().unwrap();
        assert_eq!(cred.pan, "5412345678901234");
        assert_eq!(cred.expiry(), Some("12/27".to_string()));
        assert_eq!(cred.cardholder.as_deref(), Some("DOE/JOHN"));
        assert_eq!(cred.scheme, "Visa");
    }
}
