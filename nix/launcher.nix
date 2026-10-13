{ lib, rustPlatform, pkg-config, wrapGAppsHook3, gtk3, webkitgtk_4_1, libsoup_3, makeWrapper, makeFontsConf, carlito, liberation_ttf, dejavu_fonts }:
rustPlatform.buildRustPackage {
  pname = "skhid-launcher";
  version = "0.1.0";
  src = lib.cleanSourceWith {
    src = ../.;
    filter = path: type:
      let p = lib.removePrefix (toString ../. + "/") (toString path);
      in (type == "directory" && builtins.elem p [ "client" "client/launcher" "client/updater" "srp-026g" "srp-026g/client-patches" ])
        || lib.hasPrefix "client/launcher/src" p
        || lib.hasPrefix "client/launcher/ui" p
        || builtins.elem p [ "client/launcher/Cargo.toml" "client/launcher/Cargo.lock" "client/updater/logic/Cargo.toml" "client/updater/logic" "srp-026g/client-patches/play_skhid.sh" ]
        || lib.hasPrefix "client/updater/logic/src" p;
  };
  cargoRoot = "client/launcher";
  buildAndTestSubdir = "client/launcher";
  cargoLock.lockFile = ../client/launcher/Cargo.lock;
  nativeBuildInputs = [ pkg-config wrapGAppsHook3 makeWrapper ];
  buildInputs = [ gtk3 webkitgtk_4_1 libsoup_3 ];
  # Calibri is the original launcher's font. Carlito preserves its metrics
  # on machines without Microsoft's fonts; no game artwork is shipped.
  preFixup = ''
    gappsWrapperArgs+=(--set FONTCONFIG_FILE ${makeFontsConf { fontDirectories = [ carlito liberation_ttf dejavu_fonts ]; }})
  '';
  meta = { description = "Native Skhid launcher"; license = lib.licenses.gpl3Plus; platforms = [ "x86_64-linux" ]; mainProgram = "skhid-launcher"; };
}
