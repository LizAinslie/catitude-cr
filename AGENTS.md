# AGENTS.md

Cargo workspace: `crates/crlib` (library, package/import name `crlib`), `crates/cr` (binary `cr`), `crates/cr-gui` (binary `crgui`, gpui-ce).

## Docs vs reality

The README's `catitude-cr` import paths and `./target/debug/catitude-cr` commands are stale. Use `crlib::` in code and `target/debug/cr` for the CLI. `catitude-cr` is only the Nix package/derivation name. The README also claims the Nix package excludes the GUI; `nix/package.nix` now builds and wraps the whole workspace, GUI included.

## Commands

- `nix-shell` (Linux flake: `nix develop`) provides rustc, cargo, pkg-config, pcsclite. `pcscd` must be running for any real reader; tests do not need it.
- `cargo test -p crlib` — primary suite, no reader or extra services needed.
- `cargo test -p crlib <name>` — single test (all tests are inline `#[cfg(test)]` modules).
- `cargo test -p cr-gui format` — the GUI's only test.
- `cargo run -p cr -- --list`, `cargo run -p cr -- --wait`; `--timeout SECS` before/with `--wait`. Reader argument is a substring match.
- `cargo build -p cr-gui` needs Rust 1.95+ and Linux GUI libs (included in the flake/shell); core crates need Rust 1.85+.
- `nix build` builds the workspace but gates only `cargo test -p crlib`; there is no CI, rustfmt config, clippy config, or xtask.

## Architecture

- `crates/crlib/src/lib.rs` is the public API surface. CLI entry is `crates/cr/src/main.rs`; GUI splits PC/SC work (`worker.rs`) from UI (`view.rs`, `model.rs`, `theme.rs`).
- Everything talks through `Transport` (`transport.rs`). Use `transceive`/`transceive_ok` instead of `transmit` so SW `61xx` (GET RESPONSE), `6Cxx` (Le retry), and DESFire `91AF` chaining are handled once.
- New card support = implement `CardApp` (`registry.rs`) in a downstream crate and `Registry::register`. Return `Error::NotApplicable` to skip; any other error is reported but later apps still run. `enrich` is a post-read hook; built-ins are no-ops.
- `Registry::standard()` order is ATR, EMV, DESFire, Calypso, ISO 7816.
- Parsers produce `CardInfo`; render through `info::view_rows` so CLI and GUI stay identical.

## Testing

Tests use `ScriptedTransport` (`transport.rs`): `.on(command, response)` matches exact APDU bytes, `.in_order()` enforces sequence. Keep `crlib` free of dev-dependencies; add fixtures to the inline test modules.
