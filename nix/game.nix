{ pkgs, launcher }:
{ game ? null, locale ? "en", site ? "https://skhid.link" }:
let
  inherit (pkgs) lib;
  archive = if game != null then game else pkgs.fetchurl {
    name = "survarium_full_026g0.7z";
    url = "https://archive.org/download/vostok_engine_v0.26g0_build_2777_internal_id_1104_jan_14_2015/survarium_full_026g0.7z";
    sha256 = "7435558eeb17098fc9337c4ba9470402cae4c9be974ef5d9bc544f19b122a028";
  };
  data = pkgs.runCommand "survarium-026g-data" { nativeBuildInputs = [ pkgs._7zz pkgs.python3 ]; } ''
    mkdir unpacked
    if [ -d ${lib.escapeShellArg (toString archive)} ]; then
      cp -r ${lib.escapeShellArg (toString archive)}/. unpacked/
    else
      7zz x -y ${lib.escapeShellArg (toString archive)} -ounpacked > /dev/null
    fi
    python3 ${./import-game.py} unpacked "$out"
  '';
  language = { en = "english"; ru = "russian"; ua = "ukrainian"; de = "german"; pl = "polish"; }.${locale};
  # Wine's new WoW64 mode resolves `wine winepath system32` as 64-bit.
  # Select both directories explicitly instead of relying on that redirector.
  dxvkSetup = pkgs.writeShellApplication {
    name = "skhid-install-dxvk";
    runtimeInputs = [ pkgs.coreutils pkgs.gnugrep ];
    text = ''
      [[ "''${1:-}" == install && -f "$WINEPREFIX/system.reg" ]]
      case "$(grep '^#arch=' "$WINEPREFIX/system.reg")" in
        '#arch=win64')
          for dll in d3d11 dxgi; do
            cp --remove-destination ${pkgs.dxvk.bin}/x64/"$dll.dll" "$WINEPREFIX/drive_c/windows/system32/$dll.dll"
          done
          target32="$WINEPREFIX/drive_c/windows/syswow64"
          ;;
        '#arch=win32') target32="$WINEPREFIX/drive_c/windows/system32" ;;
        *) echo 'Unknown Wine prefix architecture' >&2; exit 1 ;;
      esac
      for dll in d3d11 dxgi; do
        cp --remove-destination ${pkgs.dxvk.bin}/x32/"$dll.dll" "$target32/$dll.dll"
      done
    '';
  };
  wrapper = pkgs.writeShellApplication {
    name = "skhid";
    runtimeInputs = [ pkgs.python3 pkgs.coreutils pkgs.gnugrep pkgs.util-linux pkgs.bash pkgs.wineWow64Packages.staging pkgs.winetricks pkgs.xrandr pkgs.xdg-utils ];
    text = ''
      export SKHID_DXVK_SETUP=${dxvkSetup}/bin/skhid-install-dxvk
      export SKHID_INITIAL_LANGUAGE=${lib.escapeShellArg language}
      # All modifications, Wine prefix and updates belong to the current user.
      game_dir="$(python3 ${./prepare-game.py} ${data})"
      exec ${launcher}/bin/skhid-launcher --game-dir "$game_dir" --site ${lib.escapeShellArg site} "$@"
    '';
  };
  desktop = pkgs.makeDesktopItem {
    name = "skhid";
    desktopName = "Survarium — Skhid";
    comment = "Survarium 0.26g";
    exec = "skhid";
    terminal = false;
    categories = [ "Game" "ActionGame" ];
  };
in pkgs.symlinkJoin {
  name = "skhid-026g";
  paths = [ wrapper desktop ];
  passthru = { inherit data archive launcher dxvkSetup; };
  meta = { description = "Survarium 0.26g with native launcher and Wine"; license = lib.licenses.unfree; platforms = [ "x86_64-linux" ]; mainProgram = "skhid"; };
}
