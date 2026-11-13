# catitude-cr

An extensible PC/SC smart-card reader framework for local card inspection,
autofill, and offline wallet use.

The library talks to a card through a `Transport`, sends APDUs, parses
BER-TLV, and runs a registry of `CardApp` implementations. Built-in apps
identify the ATR, dump DESFire / Calypso / ISO 7816 *structure*, and read
EMV credentials. Downstream crates add their own apps (transit systems,
PIV/CAC, whatever you actually need) by implementing the same trait.

The library crate is `crlib`. The headless CLI is the `cr` binary. The
desktop inspector is `crgui` in `crates/cr-gui` ([Desktop
inspector](#desktop-inspector)). `cr-autofill` reads the full payment
credential and types it into the focused field ([Autofill and wallet
flow](#autofill-and-wallet-flow)).

## Card data and wallet use

catitude-cr is built for an offline wallet flow: read a card locally, keep
the credential on the device, and replay it later (for example to autofill a
mobile or online checkout). The EMV structural dump is therefore **not
redacted**: it includes the full PAN (tag `5A`), Track 1 / Track 2
equivalent data (`56`, `57`, `9F6B`, …), and the cardholder name (`5F20`).

The EMV app surfaces these high-level fields:

| field | source |
|---|---|
| `scheme` | RID / AID prefix (Visa, Mastercard, Amex, …) |
| `pan` | masked PAN from tag `5A` (last 4 digits shown) |
| `expiry` | tag `5F24`, formatted `MM/YY` |
| `application_label` | tag `50` or `9F12` |

The `pan` field stays masked for the inspector views; the unredacted dump
and `cr-autofill`'s `Credential` carry the full value. The generic ISO 7816
dumper still redacts the payment tags it recognizes, because it is a
structure viewer rather than the wallet read path.

A CVV / CVC is not present on the chip. A save-and-replay flow must ask the
user for it separately and store it with the saved credential. There is no
network path in these crates: saving, encryption, and replay are local.

Card data is sensitive. Treat a saved credential like a password, keep it
out of logs, and put it behind an OS keyring or an encrypted store rather
than a world-readable file.

DESFire and Calypso list applications, file IDs, types and sizes; they do
not read file contents.

If you need more fields, implement your own `CardApp` in a downstream crate
— `CardApp::enrich` is the post-read hook, and the APDU / TLV helpers are
reusable.

## Layout

```
catitude-cr/
  crates/crlib/          library (`crlib`)
    src/apdu.rs          command / response APDUs, 61xx & 6Cxx handling
    src/tlv.rs           BER-TLV
    src/atr.rs           ATR parse + compact historical bytes
    src/transport.rs     Transport trait, transceive, ScriptedTransport
    src/pcsc_transport.rs  pcsc-crate backend, including poll_cards
    src/registry.rs      CardApp trait + Registry
    src/info.rs          CardInfo, view_rows() for a UI list
    src/apps/            built-in parsers
  crates/cr/             `cr` binary
  crates/cr-gui/         `crgui` binary (gpui-ce)
  crates/cr-autofill/    full-credential reader + keyboard injection (`cr-autofill`)
  nix/package.nix        Nix package for the whole workspace, GUI included
  shell.nix              rustc, cargo, pkg-config, pcsclite, GUI runtime libs
```

The Nix package builds and wraps `cr`, `crlib`, and `crgui`.

## Requirements

- Rust 1.85 or later for `crlib` and `cr` (edition 2021)
- Rust 1.95 or later for `crgui` (`gpui-ce` 0.3)
- `pcscd` running (on NixOS: `services.pcscd.enable = true;`)
- `libpcsclite` and `pkg-config` visible at build time
- for `cr-autofill` on Linux, permission to open `/dev/uinput` (see below)

On NixOS, from this directory:

```
nix develop
cargo build
cargo test
./target/debug/cr --list
./target/debug/cr --wait
./target/debug/crgui
```

On other distros, install `pcsc-lite` / `libpcsclite-dev` / `pkg-config`
and have `pcscd` running before you talk to a reader.

## CLI

```
cr --list              # list readers
cr --wait              # wait for a card, then probe it
cr "Broadcom" --wait   # substring match on the reader name
cr --timeout 60 --wait
```

Output is a list of fields grouped by app, followed by the unredacted
structural dump. This is the same data `crlib::view_rows` produces, so a
GUI can render it without a second parser.

## Autofill and wallet flow

`cr-autofill` reads the full EMV credential (PAN, expiry, cardholder) and
types a chosen field into whatever window currently has focus:

```
cr-autofill fill  [--field=pan|expiry|month|year|name] [--delay=MS] [--reader=NAME]
cr-autofill watch [--field=...] [--delay=MS] [--reader=NAME]
cr-autofill diag
```

`fill` reads once, waits `--delay` so you can focus the target field, then
types. `watch` re-arms when the card is removed. `diag` prints the raw PC/SC
reader state and ATR, which is the fastest way to tell an empty slot from a
PRESENT+MUTE card. `cr`, `cr-autofill fill`, and `cr-autofill watch` also
report a PRESENT+MUTE card explicitly instead of calling it absent.

On Linux, typing uses `/dev/uinput`. If it is root-only, allow your user to
open it:

```
# temporary, until reboot
sudo sh -c 'chgrp input /dev/uinput && chmod 0660 /dev/uinput'
```

or install a udev rule:

```
echo 'KERNEL=="uinput", GROUP="input", MODE="0660"' | sudo tee /etc/udev/rules.d/99-uinput.rules
sudo udevadm control --reload-rules && sudo udevadm trigger --name-match=uinput
```

The save-and-replay wallet layer is not implemented yet. The intended flow
is: read and save the credential locally, ask the user for the CVV (it is not
on the chip), then replay the saved values into a checkout form.

## Adding a `CardApp`

```rust
use crlib::{Apdu, CardApp, CardInfo, Error, Result, Transport, transceive_ok};

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
        // parse what you need with crlib::tlv, then:
        Ok(info)
    }
}

// somewhere in main:
let mut registry = crlib::Registry::standard();
registry.register(MyTransit);
let infos = registry.read_all(&mut card);
```

Return `Error::NotApplicable` when the card is not yours; the registry
moves on. Anything else is reported to the caller and later apps still run.

`CardApp::enrich` is a post-read hook. Built-ins leave it empty. Override it
in your own type if you want a second pass over the card after `try_read`.

The standard registry, in order, is: ATR identifier, EMV, DESFire, Calypso,
and ISO 7816 structure.

## Desktop inspector

`crgui` is a read-only window on the same registry the CLI uses. It lists
PC/SC readers, waits for a card, and renders the fields the registry
returns. A full `credential` section shows the PAN, expiry, and cardholder
read through `cr-autofill`. It does not write to the card.

PC/SC runs on a worker thread (`PcscContext::poll_cards`). The window only
receives reader and card events. Copy all puts the same plain text the CLI
prints on the clipboard.

```
cargo build -p cr-gui
./target/debug/crgui
```

`gpui-ce` is pinned from crates.io (`gpui_ce_kit` 0.3.0). The flake dev
shell includes the extra libraries a local `cargo build -p cr-gui` needs, and
`nix/package.nix` wraps `crgui` with the same runtime libraries. macOS uses
Metal and the system PC/SC framework.

## Library sketch

```rust
use crlib::{PcscContext, Registry, Transport};

fn inspect() -> crlib::Result<()> {
    let ctx = PcscContext::establish()?;
    for name in ctx.readers()? {
        println!("reader: {name}");
    }
    let mut card = ctx.connect(None)?;
    println!("ATR {}", crlib::hexutil::encode_pretty(card.atr()));
    for info in Registry::standard().read_all(&mut card) {
        print!("{info}");
    }
    Ok(())
}
```

Tests use `ScriptedTransport` and do not need a reader. The EMV tests assert
that the full PAN appears in the unredacted dump while the `pan` field stays
masked.
