use crate::atr::Atr;
use crate::error::Result;
use crate::hexutil;
use crate::info::CardInfo;
use crate::registry::CardApp;
use crate::transport::Transport;

/// Always-on app: parse the ATR and attach a best-effort identification.
pub struct AtrApp;

impl CardApp for AtrApp {
    fn name(&self) -> &'static str {
        "atr"
    }

    fn aids(&self) -> &'static [&'static [u8]] {
        &[]
    }

    fn try_read(&self, card: &mut dyn Transport) -> Result<CardInfo> {
        let raw = card.atr();
        if raw.is_empty() {
            return Err(crate::error::Error::NotApplicable);
        }
        let parsed = Atr::parse(raw)?;
        let mut info = CardInfo::new(self.name());
        info.push("reader", card.reader_name());
        info.push("atr", hexutil::encode_pretty(&parsed.raw));
        info.push(
            "convention",
            match parsed.convention {
                crate::atr::Convention::Direct => "direct (TS=3B)",
                crate::atr::Convention::Inverse => "inverse (TS=3F)",
            },
        );
        if let Some(p) = parsed.default_protocol {
            info.push("protocol", format!("T={p}"));
        }
        if !parsed.historical.is_empty() {
            info.push("historical", hexutil::encode_pretty(&parsed.historical));
            if let Some(ascii) = hexutil::ascii_if_printable(&parsed.historical) {
                info.push("historical.ascii", ascii);
            }
        }
        info.push("identification", parsed.identification);
        info.push_raw("atr.raw", hexutil::encode(&parsed.raw));
        Ok(info)
    }
}
