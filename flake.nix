{
  description = "catitude-cr: extensible PC/SC smart-card reader (crlib) + cr CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self, nixpkgs, rust-overlay }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f system);
      pkgsFor = system: import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
    in
    {
      # Overlay so catitude-flake (or anything else) can pull `cr` into pkgs.
      overlays.default = final: prev: {
        catitude-cr = final.callPackage ./nix/package.nix { };
      };

      packages = forAll (system:
        let pkgs = pkgsFor system; in {
          catitude-cr = pkgs.callPackage ./nix/package.nix { };
          default = pkgs.callPackage ./nix/package.nix { };
        });

      devShells = forAll (system:
        let pkgs = pkgsFor system; in {
          default = pkgs.mkShell {
            # pcscd must be running on the host; pcsclite here is the build+link lib.
            # The extra libraries are for a local `cargo build -p cr-gui` only.
            # `nix build` does not compile the GUI.
            nativeBuildInputs = [ pkgs.pkg-config pkgs.rust-bin.stable.latest.default ];
            buildInputs = with pkgs; [
              pcsclite
              vulkan-loader
              libxkbcommon
              wayland
              xorg.libX11
              xorg.libXcursor
              xorg.libXi
              xorg.libXrandr
              fontconfig
              openssl
            ];
            shellHook = ''
              echo "catitude-cr dev shell — cargo build, then ./target/debug/cr --wait"
              echo "GUI (not packaged): cargo build -p cr-gui && ./target/debug/crgui"
            '';
          };
        });

      formatter = forAll (system: (pkgsFor system).nixfmt-rfc-style);
    };
}
