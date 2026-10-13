Skhid native launcher (Linux and Windows)

The launcher and updater run on the host OS. Linux runs the existing Survarium
0.26g Windows game through Wine internally; Windows starts it directly.

The window follows the patched 0.26g launcher: 899 x 549 logical pixels, its
original header/footer and button artwork, server page, progress bar and five
languages. Images and translated labels are read from survarium_launcher.exe
(or launcher_skhid.exe) in your game folder. No game artwork is compiled into
this program. Calibri is used when installed; Carlito supplies matching font
metrics on Linux. Font rasterization can differ between operating systems.

PLAY performs any needed Wine/DXVK setup, then hides the launcher while the
game runs. An update failure allows playing the installed game. The language
menu changes launcher labels, page and game language together and remembers
the selection. Website and shop open in your browser. F5 checks/repairs files;
F2 opens game-folder, update-server and Wine setup options. Double-clicking
the status text also opens those options. Keep the launcher open during updates
and setup. Closing the hidden launcher is handled when the game exits.

NixOS / Home Manager

The root flake supplies the game package, native launcher and Wine/DXVK runtime.
It follows the HOMM module pattern: import the module and enable the program.

  inputs.skhid.url = "github:srp-survarium/skhid-srp/feat/native-linux-client";

  # In your nixosSystem modules list:
  inputs.skhid.nixosModules.default

  # In a configuration module:
  programs.skhid = {
    enable = true;
    locale = "en";                 # en, ru, ua, de, pl; initial choice
    # game = pkgs.fetchurl { url = "..."; hash = "sha256-..."; };
    # site = "https://skhid.link";
  };

For Home Manager, import inputs.skhid.homeManagerModules.default instead.
After merge, the input may follow main. There is no need to edit the module
when changing revisions. Flake consumers can set inputs.nixpkgs.follows to
use their own compatible nixpkgs; the package otherwise uses its pinned input.

With game unset, Nix fetches the pinned original 0.26g archive from archive.org
(about 2.8 GiB). game accepts your own original archive or unpacked game folder,
including a fetchurl derivation, just like the HOMM modules. The game data stays
in the store; on first launch it is copied (using reflinks when supported) to
$XDG_DATA_HOME/skhid/0.26g/game, normally ~/.local/share/skhid/0.26g/game.
Allow about 6 GiB for each copy on filesystems without reflinks. Updates, Wine
prefix, logs and settings stay in that user copy. Subsequent starts and Nix
rebuilds preserve it. Changing the game input does not overwrite existing saves
or settings; move the user copy aside explicitly if you want a fresh import.
The first update downloads the current Skhid overlay. The full game is never
added to this repository. Normal graphics/Vulkan drivers must be enabled;
the NixOS module enables hardware.graphics, and Home Manager uses the host's
existing graphics configuration.

Run directly without changing your configuration:

  nix run github:srp-survarium/skhid-srp/feat/native-linux-client

Build the launcher alone or enter its development shell:

  nix build .#launcher
  nix develop

Build without Nix

Install Rust 1.89 or newer, a C compiler and pkg-config. Linux requires GTK 3,
WebKitGTK 4.1 and libsoup 3 development packages (Debian/Ubuntu:
libwebkit2gtk-4.1-dev), plus their runtime libraries. Windows requires the
Microsoft Edge WebView2 Runtime. The launcher UI itself does not use Wine.
For the Linux game, install Bash, Wine with 32-bit application support,
Winetricks, Vulkan drivers and the usual GNU tools used by play_skhid.sh.
Install Carlito for the original launcher font metrics when Calibri is absent.

  cargo build --locked --release --manifest-path client/launcher/Cargo.toml
  client/launcher/target/release/skhid-launcher --game-dir /path/to/Survarium/game

On Windows the executable is skhid-launcher.exe. You can put it in
binaries/x86 to find the game without arguments. Otherwise pass --game-dir
or set the folder in the first-run dialog. Original game files must already
be extracted when running the standalone binary; the Nix wrapper handles that
import automatically. The updater downloads the Skhid overlay only.

The Wine setup is embedded from srp-026g/client-patches/play_skhid.sh at build
time. It shares the script's prefix, memory-check patch and installation lock.
The Nix wrapper supplies a packaged DXVK installer; other installations use
Winetricks as before. A literal server= setting in the installed script is
retained. The game itself remains the Windows executable.

Updates and settings

The update server comes from updater_skhid.url in binaries/x86, defaulting to
https://skhid.link. --site URL overrides it. F2's server field changes the
update source for this run; it does not change the game's login server.
Downloads use native HTTPS with certificate validation and no redirects.
HTTP is allowed only for localhost testing. Manifest/ZIP/CRC/SHA-256 handling
is shared with the Windows updater in ../updater/logic.

All archive entries are checked before installation. Staged files and backups
are checked before replacement. Installation failures roll back; an interrupted
transaction recovers on the next update check. Symlink destinations and reserved
launcher files are rejected. Do not run another updater or modify game files
during an update. Edited .sh/.bat/.txt/.url files survive repair of the same
archive; a new archive replaces them. Obsolete files are removed only if they
still match their previously installed versions.

The server page is displayed in a script-disabled iframe with an opaque origin;
it cannot invoke launcher commands. Loading/offline pages come from the
installed launcher_skhid folder. Launcher actions are accepted only from the
local UI document. Web links open in the system browser. macOS is not supported.

Settings and update state are .skhid-native.json and .skhid-native.state in
binaries/x86. Game/setup output is in skhid-native.log and skhid-wine.log there.
No accounts or passwords are stored by the launcher.

Command-line operations (also work without a display)

  skhid-launcher --game-dir PATH --check
  skhid-launcher --game-dir PATH --setup       Linux only
  skhid-launcher --game-dir PATH --play
  skhid-launcher --game-dir PATH --site http://127.0.0.1:8088 --check

Release bundle

Build for the target Linux distribution, then pass --native-launcher=PATH to
make_client_026g.py. It installs the optional x86-64 ELF in game/binaries/x86;
Skhid.sh opens it when present. The Windows launcher/updater remain in the
overlay. Native Windows builds can be run directly. --native-launcher packages
the Linux executable only and does not bundle its system WebKitGTK libraries.

Local verification (CI is disabled)

  cargo test --locked --manifest-path client/launcher/Cargo.toml
  cargo test --locked --manifest-path client/updater/logic/Cargo.toml
  cargo build --locked --manifest-path client/launcher/Cargo.toml
  python -m unittest discover -s client/launcher/tests -v
  python -m unittest discover -s nix -p 'test_*.py'
  python -m unittest discover -s srp-026g/client-patches -p 'test_*launcher.py'
  python -m unittest discover -s srp-026g/client-patches -p 'test_release_overlay.py'

Integration tests use a localhost server and synthetic files. SKHID_NATIVE_EXE
selects a built binary. Unit tests cover rollback, interrupted-update recovery,
locking and local UI origin checks. Nix preparation tests cover concurrent
starts, recovery of interrupted copies and preservation of user files.
