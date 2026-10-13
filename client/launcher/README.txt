Skhid native launcher experiment (Linux and Windows)

A native Rust desktop launcher for the existing Survarium 0.26g game. Updates,
settings and the launcher window run on the host OS. Linux runs the Windows game
through Wine; Windows starts it directly.

This is an unfinished prototype, not a release-ready launcher. The native UI
must match the existing patched 0.26g Windows launcher: window layout and size,
artwork, typography, controls, update progress, embedded server page, and
localized launcher text and links. The current simplified egui window does
not meet that requirement. Compare both launchers with the same language,
server page and update state before treating the port as complete.

Build and run

Install Rust 1.89 or newer and a C compiler. On Linux, install the Wayland/X11,
xkbcommon and OpenGL runtime libraries. For the game, install Bash, Wine with
32-bit application support, Winetricks and Vulkan drivers. The Wine setup also
uses the standard Linux tools required by play_skhid.sh (including flock and
timeout). The launcher does not install system packages or ask for root.

  cargo build --locked --release --manifest-path client/launcher/Cargo.toml
  client/launcher/target/release/skhid-launcher --game-dir /path/to/Survarium/game

On NixOS, use `nix-shell client/launcher/shell.nix` to expose the graphics
libraries as well as the build tools.

On Windows the output is skhid-launcher.exe. The same source builds with the
normal Windows Rust toolchain. The GUI uses egui; no Windows launcher, embedded
Internet Explorer or Wine installation is needed just to open the native UI.

You can put the executable in game/binaries/x86; it then finds its game folder
without arguments. Otherwise select the folder in the window or pass --game-dir.
The original game/resources must already be extracted. Updates contain the
Skhid overlay only, not the full game download.

The window checks for updates on opening a valid installation. Check / repair
repeats that check. PLAY starts the installed game, including when the update
server is unavailable. On Linux, Set up Wine / DXVK prepares dependencies without
starting the game; PLAY also performs setup when needed. Operations run off the
UI thread. Keep the window open while an operation or game is running.

The Wine setup is embedded from srp-026g/client-patches/play_skhid.sh at build
time, so an older server overlay cannot remove its native-launcher support.
It uses the same wineprefix, DXVK markers, memory-check patch and log as the
standalone script. The language comes from the native menu; a literal server=
setting in the installed play_skhid.sh is retained. With no override the game's
embedded login address is used. The game itself remains the Windows executable.

Update server and files

The default update server is updater_skhid.url in binaries/x86, or
https://skhid.link when that file is absent. --site URL overrides it. The Update
server field changes the update source for this run, not the game's login host.
Downloads use native HTTPS with certificate validation, without redirects. HTTP
is permitted only for localhost tests. Manifest parsing, ZIP validation, CRC and
SHA-256 use the same library as the Windows updater in ../updater/logic.

All archive entries are validated before installation. Symlink destinations
and reserved launcher files are rejected. Staged files and backups are checked
before replacement. An ordinary installation failure restores changed files;
a journal recovers interrupted replacement on the next update check. The native
updater shares the Linux script's installation lock. Do not run another updater
or modify the installation externally while an update is in progress.

Edited .sh/.bat/.txt/.url files survive repairs of the same archive. A newer
archive replaces them. Obsolete files are removed only if they still match the
previously installed version. No accounts or passwords are stored by the native
launcher. Settings and update state are .skhid-native.json and
.skhid-native.state in binaries/x86. Game/setup output is in skhid-native.log
and skhid-wine.log there. Logs can include the local game path.

The project website and shop open in the system browser. The native window does
not embed the original launcher's server page. macOS is not supported.

Command-line operations (also work without a display)

  skhid-launcher --game-dir PATH --check
  skhid-launcher --game-dir PATH --setup       Linux only
  skhid-launcher --game-dir PATH --play
  skhid-launcher --game-dir PATH --site http://127.0.0.1:8088 --check

Release bundle

Build on a Linux distribution compatible with the target machines, then pass
--native-launcher=/path/to/skhid-launcher to make_client_026g.py. The optional
x86-64 ELF binary is installed executable at game/binaries/x86/skhid-launcher.
Skhid.sh opens it when present; otherwise the existing script behavior remains.
The Windows launcher/updater remain in the same overlay. Native Windows builds
can be run directly; this option packages the Linux executable only.

Tests

  cargo test --locked --manifest-path client/launcher/Cargo.toml
  cargo test --locked --manifest-path client/updater/logic/Cargo.toml
  cargo build --locked --manifest-path client/launcher/Cargo.toml
  python -m unittest discover -s client/launcher/tests -v
  python -m unittest discover -s srp-026g/client-patches -p 'test_*launcher.py'
  python -m unittest discover -s srp-026g/client-patches -p 'test_release_overlay.py'

The integration tests start only a localhost server and use synthetic files.
Set SKHID_NATIVE_EXE to test a release binary. They do not require Wine or the
game. The unit tests also exercise rollback, interrupted-update recovery and
locking. These checks are run locally; this experiment has no CI workflow.
