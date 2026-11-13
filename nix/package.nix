# The `cr` CLI (and the crlib library it bundles), built with rustPlatform.
# Exposed as pkgs.catitude-cr via the flake overlay; the binary is `cr`.
{ lib, rustPlatform, pkg-config, pcsclite }:

rustPlatform.buildRustPackage {
  pname = "catitude-cr";
  version = "0.1.0";

  src = lib.cleanSource ./..;
  cargoLock.lockFile = ./../Cargo.lock;

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ pcsclite ];

  # `crgui` links gpui-ce. This package stays the headless `cr` binary.
  # Build the GUI from a dev shell: `cargo build -p cr-gui`.
  cargoBuildFlags = [
    "-p"
    "cr"
    "-p"
    "crlib"
  ];
  cargoCheckFlags = [
    "-p"
    "cr"
    "-p"
    "crlib"
  ];
  cargoTestFlags = [ "-p" "crlib" ];

  # crlib's tests use a scripted in-memory Transport, no reader needed.
  doCheck = true;

  meta = {
    description = "Extensible PC/SC smart-card reader: crlib framework + cr CLI";
    mainProgram = "cr";
    license = with lib.licenses; [ mit asl20 ];
  };
}
