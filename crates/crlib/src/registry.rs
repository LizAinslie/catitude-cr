use crate::error::{Error, Result};
use crate::info::CardInfo;
use crate::transport::Transport;

/// A card application / parser that can be registered with a [`Registry`].
///
/// Implement this trait to add support for a new family of cards (transit
/// systems, PIV/CAC, student IDs, …). See the crate README for a walkthrough.
pub trait CardApp {
    /// Short stable name, used as [`CardInfo::app`].
    fn name(&self) -> &'static str;

    /// Well-known AIDs this app might select. Empty means "I'll figure it
    /// out myself" (for example by reading the ATR, or by selecting a PPSE).
    fn aids(&self) -> &'static [&'static [u8]];

    /// Try to read this app from the connected card.
    ///
    /// Return [`Error::NotApplicable`] when the card is not one you handle;
    /// the registry will then move on to the next registered app. Any other
    /// error is surfaced to the caller and does not stop later apps.
    fn try_read(&self, card: &mut dyn Transport) -> Result<CardInfo>;

    /// Extension point for downstream crates.
    ///
    /// Built-in apps already surface full card data: the EMV app keeps the
    /// `pan` field masked but writes an unredacted structural dump, and
    /// `cr-autofill` returns the full credential. Override this hook to add
    /// more fields after `try_read`; the default body is a no-op. A CVV
    /// cannot be read from the chip and must come from the user.
    fn enrich(&self, card: &mut dyn Transport, info: &mut CardInfo) -> Result<()> {
        let _ = (card, info);
        Ok(())
    }
}

/// One entry in a [`Registry::probe`] result.
#[derive(Debug)]
pub struct ProbeResult {
    pub app: &'static str,
    pub result: Result<CardInfo>,
}

impl ProbeResult {
    pub fn info(&self) -> Option<&CardInfo> {
        self.result.as_ref().ok()
    }
}

/// Ordered list of [`CardApp`] implementations tried against a card.
pub struct Registry {
    apps: Vec<Box<dyn CardApp>>,
}

impl Registry {
    pub fn new() -> Self {
        Self { apps: Vec::new() }
    }

    /// ATR identifier, EMV, DESFire, Calypso, and a generic ISO 7816
    /// structure dumper.
    pub fn standard() -> Self {
        let mut r = Self::new();
        r.register(crate::apps::AtrApp)
            .register(crate::apps::EmvApp)
            .register(crate::apps::DesfireApp)
            .register(crate::apps::CalypsoApp)
            .register(crate::apps::Iso7816App);
        r
    }

    pub fn register(&mut self, app: impl CardApp + 'static) -> &mut Self {
        self.apps.push(Box::new(app));
        self
    }

    pub fn apps(&self) -> impl Iterator<Item = &dyn CardApp> {
        self.apps.iter().map(|a| a.as_ref())
    }

    /// Run every registered app. `NotApplicable` results are dropped.
    /// Other errors are kept so the caller can show them.
    pub fn probe(&self, card: &mut dyn Transport) -> Vec<ProbeResult> {
        let mut out = Vec::new();
        for app in &self.apps {
            match app.try_read(card) {
                Err(Error::NotApplicable) => continue,
                other => {
                    let mut result = other;
                    if let Ok(ref mut info) = result {
                        // Always give the app a chance to run its enrich hook
                        // (built-ins leave it empty).
                        if let Err(err) = app.enrich(card, info) {
                            result = Err(err);
                        }
                    }
                    out.push(ProbeResult {
                        app: app.name(),
                        result,
                    });
                }
            }
        }
        out
    }

    /// Probe and return only the successful [`CardInfo`] values.
    pub fn read_all(&self, card: &mut dyn Transport) -> Vec<CardInfo> {
        self.probe(card)
            .into_iter()
            .filter_map(|p| p.result.ok())
            .collect()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
