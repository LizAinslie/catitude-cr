# The `cr` CLI, the `crlib` library, and the `crgui` desktop inspector
# (gpui-ce), built with rustPlatform. Exposed as pkgs.catitude-cr via the
# flake overlay. Binaries: `cr` (headless) and `crgui` (GUI).
{
  lib,
  rustPlatform,
  pkg-config,
  makeWrapper,
  pcsclite,
  # crgui / gpui-ce build + runtime deps
  fontconfig,
  freetype,
  wayland,
  libxkbcommon,
  vulkan-loader,
  libGL,
  xorg,
}:

let
  # gpui-ce dlopen()s these at runtime (Vulkan loader, Wayland, X11, GL).
  runtimeLibs = [
    vulkan-loader
    wayland
    libxkbcommon
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
rustPlatform.buildRustPackage {
  pname = "catitude-cr";
  version = "0.1.0";

  src = lib.cleanSource ./..;
  cargoLock.lockFile = ./../Cargo.lock;

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];
  buildInputs = [
    pcsclite
    fontconfig
    freetype
    wayland
    libxkbcommon
    vulkan-loader
    libGL
  ] ++ runtimeLibs;

  # Build the whole workspace: cr, crlib, crgui.
  cargoBuildFlags = [ "--workspace" ];
  cargoCheckFlags = [ "--workspace" ];
  # Only crlib has unit tests; crgui/cr have none worth gating the build on.
  cargoTestFlags = [ "-p" "crlib" ];
  doCheck = true;

  # crgui links gpui-ce which loads Vulkan/Wayland/X11 at runtime; point it at
  # them so it starts outside a dev shell.
  postInstall = ''
    if [ -e "$out/bin/crgui" ]; then
      wrapProgram "$out/bin/crgui" \
        --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeLibs}"
    fi
  '';

  meta = {
    description = "Extensible PC/SC smart-card reader: crlib framework, cr CLI, crgui inspector";
    mainProgram = "cr";
    license = with lib.licenses; [ mit asl20 ];
    platforms = lib.platforms.linux;
  };
}
