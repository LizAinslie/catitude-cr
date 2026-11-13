{ pkgs ? import <nixpkgs> {} }:
let
  # gpui-ce dlopen()s these at runtime; keep them on LD_LIBRARY_PATH.
  guiRuntimeLibs = with pkgs; [
    vulkan-loader
    libxkbcommon
    wayland
    libGL
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
    xorg.libxcb
    fontconfig
    freetype
  ];
in
pkgs.mkShell {
  packages = with pkgs; [
    rustc
    cargo
    rustfmt
    clippy
    pkg-config
    pcsclite
    openssl
    # For `cargo build -p cr-gui` on Linux. The Nix package does not build it.
  ] ++ guiRuntimeLibs;
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (guiRuntimeLibs ++ [ pkgs.pcsclite ]);
}
