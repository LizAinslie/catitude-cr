use crate::error::{Error, Result};
use crate::hexutil;

/// Parsed Answer-to-Reset, plus a best-effort identification string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Atr {
    pub raw: Vec<u8>,
    pub convention: Convention,
    pub historical: Vec<u8>,
    pub protocols: Vec<u8>,
    pub default_protocol: Option<u8>,
    /// Human-readable identification assembled from historical bytes and a
    /// small built-in prefix table.
    pub identification: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convention {
    Direct,  // TS = 0x3B
    Inverse, // TS = 0x3F
}

impl Atr {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() < 2 {
            return Err(Error::Other("ATR shorter than 2 bytes".into()));
        }
        let convention = match raw[0] {
            0x3B => Convention::Direct,
            0x3F => Convention::Inverse,
            other => return Err(Error::Other(format!("unknown ATR TS {other:02X}"))),
        };
        let t0 = raw[1];
        let mut historical_len = (t0 & 0x0F) as usize;
        let mut interface_presence = t0 & 0xF0;
        let mut idx = 2;
        let mut protocols = Vec::new();
        let mut tdi_seen = false;
        let mut default_protocol = None;

        // Interface bytes: for each TDi, Y flags in the high nibble say
        // whether TA(i+1)..TD(i+1) follow.
        loop {
            if interface_presence & 0x10 != 0 {
                idx += 1;
            }
            if interface_presence & 0x20 != 0 {
                idx += 1;
            }
            if interface_presence & 0x40 != 0 {
                idx += 1;
            }
            if interface_presence & 0x80 != 0 {
                if idx >= raw.len() {
                    return Err(Error::Other("truncated ATR interface bytes".into()));
                }
                let td = raw[idx];
                idx += 1;
                let proto = td & 0x0F;
                protocols.push(proto);
                if !tdi_seen {
                    default_protocol = Some(proto);
                    tdi_seen = true;
                }
                interface_presence = td & 0xF0;
            } else {
                break;
            }
            if idx > raw.len() {
                return Err(Error::Other("truncated ATR interface bytes".into()));
            }
        }

        if idx + historical_len > raw.len() {
            // Some readers omit TCK or clip historical bytes; take what's left
            // minus an optional TCK.
            historical_len = raw.len().saturating_sub(idx);
        }
        let historical = raw[idx..idx + historical_len.min(raw.len() - idx)].to_vec();

        let identification = identify(raw, &historical);
        Ok(Self {
            raw: raw.to_vec(),
            convention,
            historical,
            protocols,
            default_protocol,
            identification,
        })
    }
}

fn identify(raw: &[u8], historical: &[u8]) -> String {
    if let Some(name) = known_prefix(raw) {
        return name.to_string();
    }
    if let Some(from_hist) = identify_historical(historical) {
        return from_hist;
    }
    if let Some(ascii) = hexutil::ascii_if_printable(historical) {
        return format!("historical: {ascii}");
    }
    if historical.is_empty() {
        "unknown ATR".into()
    } else {
        format!("historical {}", hexutil::encode_pretty(historical))
    }
}

fn known_prefix(raw: &[u8]) -> Option<&'static str> {
    // Prefix matches, longest first. These are identifiers, not parsers.
    const TABLE: &[(&[u8], &str)] = &[
        (
            &[
                0x3B, 0x8F, 0x80, 0x01, 0x80, 0x4F, 0x0C, 0xA0, 0x00, 0x00, 0x03, 0x06,
            ],
            "PC/SC compatible contactless (ISO 14443, PC/SC 2.01 payload)",
        ),
        (
            &[
                0x3B, 0x8C, 0x80, 0x01, 0x50, 0x43, 0x50, 0x41, 0x59, 0x49, 0x43,
            ],
            "PC/SC contactless (PCPAYIC)",
        ),
        (
            &[0x3B, 0x67, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            "GEMALTO / generic EMV",
        ),
        (
            &[0x3B, 0x6E, 0x00, 0x00, 0x80, 0x31, 0x80, 0x66],
            "Giesecke+Devrient / T=0",
        ),
        (&[0x3B, 0xF8, 0x13, 0x00, 0x00, 0x81, 0x31], "NXP / JCOP"),
        (
            &[0x3B, 0x8A, 0x80, 0x01, 0x00],
            "Contactless Type A (short ATR)",
        ),
        (
            &[0x3B, 0x81, 0x80, 0x01, 0x80, 0x80],
            "MIFARE DESFire (typical)",
        ),
        (&[0x3B, 0x81, 0x80, 0x01, 0x80], "ISO 14443-4 Type A"),
    ];
    for (prefix, name) in TABLE {
        if raw.starts_with(prefix) {
            return Some(*name);
        }
    }
    None
}

/// ISO 7816-4 compact-TLV in historical bytes, when the category indicator
/// is 0x00 or 0x80.
fn identify_historical(historical: &[u8]) -> Option<String> {
    if historical.is_empty() {
        return None;
    }
    let (category, body) = historical.split_first()?;
    let body = match *category {
        0x00 => body,
        0x80 => {
            // last 3 bytes are often a status indicator; compact-TLV before that
            if body.len() >= 3 {
                &body[..body.len() - 3]
            } else {
                body
            }
        }
        _ => {
            return hexutil::ascii_if_printable(historical);
        }
    };
    let mut i = 0;
    let mut parts = Vec::new();
    while i < body.len() {
        let hi = body[i] >> 4;
        let len = (body[i] & 0x0F) as usize;
        i += 1;
        if i + len > body.len() {
            break;
        }
        let value = &body[i..i + len];
        i += len;
        match hi {
            0x4 => {
                // issuer / card name, often ASCII
                if let Some(s) = hexutil::ascii_if_printable(value) {
                    parts.push(s);
                }
            }
            0x5 => {
                parts.push(format!("AID {}", hexutil::encode(value)));
            }
            0x2 => {
                if let Some(s) = hexutil::ascii_if_printable(value) {
                    parts.push(format!("issuer {s}"));
                }
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_direct_no_interface() {
        // TS, T0 (K=5, no interface), 5 historical, no TCK required here
        let raw = [0x3B, 0x05, b'H', b'E', b'L', b'L', b'O'];
        let atr = Atr::parse(&raw).unwrap();
        assert_eq!(atr.convention, Convention::Direct);
        assert_eq!(atr.historical, b"HELLO");
        assert!(atr.identification.contains("HELLO"));
    }
}
