# catitude-cr

An extensible PC/SC smart-card reader framework, plus the parsers that are
safe to ship.

The library talks to a card through a `Transport`, sends APDUs, parses
BER-TLV, and runs a registry of `CardApp` implementations. Built-in apps
identify the ATR, dump DESFire / Calypso / ISO 7816 *structure*, and read a
deliberately limited set of EMV fields. Downstream crates add their own
apps (transit systems, PIV/CAC, whatever you actually need) by implementing
the same trait.

This workspace is a starting point for a GPUI inspector. The crate that
definitely builds here is `catitude-cr-cli`; see [Dropping this into GPUI](#dropping-this-into-gpui).

## Safe-fields policy

Built-in parsers never return, store, or dump:

- a full payment-card PAN (EMV tag `5A`)
- Track 1 / Track 2 equivalent data (tags `57`, `56`, `9F6B`, …)
- a cardholder name from track data or tag `5F20`
- a CVV / CVC (that value is not on the chip)

The EMV app returns only:

| field | source |
|---|---|
| `scheme` | RID / AID prefix (Visa, Mastercard, Amex, …) |
| `pan` | last 4 digits of tag `5A`, the rest replaced with `*` |
| `expiry` | tag `5F24`, formatted `MM/YY` |
| `application_label` | tag `50` or `9F12` |

Sensitive tags are written as `[redacted]` in the structural dump. The
ISO 7816 dumper applies the same redaction if it happens to see those tags.
DESFire and Calypso list applications, file IDs, types and sizes; they do
not read file contents.

catitude-cr will not grow a full-PAN or track-data parser. If you have a lawful
need to surface additional fields, implement your own `CardApp` in a
downstream crate — `CardApp::enrich` is the hook, and the APDU / TLV helpers
are reusable. Do not patch `apps/emv.rs` to weaken the masking.

## Layout

```
catitude-cr/
  crates/catitude-cr/        library
    src/apdu.rs          command / response APDUs, 61xx & 6Cxx handling
    src/tlv.rs           BER-TLV
    src/atr.rs           ATR parse + compact historical bytes
    src/transport.rs     Transport trait, transceive, ScriptedTransport
    src/pcsc_transport.rs  pcsc-crate backend
    src/registry.rs      CardApp trait + Registry
    src/info.rs          CardInfo, view_rows() for a UI list
    src/apps/            built-in parsers
  crates/catitude-cr-cli/    `catitude-cr` binary
  shell.nix              rustc, cargo, pkg-config, pcsclite
```

## Requirements

- Rust 1.85 or later (edition 2021)
- `pcscd` running (on NixOS: `services.pcscd.enable = true;`)
- `libpcsclite` and `pkg-config` visible at build time

On NixOS, from this directory:

```
nix-shell
cargo build
cargo test
./target/debug/catitude-cr --list
./target/debug/catitude-cr --wait
```

On other distros, install `pcsc-lite` / `libpcsclite-dev` / `pkg-config`
and have `pcscd` running before you talk to a reader.

## CLI

```
catitude-cr --list              # list readers
catitude-cr --wait              # wait for a card, then probe it
catitude-cr "Broadcom" --wait   # substring match on the reader name
catitude-cr --timeout 60 --wait
```

Output is a list of fields grouped by app, followed by a redacted
structural dump. This is the same data `catitude-cr::view_rows` produces, so a
GUI can render it without a second parser.

## Adding a `CardApp`

```rust
use catitude-cr::{Apdu, CardApp, CardInfo, Error, Result, Transport, transceive_ok};

pub struct MyTransit;

impl CardApp for MyTransit {
    fn name(&self) -> &'static str { "my-transit" }

    fn aids(&self) -> &'static [&'static [u8]] {
        &[b"1TIC.ICA"]
    }

    fn try_read(&self, card: &mut dyn Transport) -> Result<CardInfo> {
        let resp = transceive_ok(card, &Apdu::select_by_name(b"1TIC.ICA"))
            .map_err(|_| Error::NotApplicable)?;
        let mut info = CardInfo::new(self.name());
        info.push("fci_len", resp.data.len().to_string());
        // parse what you need with catitude-cr::tlv, then:
        Ok(info)
    }
}

// somewhere in main:
let mut registry = catitude-cr::Registry::standard();
registry.register(MyTransit);
let infos = registry.read_all(&mut card);
```

Return `Error::NotApplicable` when the card is not yours; the registry
moves on. Anything else is reported to the caller and later apps still run.

`CardApp::enrich` is a post-read hook. Built-ins leave it empty. Override it
in your own type if you want a second pass over the card after `try_read`.

The standard registry, in order, is: ATR identifier, EMV (safe fields),
DESFire, Calypso, ISO 7816 structure.

## Dropping this into GPUI

GPUI (and a Zed-style app) was the intended front-end, but pulling `gpui`
into this workspace would drag in a large GPU / windowing stack that does
not build headlessly here. The CLI is the reference front-end. A view is
one loop over `catitude-cr::view_rows`:

```rust
use catitude-cr::{view_rows, CardInfo, RowKind};

fn card_rows(infos: &[CardInfo]) -> Vec<(String, String, RowKind)> {
    view_rows(infos)
        .into_iter()
        .map(|r| (r.label, r.value, r.kind))
        .collect()
}

// in a gpui element:
// for (label, value, kind) in card_rows(&infos) {
//     match kind {
//         RowKind::Heading => heading(label),
//         RowKind::Field   => row(label, value),
//         RowKind::Dump    => muted_row(label, value),
//     }
// }
```

The probe itself is the same as the CLI: `PcscContext::establish`,
`wait_for_card` / `connect`, `Registry::standard().read_all(&mut card)`.
Put that on a background thread and publish `Vec<CardInfo>` into your
GPUI model.

## Library sketch

```rust
use catitude-cr::{PcscContext, Registry, Transport};

fn inspect() -> catitude-cr::Result<()> {
    let ctx = PcscContext::establish()?;
    for name in ctx.readers()? {
        println!("reader: {name}");
    }
    let mut card = ctx.connect(None)?;
    println!("ATR {}", catitude-cr::hexutil::encode_pretty(card.atr()));
    for info in Registry::standard().read_all(&mut card) {
        print!("{info}");
    }
    Ok(())
}
```

Tests use `ScriptedTransport` and do not need a reader. The EMV tests
assert that a full PAN never appears in the rendered `CardInfo`.
