use crate::error::{Error, Result};

/// A command APDU (ISO 7816-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Apdu {
    pub cla: u8,
    pub ins: u8,
    pub p1: u8,
    pub p2: u8,
    pub data: Vec<u8>,
    /// Expected response length. `Some(0)` encodes Le = 0 (up to 256 bytes).
    pub le: Option<u8>,
}

impl Apdu {
    pub fn new(cla: u8, ins: u8, p1: u8, p2: u8) -> Self {
        Self {
            cla,
            ins,
            p1,
            p2,
            data: Vec::new(),
            le: Some(0),
        }
    }

    pub fn with_data(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.data = data.into();
        self
    }

    pub fn with_le(mut self, le: Option<u8>) -> Self {
        self.le = le;
        self
    }

    /// SELECT by name / AID (INS 0xA4, P1 0x04, P2 0x00).
    pub fn select_by_name(name: &[u8]) -> Self {
        Self::new(0x00, 0xA4, 0x04, 0x00)
            .with_data(name.to_vec())
            .with_le(Some(0))
    }

    /// SELECT by file identifier (INS 0xA4, P1 0x00, P2 0x00).
    pub fn select_by_fid(fid: [u8; 2]) -> Self {
        Self::new(0x00, 0xA4, 0x00, 0x00)
            .with_data(fid.to_vec())
            .with_le(Some(0))
    }

    /// SELECT the master file (`3F00`).
    pub fn select_mf() -> Self {
        Self::select_by_fid([0x3F, 0x00])
    }

    /// READ RECORD. `sfi` is the short file identifier (1..=31); `record` is 1-based.
    pub fn read_record(sfi: u8, record: u8) -> Self {
        let p2 = (sfi << 3) | 0x04; // read record P2: SFI + "P1 is a record number"
        Self::new(0x00, 0xB2, record, p2).with_le(Some(0))
    }

    /// GET DATA for a BER tag (INS 0xCA). Two-byte tags go in P1-P2.
    pub fn get_data(tag: u16) -> Self {
        Self::new(0x00, 0xCA, (tag >> 8) as u8, tag as u8).with_le(Some(0))
    }

    /// GET RESPONSE (used after SW 61xx).
    pub fn get_response(le: u8) -> Self {
        Self::new(0x00, 0xC0, 0x00, 0x00).with_le(Some(le))
    }

    /// ISO-wrapped native command used by MIFARE DESFire (`CLA = 0x90`).
    pub fn desfire(ins: u8, data: &[u8]) -> Self {
        Self::new(0x90, ins, 0x00, 0x00)
            .with_data(data.to_vec())
            .with_le(Some(0))
    }

    /// GET PROCESSING OPTIONS. `pdol_data` is the already-encoded PDOL payload
    /// (the value of tag `83`), not including the `83` header.
    pub fn get_processing_options(pdol_data: &[u8]) -> Self {
        let mut data = Vec::with_capacity(2 + pdol_data.len());
        data.push(0x83);
        data.push(pdol_data.len() as u8);
        data.extend_from_slice(pdol_data);
        Self::new(0x80, 0xA8, 0x00, 0x00)
            .with_data(data)
            .with_le(Some(0))
    }

    /// Encode as a short-length command APDU (Lc / Le each a single byte).
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if self.data.len() > 255 {
            return Err(Error::Apdu(
                "extended-length APDUs are not implemented".into(),
            ));
        }
        let mut out = vec![self.cla, self.ins, self.p1, self.p2];
        if !self.data.is_empty() {
            out.push(self.data.len() as u8);
            out.extend_from_slice(&self.data);
        }
        if let Some(le) = self.le {
            out.push(le);
        }
        Ok(out)
    }
}

/// A response APDU: body plus SW1-SW2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApduResponse {
    pub data: Vec<u8>,
    pub sw1: u8,
    pub sw2: u8,
}

impl ApduResponse {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 2 {
            return Err(Error::Apdu("response shorter than 2 bytes".into()));
        }
        let (data, sw) = bytes.split_at(bytes.len() - 2);
        Ok(Self {
            data: data.to_vec(),
            sw1: sw[0],
            sw2: sw[1],
        })
    }

    pub fn sw(&self) -> u16 {
        ((self.sw1 as u16) << 8) | self.sw2 as u16
    }

    pub fn is_ok(&self) -> bool {
        self.sw() == 0x9000 || self.sw() == 0x9100
    }

    /// SW 61xx: more data available, `sw2` is the Le to use with GET RESPONSE.
    pub fn more_bytes(&self) -> Option<u8> {
        (self.sw1 == 0x61).then_some(self.sw2)
    }

    /// SW 6Cxx: wrong Le, retry with `sw2`.
    pub fn wrong_le(&self) -> Option<u8> {
        (self.sw1 == 0x6C).then_some(self.sw2)
    }

    /// DESFire ISO-wrap success is `91xx`, with `91AF` meaning "additional frame".
    pub fn desfire_more(&self) -> bool {
        self.sw() == 0x91AF
    }

    pub fn ok(self) -> Result<Self> {
        if self.is_ok() || self.more_bytes().is_some() || self.desfire_more() {
            Ok(self)
        } else {
            Err(Error::Status {
                sw1: self.sw1,
                sw2: self.sw2,
            })
        }
    }
}

impl std::fmt::Display for ApduResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} SW={:02X}{:02X}",
            crate::hexutil::encode_pretty(&self.data),
            self.sw1,
            self.sw2
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_by_name_encoding() {
        let aid = b"1PAY.SYS.DDF01";
        let bytes = Apdu::select_by_name(aid).to_bytes().unwrap();
        assert_eq!(bytes[0], 0x00);
        assert_eq!(bytes[1], 0xA4);
        assert_eq!(bytes[2], 0x04);
        assert_eq!(bytes[3], 0x00);
        assert_eq!(bytes[4], aid.len() as u8);
        assert_eq!(&bytes[5..5 + aid.len()], aid);
        assert_eq!(*bytes.last().unwrap(), 0x00);
    }

    #[test]
    fn read_record_p2() {
        let bytes = Apdu::read_record(1, 1).to_bytes().unwrap();
        assert_eq!(bytes, vec![0x00, 0xB2, 0x01, 0x0C, 0x00]);
    }

    #[test]
    fn response_parse() {
        let r = ApduResponse::from_bytes(&[0x6F, 0x00, 0x90, 0x00]).unwrap();
        assert!(r.is_ok());
        assert_eq!(r.data, vec![0x6F, 0x00]);
        let m = ApduResponse::from_bytes(&[0x61, 0x1C]).unwrap();
        assert_eq!(m.more_bytes(), Some(0x1C));
    }
}
