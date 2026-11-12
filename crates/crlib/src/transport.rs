use crate::apdu::{Apdu, ApduResponse};
use crate::error::Result;

/// Byte-oriented smart-card transport.
///
/// Implemented by the PC/SC backend and by a scripted mock used in tests.
/// APDU retry / GET-RESPONSE handling lives in [`transceive`], not in the
/// trait, so alternative transports only have to ship one command at a time.
pub trait Transport {
    fn reader_name(&self) -> &str;
    fn atr(&self) -> &[u8];
    fn transmit(&mut self, command: &[u8]) -> Result<ApduResponse>;
}

/// Transmit an APDU, transparently handling SW `6Cxx` (wrong Le) and `61xx`
/// (GET RESPONSE).
pub fn transceive(card: &mut dyn Transport, command: &Apdu) -> Result<ApduResponse> {
    let mut resp = card.transmit(&command.to_bytes()?)?;

    if let Some(le) = resp.wrong_le() {
        let retry = command.clone().with_le(Some(le));
        resp = card.transmit(&retry.to_bytes()?)?;
    }

    let mut data = resp.data.clone();
    while let Some(le) = resp.more_bytes() {
        let get = Apdu::get_response(le);
        resp = card.transmit(&get.to_bytes()?)?;
        data.extend_from_slice(&resp.data);
    }

    // DESFire ISO wrapping uses 91AF for "additional frame". The next
    // command is 90 AF 00 00 00.
    while resp.desfire_more() {
        let more = Apdu::desfire(0xAF, &[]);
        resp = card.transmit(&more.to_bytes()?)?;
        data.extend_from_slice(&resp.data);
    }

    resp.data = data;
    Ok(resp)
}

/// [`transceive`] and require a success status (`9000` or `9100`).
pub fn transceive_ok(card: &mut dyn Transport, command: &Apdu) -> Result<ApduResponse> {
    transceive(card, command)?.ok()
}

/// A `Transport` that replies from a script of exact command → response pairs.
/// Commands not in the script return `Error::NotApplicable` when `strict` is
/// false, or `Error::Apdu` when `strict` is true.
#[derive(Debug, Clone)]
pub struct ScriptedTransport {
    reader: String,
    atr: Vec<u8>,
    /// Remaining (command, response) pairs, consumed in order when
    /// `in_order` is true; otherwise matched by exact command bytes.
    script: Vec<(Vec<u8>, Vec<u8>)>,
    in_order: bool,
    pub strict: bool,
}

impl ScriptedTransport {
    pub fn new(atr: impl Into<Vec<u8>>) -> Self {
        Self {
            reader: "scripted".into(),
            atr: atr.into(),
            script: Vec::new(),
            in_order: false,
            strict: false,
        }
    }

    pub fn in_order(mut self) -> Self {
        self.in_order = true;
        self
    }

    pub fn on(mut self, command: impl Into<Vec<u8>>, response: impl Into<Vec<u8>>) -> Self {
        self.script.push((command.into(), response.into()));
        self
    }
}

impl Transport for ScriptedTransport {
    fn reader_name(&self) -> &str {
        &self.reader
    }

    fn atr(&self) -> &[u8] {
        &self.atr
    }

    fn transmit(&mut self, command: &[u8]) -> Result<ApduResponse> {
        if self.in_order {
            if self.script.is_empty() {
                return Err(crate::error::Error::Apdu("script exhausted".into()));
            }
            let (expected, response) = self.script.remove(0);
            if expected != command {
                return Err(crate::error::Error::Apdu(format!(
                    "script expected {}, got {}",
                    crate::hexutil::encode_pretty(&expected),
                    crate::hexutil::encode_pretty(command)
                )));
            }
            return ApduResponse::from_bytes(&response);
        }
        for (expected, response) in &self.script {
            if expected.as_slice() == command {
                return ApduResponse::from_bytes(response);
            }
        }
        if self.strict {
            Err(crate::error::Error::Apdu(format!(
                "no script entry for {}",
                crate::hexutil::encode_pretty(command)
            )))
        } else {
            // Default: file not found / not supported, so apps can skip.
            ApduResponse::from_bytes(&[0x6A, 0x82])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_response_is_chained() {
        let sel = Apdu::select_by_name(b"TEST");
        let mut card = ScriptedTransport::new(vec![0x3B, 0x00])
            .in_order()
            .on(sel.to_bytes().unwrap(), vec![0x61, 0x03])
            .on(
                Apdu::get_response(0x03).to_bytes().unwrap(),
                vec![0x01, 0x02, 0x03, 0x90, 0x00],
            );
        let resp = transceive_ok(&mut card, &sel).unwrap();
        assert_eq!(resp.data, vec![0x01, 0x02, 0x03]);
    }
}
