# SRP BugTrap (Rust, Win32)

A local crash reporter for the native 32-bit Survarium client. The DLL is written
in Rust, with small x86 assembly helpers for the compiler ABI. It implements all
82 exported BugTrap names with their original ordinals and calling conventions.
The installed 0.26g Skhid client has been tested under Wine Staging 11.8 / DXVK.

On a handled crash it writes
`SRP-crash-YYYYMMDD-HHMMSS-mmm-pPID-tTID-N.dmp` **beside the game EXE**, before
callbacks or the dialog. Closing the dialog keeps the dump. Existing files are
never overwritten. If the EXE folder is unwritable, it tries the Windows temporary
folder; the dialog shows the filename and folder in separate labeled fields,
or a failure message. Long folders use middle ellipsis. It uses static text
without selection or scrollbars. The layout follows the original C++
`IDD_SIMPLE_DLG`: an explicit 8-point Tahoma font with smoothing, white message panel, thin divider,
and gray footer with Submit Bug and Close buttons. The panel is taller to fit
the dump details; the original Mail and More controls are omitted. The original
green beetle icon (`vostok/sources/BugTrap/res/Bug.ico`) is embedded in the DLL
and used in both the message panel and the “Vostok Engine” window title.
Both button labels are centered horizontally and vertically.

