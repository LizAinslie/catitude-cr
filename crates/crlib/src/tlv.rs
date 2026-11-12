use crate::error::{Error, Result};

/// One BER-TLV object. Constructed tags have `children` filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tlv {
    pub tag: Vec<u8>,
    pub value: Vec<u8>,
    pub children: Vec<Tlv>,
}

impl Tlv {
    pub fn is_constructed(&self) -> bool {
        tag_is_constructed(&self.tag)
    }

    pub fn tag_hex(&self) -> String {
        crate::hexutil::encode(&self.tag)
    }

    pub fn value_hex(&self) -> String {
        crate::hexutil::encode_pretty(&self.value)
    }

    /// Depth-first search for the first object with this tag.
    pub fn find<'a>(&'a self, tag: &[u8]) -> Option<&'a Tlv> {
        if self.tag == tag {
            return Some(self);
        }
        for child in &self.children {
            if let Some(found) = child.find(tag) {
                return Some(found);
            }
        }
        None
    }

    /// All objects with this tag, depth-first.
    pub fn find_all<'a>(&'a self, tag: &[u8]) -> Vec<&'a Tlv> {
        let mut out = Vec::new();
        collect(self, tag, &mut out);
        out
    }
}

fn collect<'a>(tlv: &'a Tlv, tag: &[u8], out: &mut Vec<&'a Tlv>) {
    if tlv.tag == tag {
        out.push(tlv);
    }
    for child in &tlv.children {
        collect(child, tag, out);
    }
}

pub fn tag_is_constructed(tag: &[u8]) -> bool {
    !tag.is_empty() && tag[0] & 0x20 != 0
}

/// Parse a single BER-TLV object, returning it and the unparsed remainder.
pub fn parse_one(input: &[u8]) -> Result<(Tlv, &[u8])> {
    let (tag, rest) = parse_tag(input)?;
    let (len, rest) = parse_len(rest)?;
    if rest.len() < len {
        return Err(Error::Tlv(format!(
            "tag {} length {len} exceeds remaining {}",
            crate::hexutil::encode(&tag),
            rest.len()
        )));
    }
    let (value, rest) = rest.split_at(len);
    let children = if tag_is_constructed(&tag) {
        parse_all(value).unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok((
        Tlv {
            tag,
            value: value.to_vec(),
            children,
        },
        rest,
    ))
}

/// Parse a sequence of concatenated BER-TLV objects.
pub fn parse_all(mut input: &[u8]) -> Result<Vec<Tlv>> {
    let mut out = Vec::new();
    while !input.is_empty() {
        // Padding bytes 0x00 and 0xFF show up between records on some cards.
        if input[0] == 0x00 || input[0] == 0xFF {
            input = &input[1..];
            continue;
        }
        let (tlv, rest) = parse_one(input)?;
        out.push(tlv);
        input = rest;
    }
    Ok(out)
}

/// Search a list of sibling objects (and their descendants) for a tag.
pub fn find<'a>(nodes: &'a [Tlv], tag: &[u8]) -> Option<&'a Tlv> {
    for n in nodes {
        if let Some(found) = n.find(tag) {
            return Some(found);
        }
    }
    None
}

pub fn find_all<'a>(nodes: &'a [Tlv], tag: &[u8]) -> Vec<&'a Tlv> {
    let mut out = Vec::new();
    for n in nodes {
        out.extend(n.find_all(tag));
    }
    out
}

fn parse_tag(input: &[u8]) -> Result<(Vec<u8>, &[u8])> {
    if input.is_empty() {
        return Err(Error::Tlv("empty tag".into()));
    }
    let b0 = input[0];
    let mut tag = vec![b0];
    let mut rest = &input[1..];
    if b0 & 0x1F == 0x1F {
        loop {
            if rest.is_empty() {
                return Err(Error::Tlv("truncated multi-byte tag".into()));
            }
            let b = rest[0];
            tag.push(b);
            rest = &rest[1..];
            if b & 0x80 == 0 {
                break;
            }
        }
    }
    Ok((tag, rest))
}

fn parse_len(input: &[u8]) -> Result<(usize, &[u8])> {
    if input.is_empty() {
        return Err(Error::Tlv("empty length".into()));
    }
    let b = input[0];
    if b & 0x80 == 0 {
        return Ok((b as usize, &input[1..]));
    }
    let nbytes = (b & 0x7F) as usize;
    if nbytes == 0 {
        return Err(Error::Tlv(
            "indefinite lengths are not used in card BER-TLV".into(),
        ));
    }
    if nbytes > 4 || input.len() < 1 + nbytes {
        return Err(Error::Tlv("invalid long-form length".into()));
    }
    let mut len = 0usize;
    for i in 1..=nbytes {
        len = (len << 8) | (input[i] as usize);
    }
    Ok((len, &input[1 + nbytes..]))
}

/// Walk every node, calling `f` with each object and its depth.
pub fn walk(nodes: &[Tlv], mut f: impl FnMut(&Tlv, usize)) {
    fn rec(nodes: &[Tlv], depth: usize, f: &mut impl FnMut(&Tlv, usize)) {
        for n in nodes {
            f(n, depth);
            rec(&n.children, depth + 1, f);
        }
    }
    rec(nodes, 0, &mut f);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        // tag 50, length 4, "VISA"
        let (tlv, rest) = parse_one(&[0x50, 0x04, b'V', b'I', b'S', b'A']).unwrap();
        assert!(rest.is_empty());
        assert_eq!(tlv.tag, vec![0x50]);
        assert_eq!(tlv.value, b"VISA");
        assert!(tlv.children.is_empty());
    }

    #[test]
    fn parse_constructed_and_two_byte_tag() {
        // A typical (shortened) FCI: 6F 0A  84 02 A0 00  A5 04  5F 2D 02 65 6E
        // 6F 0B  84 02 A000  A5 05  5F2D 02 "en"
        let bytes = [
            0x6F, 0x0B, 0x84, 0x02, 0xA0, 0x00, 0xA5, 0x05, 0x5F, 0x2D, 0x02, b'e', b'n',
        ];
        let nodes = parse_all(&bytes).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].tag, vec![0x6F]);
        assert!(nodes[0].find(&[0x84]).is_some());
        let lang = nodes[0].find(&[0x5F, 0x2D]).unwrap();
        assert_eq!(lang.value, b"en");
    }

    #[test]
    fn long_form_length() {
        let mut bytes = vec![0x50, 0x81, 0x05];
        bytes.extend_from_slice(b"hello");
        let (tlv, _) = parse_one(&bytes).unwrap();
        assert_eq!(tlv.value, b"hello");
    }
}
