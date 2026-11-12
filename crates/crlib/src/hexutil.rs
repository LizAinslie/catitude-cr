/// Encode bytes as an uppercase hex string, no separators.
pub fn encode(bytes: &[u8]) -> String {
    hex_with_sep(bytes, "")
}

/// Encode bytes as uppercase hex, space-separated (`"3B 8F 80"`).
pub fn encode_pretty(bytes: &[u8]) -> String {
    hex_with_sep(bytes, " ")
}

fn hex_with_sep(bytes: &[u8], sep: &str) -> String {
    let mut out = String::with_capacity(bytes.len() * (2 + sep.len()));
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push_str(sep);
        }
        out.push(nibble(b >> 4));
        out.push(nibble(b & 0x0f));
    }
    out
}

fn nibble(n: u8) -> char {
    match n {
        0..=9 => char::from(b'0' + n),
        10..=15 => char::from(b'A' + (n - 10)),
        _ => '?',
    }
}

/// Decode a hex string, ignoring whitespace and a leading `0x`.
pub fn decode(s: &str) -> crate::error::Result<Vec<u8>> {
    let s = s.trim();
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    let mut cleaned = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_hexdigit() {
            cleaned.push(c);
        } else if c.is_ascii_whitespace() || c == ':' || c == '-' {
            continue;
        } else {
            return Err(crate::error::Error::Other(format!(
                "invalid hex character {c:?}"
            )));
        }
    }
    if cleaned.len() % 2 != 0 {
        return Err(crate::error::Error::Other(
            "odd number of hex digits".into(),
        ));
    }
    let mut out = Vec::with_capacity(cleaned.len() / 2);
    let bytes = cleaned.as_bytes();
    for chunk in bytes.chunks_exact(2) {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_val(b: u8) -> crate::error::Result<u8> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(crate::error::Error::Other("invalid hex".into())),
    }
}

/// Printable ASCII if every byte is in `0x20..=0x7E`; otherwise `None`.
pub fn ascii_if_printable(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || !bytes.iter().all(|b| (0x20..=0x7E).contains(b)) {
        return None;
    }
    Some(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let bytes = [0x3b, 0x8f, 0x00, 0xff];
        assert_eq!(encode(&bytes), "3B8F00FF");
        assert_eq!(encode_pretty(&bytes), "3B 8F 00 FF");
        assert_eq!(decode("3B 8F:00-FF").unwrap(), bytes);
    }
}