Submit Bug opens [SRP Discord](https://discord.gg/Pxa4prXfwu) in the default
browser, only when clicked. The dialog stays open so the player can attach the
saved dump manually. A browser-launch failure displays the invite address.
Nothing is uploaded automatically. Email/upload entry points fail with
`ERROR_NOT_SUPPORTED`, and legacy support destinations are discarded. The DLL
imports `kernel32.dll`, `user32.dll`, `gdi32.dll` and `shell32.dll` (for the browser
link); it loads `dbghelp.dll` by absolute path from beside the EXE or the Windows
system directory when capturing a dump.

## Install

Use the 0.26g player overlay, or build the DLL for manual installation. Keep a
backup of the game's original `bugtrap.dll` and replace it with the built DLL
under that filename. Keep the matching `BugTrap.pdb` locally for debugging.
An EXE rebuild is unnecessary. This build
supports the native ANSI Win32 interface; x64 and BugTrapN/managed UI are outside
its scope. Native Windows testing is still outstanding.

## Capture behavior

- Uses the original faulting thread ID, exception record and register context.
- Runs the writer on a fresh 1 MiB stack, including for stack overflow. The
  overflow path skips callbacks and UI afterward.
- Captures despite `MiniDumpNoDump`, disabled report flags or silent activity.
- Keeps the game's `BT_SetUserMessage` in the dump's comment stream.
- Serializes its own captures without acquiring a configuration or logging lock.
  Concurrent crashing threads wait until the primary dump is closed, then apply
  their exit mode without waiting for callbacks or the dialog. Recursive captures
  on the owner/writer thread and overlapping snapshot requests return busy. A crash
  during a manual snapshot waits for it, then captures a separate crash report
  with the faulting thread context and normal callbacks/dialog.
- Synthetic SEH/C++/native .NET entry points propagate an exception to an outer
  native handler in `BTEM_CONTINUESEARCH` mode, and return in `BTEM_EXECUTEHANDLER`
  mode. Propagation happens after the Rust capture frames have returned.
- When Wine DbgHelp emits an empty module stream, collects modules from a snapshot
  of the current process and fills the standard module records, paths, addresses,
  timestamps and CodeView/PDB identities. Existing nonempty streams are preserved.
  A supplemental-write failure keeps the primary dump and reports it as incomplete.

This is an in-process handler. Forced termination, fail-fast paths that bypass
exception filters, loader deadlocks, severe corruption, unavailable DbgHelp, or
unwritable/full storage can prevent capture. It cannot guarantee a dump for those
cases. See Microsoft's [MiniDumpWriteDump guidance](https://learn.microsoft.com/en-us/windows/win32/api/minidumpapiset/nf-minidumpapiset-minidumpwritedump)
and [module record format](https://learn.microsoft.com/en-us/windows/win32/api/minidumpapiset/ns-minidumpapiset-minidump_module).

## API scope

| Legacy interface | Behavior |
|---|---|
| SEH/C++ filters, install/uninstall, exit mode | Implemented; repeated installation restores our filter after CRT startup |
| `BT_NetFilter` / `BT_CallNetFilter` | Native context capture; no managed stack processing |
| App name/version, user message, intro messages, flags, dump type, report format/path | Setters/getters retained; local capture policy and SRP dialog remain fixed |
| Pre/post handlers | Invoked on the report worker after the dump closes; skipped on stack overflow |
| `BT_SaveSnapshot[Ex]` | Writes an automatically named local raw dump with the same folder policy; the legacy filename argument is ignored |
| Email, uploads, registry export, version-resource reading, tracing/log APIs, import-table interception | Unsupported; operations with failure results return failure; applicable calls set `ERROR_NOT_SUPPORTED` |
| Attached log/archive list | Empty; the game's own logs remain in their existing location |

There is no XML/ZIP report generation, screenshot capture, restart-after-crash,
advanced BugTrap UI or mail integration. The dump is the report. Settings strings
are immutable process-lifetime copies (up to 32,767 bytes each), so getter pointers
stay valid after another setter; configuration calls allocate, capture does not
allocate through Rust. Win32/DbgHelp may allocate internally. Callers must supply
valid native pointers and coordinate configuration changes.

## Build

Use the same nightly Rust environment as the game workspaces;
`srp-026g/flake.nix` supplies it with `rust-src`. The build also needs a host C
linker for Cargo build scripts and `rust-lld` from that toolchain. No Windows
SDK or proprietary compiler is required for the DLL when cross-building on Linux.
The two locked dependencies are Microsoft's `windows-sys` and `windows-link`.

```sh
nix develop ./srp-026g
cd client/bugtrap
./build.sh
```

This uses `-Z build-std=core` and the `i686-pc-windows-msvc` target. Output:
`target/i686-pc-windows-msvc/release/BugTrap.dll` and `BugTrap.pdb`.
It has no Rust standard-library runtime or C runtime DLL dependency. Unsafe code
is confined to the native ABI, Win32 calls, fixed crash state and compiler helpers;
Rust cannot make an already-corrupted game's memory safe.

## Verify

The C++ programs in `tests/` are independent ABI/crash callers, not part of the
shipping DLL. From an x86 Visual Studio developer prompt with `lld-link` on PATH,
run `tests\build.cmd` to build them. VS2008 was used for validation.

On Linux with Wine and an X server (Xvfb works), use an isolated test prefix:

```sh
python3 tests/check_abi.py target/i686-pc-windows-msvc/release/BugTrap.dll
WINEPREFIX=/path/to/test-prefix python3 tests/check_dumps.py target/i686-pc-windows-msvc/release
rustc --edition 2024 --test src/arithmetic.rs -o target/arithmetic-test
./target/arithmetic-test
```

The nineteen crash cases verify exception codes and thread IDs, module presence, the
game error comment, callback ordering, stack overflow, dialog closure, repeated
crashes, Unicode paths, a different working directory, read-only EXE fallback,
and explicit failure when neither folder is writable. They also cover all three
synthetic entry points in continue-search/execute-handler modes, recursive worker
callbacks, and a second crash while a test-only DbgHelp shim holds the primary
writer behind a gate. The concurrency case verifies that the dump survives and
the second crash does not wait for the primary callback. A snapshot-overlap case
checks that a real crash saves its own exception stream and runs its callback
after the snapshot finishes. Run as a normal user so
permission tests are effective. The Discord case intercepts only the browser
API call and checks the exact invite, absence of automatic launches, and that
the dialog and dump survive the click; it does not contact Discord. The ABI check also rejects runtime/network import
changes and checks all 82 legacy ordinals.

For an **idle, disposable installed 0.26g client** with `survarium_skhid.exe`:

```sh
python3 tests/game_test.py /path/to/game/binaries/x86 \
  --wine /path/to/wine \
  --prefix /path/to/game-prefix \
  --output "$PWD/target/game-test-026g"
```

The harness temporarily replaces BugTrap and adds a D3D11 forwarding test DLL.
After the game window appears and 15 seconds elapse, it triggers a real null write
inside the client, checks the dialog, closes it and parses the persistent dump.
It validates the original exception/register context, fault module, game module
and Rust PDB identity. It restores the original BugTrap and removes both test
DLLs in `finally`, checks the EXE and earlier dump hashes, and keeps evidence in
`--output`. Its server address is loopback. Do not use this against a running
play session. The test does not change host ptrace policy.

The existing player-overlay builder includes the built DLL. From the repository
root, create the player ZIP with:

```sh
python3 srp-026g/client-patches/make_client_026g.py /path/to/game \
  target/Survarium-0.26g-srp-files.zip --pack-zip --server=026g.skhid.link:1234
```

It contains the patched EXE, `bugtrap.dll`, launcher, login certificate and player
README. Symbols, sources, tests and crash dumps stay outside the player ZIP.
