# survarium_updater.exe 0.26g — what the original updater does and says on the pipe

> **Version scope:** client 0.26g (build 2777), `survarium_updater.exe`: what
> the game's own updater did and said on the launcher's pipe. Static reverse
> engineering of 2026-10-13; nothing was run. It is not used by the Skhid
> launcher, which starts its own updater (`../../client-patches/launcher/README.md`,
> `client/updater`). The launcher's side: `launcher.md` beside this page.

Static analysis only (nothing was run). Target:
`downloads\v026g\extracted\Survarium\game\binaries\x86\survarium_updater.exe`
(PE32, link time 2015-01-14, image base `0x00010000`, `.text` VA `0x11000` / raw `0x400`,
`.rdata` VA `0xFC000` / raw `0xEAE00`, `.data` VA `0x117000` / raw `0x105800`).
All addresses below are VAs in that image, as printed by
`tools\re-kit\launcher\v026g\survarium_updater.disasm.txt`.

Confidence marks used everywhere:

- **PROVEN** — read in the 0.26g disassembly at the address given.
- **LIKELY** — inferred (from 0.21d names, strings, Windows API behaviour, or the launcher).
- **UNKNOWN** — could not be determined statically from the updater.

Helper scripts (stdlib only) are in `tools\re-kit\launcher\scripts\`:
`upd_lib.py` (annotated listing, strings + xrefs, folded logging), `upd_names026.py`
(recovered function names), `upd_msgids.py` (every packet send site + id byte),
`upd_states.py` (every `set_state` call + value), `upd_fields.py` (accesses to an updater member).
Set `UPD_CACHE` to a scratch folder before running them (they pickle the parsed listing).

Method names come from the `__FUNCTION__` strings that the 0.26g log calls still push
(`.\updater.cpp`, line number, function signature). A function was located by the VA of
its `__FUNCTION__` string. The 0.21d PDB listing was used only to understand; 0.21d has a
much smaller protocol (ids 1–5 only), so nothing here is copied from it.

---

## 0. Things a replacement author needs first (summary)

- Pipe name is **argv[2]**, not a constant. The launcher builds it as
  `\\.\pipe\vostok_updater_pipe-<name>` (launcher `0x14872`/`0x15723`, format `L"%s-%s"`).
- Frame = `u16 LE payload_length` + payload; payload byte 0 is the message id. Variable size,
  **not** padded to 4096. No hello / handshake in either direction.
- Updater opens the pipe **once** with `CreateFileW`, no `WaitNamedPipe`, no retry, no
  `SetNamedPipeHandleState` — the launcher's pipe instance must already exist.
- The only thing sent on a normal run is `02 <state>` (state changed), plus `01 …` progress
  and `06..0D` install info during real work. "Game is up to date" = state `0x0E`.
- The updater does not exit by itself after reaching a final state. It exits when the
  launcher closes the pipe or sends `0x1B` (abort); it then reports state `0x0F`, flushes,
  closes its handle, and returns exit code 0.

---

## 1. Command line

### 1.1 Parsing — `updater::initialize_options` `0x20041`..`0x207D7` (called from the constructor, `0x1AD40`)

**PROVEN**

- `GetCommandLineW` → `CommandLineToArgvW` (`0x200C9`, `0x200D4`). `argc < 2` → return with
  "options valid" flag `this+0x20168D` left 0 (`0x200E7`).
- `argv[1]` is compared with `_wcsicmp` (`0xDF1CE`, case-insensitive) and sets
  `m_action` = `this+0x201630`:

  | argv[1] | action | VA | dispatched by `updater::start` (`0x1B4CD`) to |
  |---|---|---|---|
  | `full_update` | 1 | `0x201DA` | `start_update` `0x1BA88` |
  | `update` | 2 | `0x201FF` | `start_update` `0x1BA88` |
  | `install` | 3 | `0x20221` | `start_install(false)` `0x1C294` |
  | anything else | 4 | `0x202E1` | `uninstall` `0x1CE63` |
  | `copy` | 5 | `0x20243` | `copy` `0x1D317` |
  | `seed` | 6 | `0x20261` | `run_seeding` `0x1B548` |
  | `create_shortcut` | 7 | `0x20280` | handled in `execute` before any pipe work (`0x1AF03`) |

- There is **no argc check per action** beyond `argc >= 2`: the code reads the fixed
  indexes below unconditionally. With too few arguments it reads the terminating NULL of
  the argv array (and beyond) and passes it to the copy routine — behaviour undefined
  (LIKELY crash inside `wcscpy_s`).
- After the fixed arguments, every remaining `argv[i]` from a per-action start index up to
  `argc-1` is compared with `disable_pipe` (`0x20793`); a match sets `this+0x20168E`
  (pipe disabled: no pipe is opened, errors are shown with `MessageBox` instead).

Argument tables (member offsets are `this+…`; wide strings are copied with a 260-char limit
via `0x25C47` unless noted):

**`full_update` / `update`** (`0x2050C`..`0x206C9`; `disable_pipe` scan starts at index 13)

| argv | stored at | meaning | evidence for the meaning |
|---|---|---|---|
| 2 | `0x201698` | pipe name; the text from character 29 on is copied to the global application name `0x11A630` (default `L"survarium"`, max 0x40) | `0x2050C`, `0x20522`; pipe name passed to `CreateFileW` from `0x1B292`→`0x1A357` |
| 3 | `0x2018A0` | current game version string (sent as `version=`) | `0x20A3F` |
| 4 | `0x201640` | download type: `torrent` → 0, anything else → 1 (direct/HTTP) | `0x20547`..`0x2055A` |
| 5 | `0x201EB8` | install root folder; `<argv5>\downloads` is derived into `0x201AA8` | `0x20560`, `0x206A7`..`0x206C1` |
| 6 | `0x2020C0` | folder (must exist) | meaning UNKNOWN (passed to the package installer, `0x1CAFF`) |
| 7 | `0x2022C8` | folder (must exist) | meaning UNKNOWN (installer, `0x1CB30`) |
| 8 | `0x2026D8` | user data / log folder (must exist) | used as log directory in `execute` `0x1B014` |
| 9 | `0x2024D0` | a file (must exist, must not be a directory) | meaning UNKNOWN (installer, `0x1CB92`) |
| 10 | `0x2028E0` | user name (max 0x101) | used with the log folder in `execute` `0x1B030` |
| 11 | `0x202AE2` | string | meaning UNKNOWN (installer, `0x1CBF8`) |
| 12 | `0x202CEA` (narrow) | update server `host[/path]` or `host:port`; text after `:` goes to `0x202DEE` (service/port, max 0x10), default service `"http"` | `0x205D1`..`0x206A1` |
| 13 | `0x201644` (u16) | torrent listen port, decimal | `0x205E9`..`0x2064A` |

**`install` / uninstall** (`0x206E0`..`0x2078C`; scan starts at index 10)

| argv | stored at | meaning |
|---|---|---|
| 2 | `0x201698` | pipe name (same treatment) |
| 3 | `0x201CB0` | package file (`.sup`) — must be a file |
| 4 | `0x201EB8` | install root folder |
| 5 | `0x2020C0` | folder |
| 6 | `0x2022C8` | folder |
| 7 | `0x2026D8` | user data / log folder |
| 8 | `0x2024D0` | file |
| 9 | `0x2028E0` | user name |
| 10 | `0x202AE2` | string |

**`copy`** (`0x20315`..`0x2035D`; scan starts at index 2)

| argv | stored at | meaning |
|---|---|---|
| 2 | `0x201698` | pipe name |
| 3 | `0x2026D8` | user data / log folder |
| 4 | `0x2028E0` | user name |

**`seed`** (`0x2036B`..`0x204F1`; scan starts at index 2)

| argv | stored at | meaning |
|---|---|---|
| 2 | `0x201698` | pipe name |
| 3 | `0x201AA8` | root folder, `\downloads` appended |
| 4 | `0x2026D8` | user data / log folder |
| 5 | `0x2028E0` | user name |
| 6 | `0x201644` | listen port (decimal u16) |
| 7 | `0x201678` | `true` → seeding debug enabled |
| 8 | `0x201648` | upload speed limit (decimal u32) |

**`create_shortcut`** (`0x20293`..`0x202DC`): argv 2..5 → `0x203DFE`, `0x204006`, `0x20420E`,
`0x204416`. `execute` appends `L".lnk"` to the first and calls `create_shortcut` (`0x24A50`,
COM `IShellLinkW` + `IPersistFile::Save`), then returns 0. No pipe, no validation.

### 1.2 Validation — `validate_options` `0x207DA`

**PROVEN** (uses `GetFileAttributesW`; `0x29088` = "is a directory", `0x29094` = "is not a directory")

- update / full_update: `0x201EB8`, `0x2020C0`, `0x2022C8`, `0x2026D8` must be existing
  directories, `0x2024D0` an existing file, `0x201AA8` (`<root>\downloads`) an existing directory.
- install / uninstall: `0x201CB0` existing file, `0x2020C0`, `0x2022C8`, `0x2026D8`
  directories, `0x2024D0` file.
- seed: `0x201AA8` and `0x2026D8` directories.
- copy (and 7): always valid.
- Success sets `this+0x20168D = 1` (`0x207C9`).

### 1.3 Wrong / missing arguments — `updater::start` `0x1B3B6`

**PROVEN**: if the valid flag is 0 (`0x1B44C`): unless `disable_pipe`, send error message
(id 5) with the text `Invalid arguments` (`0x1B51F`), then `set_state(3)` (`0x1B53C`).
The process then keeps waiting on the pipe; when it ends, the exit code is 1 (see 3.4).
Note this happens **after** the pipe was opened using argv[2]; if argv[2] is not an
openable pipe the process exits with code 2 before `start` (1.4).

### 1.4 `updater::execute` `0x1AEE4` (called from `application::execute` `0x1116F`)

**PROVEN**, in order:

1. action 7 → create shortcut, return 0.
2. Log directory: if `0x2026D8` is empty, a default user-data directory (`0x292AF`, via
   `SHGetKnownFolderPath`) and `GetUserNameA` are used; otherwise argv values. Logging is
   initialised (`0x2977E`) unless action is 4.
3. Unless `disable_pipe`: set disconnect handler `on_pipe_disconnected` (`0x1D90A`, stored
   at pipe+0x40) and message handler `on_pipe_message` (`0x1E558`, pipe+0x00), then
   `pipe.connect(argv[2])` (`0x1A333`). Failure → log `pipe error`, **return 2** (`0x1B311`).
4. `start()` (`0x1B31C`), then `io_service.run` (`0x1B32D`). Run error → return 4.
5. Return `error_code()` (`0x233F0`).

### 1.5 Service child?

**PROVEN**: the import table has no service API (`StartServiceCtrlDispatcher`,
`OpenSCManager`, … absent; see `survarium_updater.headers.txt`). The updater is an ordinary
process whoever starts it. **LIKELY**: `application::initialize` calls `0x2919C`
(token / SID check; imports `OpenProcessToken`, `GetTokenInformation`,
`AllocateAndInitializeSid`, `EqualSid`) to decide whether dialogs may be shown — the 0.21d
name of that helper is `application_has_interactive_user`. The launcher has the text
"Can't run survarium_updater from service nor directly", so the launcher may start it
through `survarium_service.exe`; that is the launcher's / service's business.

### 1.6 Single-instance check — `two_updaters_running` `0x22A67`

**PROVEN**

- No named object (no mutex/event; `CreateMutex*` is not imported). It walks the process
  list: `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)`, `Process32FirstW`/`Process32NextW`,
  `OpenProcess(PROCESS_ALL_ACCESS)` on each, compares `szExeFile` with
  `L"survarium_updater.exe"` (`0x22C25`), returns `count > 1` (`0x22C8B`).
- Only caller: the torrent-download start routine `0x22C99`, which loops
  `while (two_updaters_running()) Sleep(1000)` (`0x22CAA`..`0x22CBC`). It is not used at
  process start and never makes the updater exit.
- Two defects in the original: `dwSize` is set to `0x128` (`0x22B0F`) while the W function
  is called, and the counter is incremented when the name is **different** (`0x22C5B`:
  `test eax,eax / je skip / inc`).
- **LIKELY** consequence on real Windows: `Process32FirstW` rejects the too-small `dwSize`,
  so the function logs `Process32First`, sleeps 5 s (`0x22AF5`) and returns false every time.
  A replacement needs no such check.

---

## 2. Pipe client side

### 2.1 Opening — `0x1A333`

**PROVEN**

```
CreateFileW(name = argv[2],
            GENERIC_READ | GENERIC_WRITE (0xC0000000),
            share 0, security NULL,
            OPEN_EXISTING (3),
            FILE_FLAG_OVERLAPPED (0x40000000),
            NULL)                                   ; 0x1A344..0x1A35A
```

- `INVALID_HANDLE_VALUE` → error = `GetLastError()`, return; `execute` exits with code 2.
  **No retry, no `WaitNamedPipeW`** (not imported), **no `SetNamedPipeHandleState`** (not
  imported) — so the client end stays in the default byte-read mode.
- Success: pipe state (`pipe+0x10B0`, i.e. `this+0x13C8`) = 3 "connected", the handle is
  attached to the asio IOCP (`0x24C87`), "reading header" flag (`pipe+0x10B4`) = 1, first
  async read is started (`0x1A41C`).
- Pipe state values: 1 = not connected (initial, `0x1A2DC`), 3 = connected, 0 = failing.
  Every `send_*` tests `this+0x13C8 == 3`.

### 2.2 Wire framing

**PROVEN**

`fixed_packet<4096>` in memory (built on the stack by every sender, e.g. `0x23091`..`0x230CA`):

| offset | field |
|---|---|
| +0x00 | u32, 0 (heap flag; 0 = inline buffer) |
| +0x04 | pointer to data (the inline buffer at +0x14) |
| +0x08 | u32 capacity = 0x1000 |
| +0x0C | u32 size, **initialised to 2** |
| +0x10 | pointer to the packet (writer handle) |
| +0x14 | byte buffer[0x1000] |

- Append (`0x1A1A7`): copies `len` bytes at `data + size`, `size += len`. Exceeding 0x1000
  is a fatal assertion (`mutable buffer overflow`, `0x1A158`).
- Send (`0x1A3B1`): writes `u16 (size - 2)` at `data[0]` (`0x1A3CB`..`0x1A3CE`), then one
  synchronous write of exactly `size` bytes (`0x1A3E3`). So on the wire:

```
+0  u16 LE  payload_length      (= number of bytes that follow; 1..4094)
+2  u8      message id
+3  ...     fields
```

- One packet = one write of `2 + payload_length` bytes. Not fixed 4096, no other header,
  no checksum, no padding.
- Write failure: pipe state = 0 and a close is posted to the io_service (`0x1A3F7`..`0x1A411`),
  which closes the handle, sets state 1 and calls the disconnect handler (`0x1A4AC`).
- Receive (`0x1A41C`, completion `0x1A51C`): read exactly 2 bytes into `pipe+0x98` (u16
  length), then read exactly `length` bytes into the receive packet at `data+2`, call the
  message handler, repeat. Reads are asio "read exactly N" operations, so the launcher may
  write header and body in one or several writes; message-mode vs byte-mode on the server
  end does not matter to the updater as long as the bytes arrive in this order.
  **LIKELY** the launcher end reads the same way (colleague's notes).
- Any read error (including the launcher closing the pipe) → close → `on_pipe_disconnected`.

### 2.3 Field serialization

**PROVEN**

| kind | encoding | routine |
|---|---|---|
| integers | raw little-endian bytes of the C++ variable: u8, u16, u32, u64 | `0x1A1A7` |
| narrow string | `u8 length` + `length` bytes, no terminator; length is `strlen & 0xFF` (longer text is truncated/garbled, effective limit 255) | `0x1A1D5` |
| wide string | `u16 character_count` + UTF-16LE characters, no terminator | `0x1A20B` |
| bool (in id 0x18 only) | u8, 0x00 = false, 0xFF = true | `0x1A5CA` |

Incoming fields are read from `data+3` on with plain byte copies of the same widths
(`on_pipe_message`, `0x1E56E` onward).

---

## 3. Messages the updater sends

Complete list: `upd_msgids.py` finds 17 call sites of the send routine `0x1A3B1`; there are
no others. "needs pipe" = the sender tests pipe state == 3 itself; when not connected the
ones marked *(box)* show a `MessageBox` instead.

| id | name (from `__FUNCTION__`) | payload after the id byte | sender VA | when |
|---|---|---|---|---|
| 0x01 | `send_progress_message` (0.21d name) | 4 × u64: `done`, `total`, `speed`, `eta` (see 3.2) | `0x23163` | download / install progress |
| 0x02 | `send_state_message` (0.21d name) | u8 state (low byte of `m_state`) | `0x23084` | every state change (see 3.3) |
| 0x03 | `send_command_line_message` (0.21d name) | wide string | `0x235D8` | downloaded file is an `.exe`: text is `"<full path>"` in double quotes (`0x2357F`..`0x235C4`) |
| 0x04 | `send_not_enough_disk_space_message` | `info` (see 3.5) | `0x2366A` *(box)* | disk full / cannot set file size / cannot write |
| 0x05 | `send_error_message(const char[])` | narrow string | `0x2457E` *(box)* | free-text error (see 3.6) |
| 0x06–0x0D | `on_package_info` | narrow string | `0x1D72E` (id = 6 + kind, `0x1D8A8`) | installer step info (see 3.7) |
| 0x0E | (no name string) "question: answer me" | none | `0x23324` | after an error message; blocks until 0x1C / 0x1D arrives (see 3.8) |
| 0x0F | `send_cannot_open_file_message` | none | `0x238A6` *(box)* | install error 2 |
| 0x10 | `send_cannot_close_file_message` | none | `0x239F4` *(box)* | install error 3 |
| 0x11 | `send_cannot_move_file_message` | none | `0x23B42` *(box)* | install error 4 |
| 0x12 | `send_cannot_run_shell_command_message` | none | `0x23DA8` *(box)* | install error 5 |
| 0x13 | — | — | — | never sent by the updater |
| 0x14 | `send_port_is_busy_message` | `info` (stale contents) | `0x240C0` | torrent listen port could not be bound |
| 0x15 | `send_port_is_ok_message` | `info` (stale contents) | `0x241C6` | torrent listen port bound |
| 0x16 | `send_cannot_verify_hash_message` | none | `0x242CC` *(box)* | install error 6 |
| 0x17 | `send_package_hash_mismatch_message` | none | `0x2441A` *(box)* | install error 7 |
| 0x18 | `seeding_debug_tick` | u8 bool (always 0xFF), u32, u32 upload limit, u32 | `0x1B9FB` | seed mode with debug on, periodic |
| 0x19 | `send_ask_to_restart_message` | none | `0x23EF6` | install failed and was not cancelled (see 3.9) |

`send_cannot_set_file_size_message` (`0x23C90`) and `send_cannot_write_to_file_message`
(`0x23D1C`) have no id of their own: they fill `info` with error type 2 / 3 and send id 0x04.

### 3.1 There is no "ready to play" message

**PROVEN**: the only thing sent when the version server answers "latest" is the state
message `02 0E` (`0x20FA6`..`0x20FAB`). Nothing else in the sender list fits.

### 3.2 Progress (id 0x01) — fields at `this+0x201608`, `+0x201610`, `+0x201618`, `+0x201620`

**PROVEN**

- `done`, `total`: bytes for downloads (torrent: `0x1FF9C`..`0x1FFBD`; direct:
  `0x222BD`..`0x222E7`); for installation they are weighted work units over 6 installer
  stages (`0x1BC32`..`0x1BD04`), not bytes.
- `speed`: bytes per second. Torrent and direct paths store 0 and `send_progress_message`
  computes it from a 65-sample ring buffer when it is 0 (`0x23174`..`0x23281`).
- `eta`: written only by install progress: remaining time in seconds, or
  `0xFFFFFFFFFFFFFFFF` when unknown (`0x1BF9C`..`0x1BFC4`). During downloads it is whatever
  was there before (**LIKELY** 0 on a fresh process).
- Rate: torrent ≥ 500 ms between messages (`0x2000E`); direct download every 500 ms timer
  while state is 9 (`0x2234F`, `0x223A9`); install ≥ 250 ms (`0x1C06B`).
- `start_progress` (`0x2311D`) zeroes `done/total/speed` and, if connected, sends one
  progress message immediately. It runs at the start of a direct download (`0x21CA8`), of a
  torrent download (`0x23017`) and of an install (`0x1C3D1`).

### 3.3 State message (id 0x02) — `set_state` `0x2305E`

**PROVEN**: sent only if the new state differs from the current one, **and**
`this+0x201691` is set (constructor sets it; `run_seeding` clears it at `0x1B5C8`, so seed
mode reports no states), **and** the pipe is connected.

### 3.4 `updater_state` values

Names are not in the binary (the enum type name `survarium_updater::updater_state` is, in
`async_save_torrent_resume_data`'s signature). Values are **PROVEN** by the `set_state`
call sites (`upd_states.py`); the labels are mine.

| value | label (mine) | set at | reported |
|---|---|---|---|
| 0x01 | fatal error | many, e.g. `0x20F6E` (bad version reply), `0x1C5FC` (install aborted, code 0x16), `0x1D3B4` (copy failed), `0x220EB`, `0x1FC60` (user said no) | yes |
| 0x02 | initial | constructor `0x1AC09` | never (it is the start value) |
| 0x03 | invalid arguments | `0x1B53C` | yes |
| 0x04 | checking version (request sent) | `0x20B6F` | yes |
| 0x05 | version request failed, retry pending | `0x20BFF` | yes |
| 0x06 | downloading the `.torrent` file over HTTP | `0x2187D` | yes |
| 0x07 | `.torrent` download failed | `0x22548` | yes |
| 0x08 | torrent: checking files | `0x1F568` (libtorrent states 0, 1, 7) | yes |
| 0x09 | downloading | `0x21D37` (direct), `0x23027` (torrent added), `0x1F584` (libtorrent states 2, 3, 6) | yes |
| 0x0A | direct download error, retry in 5 s | `0x22181` | yes |
| 0x0B | saving torrent resume data | `0x1FE02` | yes |
| 0x0C | download complete | `0x23518` | yes |
| 0x0D | command line delegated (installer `.exe` handed to the launcher, after id 0x03) | `0x235CE` | yes |
| 0x0E | **latest version / up to date** | `0x20FAB` | yes |
| 0x0F | aborted / finished aborting | `0x1E4A7` (`end_aborting`); also used as target state | yes, then the pipe is closed |
| 0x10 | installing | `0x1C3DB` | yes |
| 0x11 | install finished OK | `0x1CA01` (via `0x1C9FA`) | yes |
| 0x12 | uninstall finished | `0x1D163` | yes |
| 0x13 | copy finished OK | `0x1D697` | yes |
| 0x14 | disk-space question pending | `0x1FC45`, `0x220CD` | yes |
| 0x15 | seeding (seed mode) | `0x1B5E5` | no (flag cleared first) |
| 0x16 | server maintenance | `0x21050` | yes |
| 0x17 | restarting after a failed install | `0x1C89F`, `0x240AB` | yes |

`m_target_state` (`this+0x20162C`) holds the state to enter after an asynchronous step
(values seen: 9, 0x0C, 0x0F).

Process exit code = `error_code()` `0x233F0`: state ≥ 0x0D → 0; state 3 → 1; any other
state (1, 2, 4..0x0C) → 3. Plus 2 (pipe connect failed) and 4 (io_service error) from
`execute`.

### 3.5 `info` structure (`not_enought_disk_space_info`, `this+0x204620`), serializer `0x1A598`

**PROVEN**

```
wide string   path      ; a single drive letter, e.g. "C", or a folder path (0x1F934..0x1F9C7)
u64           free      ; GetDiskFreeSpaceExW on <root>\downloads, 0 on failure (0x1F9CC..0x1F9FB)
u8            error_type; 0 = disk full, 2 = cannot set file size, 3 = cannot write (0x1FA01)
```

Filled by `fill_not_enough_disk_space_info(error_type)` `0x1F8BB`. Ids 0x14 and 0x15
serialize the same member without filling it first, so their payload content is whatever
is in it (all zero on a fresh process: empty string, 0, 0). **UNKNOWN** whether the
launcher reads those fields for 0x14/0x15.

### 3.6 Error text (id 0x05)

**PROVEN**: narrow string, bytes exactly as produced by `sprintf` in the process ANSI code
page; all format strings are ASCII English. Length byte = `strlen & 0xFF`. Texts (format
string VA → call site):

| text | where |
|---|---|
| `Invalid arguments` | `0x1B51F` |
| `malformed version request` | `0x20F4E` |
| `Invalid url %s` | `0x210A2` |
| `invalid file extension on version response (%s)` | `0x21151` |
| `Can't find and truncate .torrent extension from url (%s)` | `0x21183` |
| `Can't create http download directory %S` | `0x21A14` |
| `direct download error: %d (%s), http error code %hd` | `0x221EE` |
| `Can't save http fast resume data` | `0x2229E` |
| `Can't download torrent file: %d (%s), http error code %hd` | `0x22520` |
| `Invalid torrent file %s (error_code %d (%s))` | `0x2272F` |
| `Can't create download directory %S` | `0x22E07` |
| `Can't add torrent: %d (%s)` | `0x22F7B` |
| `Fastresume file is too big (limit is %Iu): %I64u` | `0x248E5` |
| `Error while loading torrent fast resume data: %d (%s)` | `0x249A6` |
| `Invalid target state after saving torrent resume data %d` | `0x1FD43` |
| (four more call sites in torrent validation and the resume-data alert handlers; their texts were not read) | `0x212D9`, `0x2139D`, `0x1F7BE`, `0x1F8A9` |

A failed **version request** sends no text at all — only states 5 / 4.

### 3.7 Install info (ids 0x06..0x0D)

**PROVEN**: `package_info(kind, text)` is called on the installer thread, posted to the
io_service, and `on_package_info` sends `id = 6 + kind`, narrow string = text; skipped if
`disable_pipe` (`0x1D7C2`). Kind names from the jump table at `0x1C0C4`:

| id | kind | name string |
|---|---|---|
| 0x06 | 0 | `cmd` |
| 0x07 | 1 | `extract` |
| 0x08 | 2 | `assert` |
| 0x09 | 3 | `patch` |
| 0x0A | 4 | `start_transaction` |
| 0x0B | 5 | `move_transacted` |
| 0x0C | 6 | `commit_transaction` |
| 0x0D | 7 | `install_info` |

`on_package_error` (`0x1D6A5`) only logs; it sends nothing.

### 3.8 The question message (id 0x0E) — `0x23324`

**PROVEN**: sends `0E`, sets `this+0x201694 = 0`, then runs the io_service one handler at
a time (`0x27B1A`) until `this+0x201694` becomes non-zero; returns true if it is 1.
`on_pipe_message` sets it to 1 on id 0x1C and to 2 on id 0x1D. It is always sent right
after a specific error message, which tells the launcher what to ask:

| preceding message | caller | on answer 0x1C (yes) | on answer 0x1D (no) |
|---|---|---|---|
| 0x04 (disk full, torrent) | `0x1FC4B` | `start_torrent_loading` | state 1 |
| 0x04 (disk full, direct) | `0x220D3` | `start_direct_download` | state 1 |
| 0x0F/0x10/0x11/0x12/0x16 or 0x04 (install error) | `0x1FB70` | `on_install_catch_error` returns true to the installer; for signature checks the step is retried (`0xB7609`..`0xB7616`), other steps LIKELY the same | returns false; installer fails (error 0x15 for signature checks) |
| 0x19 (install failed) | `0x1C777` | delete package file, state 0x17, `start()` again | `end_aborting` → state 0x0F, pipe closed |

Id 0x17 (package hash mismatch) is **not** followed by 0x0E (`0x1FAE8`..`0x1FAEF`).
If the pipe is lost while waiting, nothing sets the answer (**LIKELY** the original then
spins in that loop).

### 3.9 `send_ask_to_restart_message` (id 0x19) is about the download, not the launcher

**PROVEN**: sent only from `on_install_end` (`0x1C6F4`) when the install result is neither
0 (success) nor 0x16 (aborted) and the "terminate installation" flag (`this+0x201600`) is
clear. Payload: none. Expected answer: the 0x0E question that follows (3.8). When the pipe
is not connected the function itself deletes the package file, sets state 0x17 and calls
`start()` (`0x23FFA`..`0x240B2`). **LIKELY** it corresponds to the launcher resource text
"Package corrupted! Restarting download in %u sec.".

---

## 4. Messages the updater receives — `on_pipe_message` `0x1E558`

**PROVEN** (id read at `0x1E57E`, switch at `0x1E5F2`..`0x1E619`)

| id | payload | effect | VA |
|---|---|---|---|
| 0x1A | u8 download type (0 = torrent, 1 = direct) | `switch_download_type` `0x1EDB6`: no-op if unchanged; otherwise stores it and, depending on state, restarts the download (`restart_downloading`), or in states 0x0E / 0x10 starts or stops seeding | `0x1E6B4` |
| 0x1B | none | `abort()` `0x1D996` | `0x1E6A9` |
| 0x1C | none | answer "yes" to 0x0E (`this+0x201694 = 1`) | `0x1E699` |
| 0x1D | none | answer "no" to 0x0E (`this+0x201694 = 2`) | `0x1E689` |
| 0x1E | u64 limit, clamped to 0x7FFFFFFF | `change_upload_speed_limit` `0x1B665` | `0x1E65D` |
| 0x21 | u16 port | `change_port_number` `0x1EF92`: if different, store, stop torrent downloading if a session is active, `restart_downloading` | `0x1E648` |
| any other id | u8 flag | flag ≠ 0 → start the seeding debug timer (`0x1B732`); 0 → cancel it | `0x1E61B` |

- The "any other id" branch is the compiler's merged default, so the real id of the
  seeding-debug toggle is **UNKNOWN** from the updater (0x1F, 0x20 and everything below
  0x1A all land there). A replacement must simply ignore unknown ids.
- Cross-check against the 0.26g launcher (its packet builders at `0x1734D`, `0x173E7`,
  `0x174D8` and the callers of its one-byte sender `0x1749F`): the launcher sends only
  0x1A, 0x1B, 0x1C, 0x1D and 0x21. 0x1E and the debug toggle are **LIKELY** sent by
  whatever drives `seed` mode.

### 4.1 `on_pipe_disconnected` `0x1D90A`

**PROVEN**: logs `on pipe disconnected` and calls `abort()`.

`abort()` `0x1D996`: `abort_or_restart_downloading(false)` (`0x1DB82`), cancels the five
timers, and if the seeding flag (`this+0x201690`) is still set afterwards, logs
`is seeding` and terminates the process (`0xBA4C0`).

`abort_or_restart_downloading(false)` acts by state (read as an outline only, the
per-state branches at `0x1DCE0`..`0x1E2A9` were not all traced): cancels an HTTP request in
progress, asks libtorrent to save resume data (state 0x0B, target 0x0F), or — LIKELY —
sets the installer's terminate flag (`InterlockedExchange` at `0x1E291`); in idle / final
states it goes straight to `end_aborting` (`0x1E3AD`, PROVEN):
stop the libtorrent session, `set_state(0x0F)`, and if the pipe is connected
`FlushFileBuffers(pipe)`, close the handle, pipe state = 1 (`0x1E51A`..`0x1E547`). With no
pending work left, `io_service.run` returns and the process exits with code 0.

---

## 5. Normal sequences

Notation: `U→L 02 04` = updater sends payload bytes `02 04` (on the wire `02 00 02 04`).
Timing constants are PROVEN at the VA given; the order is the order of the calls in the code.

### (a) Start, version check, already latest

1. Launcher has created the pipe and started the updater. Updater: `CreateFileW` on
   argv[2] (once). Nothing is sent on connect.
2. `start()` → `start_update` → `send_version_request` → `U→L 02 04`.
3. HTTP reply line `<you use the latest version!>` → `U→L 02 0E` (`0x20FAB`).
4. A 60 000 ms timer is armed (`0x20FC4`) whose handler calls `send_version_request`
   again (`0x20CD6`): so every 60 s `U→L 02 04`, then `U→L 02 0E` again.
5. If download type is torrent and it is not seeding yet, `start_seeding()` runs
   (`0x20FF8`..`0x21013`): libtorrent session, seeds every `downloads\torrent\*.sup.torrent`.
6. **The process stays alive.** It ends when the launcher sends `1B` or closes the pipe:
   `U→L 02 0F` (only if the pipe is still open), flush, close, exit code 0. The launcher
   then sees the pipe break.

`<server maintenance!>` is the same with state 0x16 instead of 0x0E (also re-polled every 60 s).

### (b) Start, update found, download, install, done

Direct (HTTP) download of a `.sup` package:

1. `U→L 02 04`
2. reply = URL ending `.sup` → `start_direct_download`: `U→L 01 <four zero u64>` (start_progress),
   `U→L 02 09`
3. `U→L 01 done total speed eta` every 500 ms
4. finished → `U→L 02 0C`
5. `start_install(true)`: `U→L 01 <zeros>`, `U→L 02 10`
6. during install: `U→L 06..0D <text>` per step, `U→L 01 …` at most every 250 ms
7. success → `U→L 02 11`. Process stays alive until `1B` / pipe close → `U→L 02 0F`, exit 0.

Torrent download: after step 1 → `U→L 02 06` (fetching the `.torrent` by HTTP) → session
start → `U→L 15 <info>` (port ok) **or** `U→L 14 <info>` (port busy; the session is stopped
and the updater waits for `L→U 21 <u16 port>`) → `U→L 01 <zeros>`, `U→L 02 09`; libtorrent
state changes give `02 08` (checking) / `02 09`; progress `01 …`; at completion
`U→L 02 0B` may appear (saving resume data) → `U→L 02 0C` → install as above (the torrent
keeps seeding during install).

Reply URL ends `.exe` (web installer): after `02 0C` → `U→L 03 <wide "\"path\"">` →
`U→L 02 0D`; the updater does not run the file itself.

File already downloaded (`downloads\http\<name>` + matching `.fastresume`, or a finished
torrent): `02 04` is followed directly by `02 0C` (`0x211B8`..`0x211E7`).

### (c) Version check fails (server down)

1. `U→L 02 04`
2. HTTP error → `on_version_request_error`: `U→L 02 05` (`0x20BFF`), log only, no text
   message; timer 3 000 ms (`0x20C97`).
3. timer → `send_version_request` → `U→L 02 04`, and so on **forever** (no retry limit, no
   final state). TCP connect timeouts make the real period longer than 3 s.
4. Ends only by `1B` / pipe close (→ `02 0F`, exit 0).

A reply containing `malformed query` → `U→L 05 "malformed version request"`, `U→L 02 01`.

---

## 6. Version check (overview)

**PROVEN** (`send_version_request` `0x209BC`, `on_version_response` `0x20DBD`)

- Plain HTTP/1.0 GET built by `vostok::network_core::http_client` (`GET `, ` HTTP/1.0\r\n`,
  `Host: `, `Accept: */*\r\n`, `Connection: close\r\n\r\n`; strings at `0x10115C`..`0x1011A0`).
- Host and port come from argv[12]: host = text before the first `/`, service = text after
  `:` or `"http"`. Path = `sprintf("%s/?update_type=%s&version=%s&download_type=%s", …)`
  (`0x20B02`) with: path part of argv[12] (may be empty); `full_update` if action is 1 else
  `update`; argv[3] converted to narrow (max 100 chars); `http` if download type is 1 else
  `torrent`.
- Response: the first line of the body (read up to `\n`, `0x20F21`):
  - contains `malformed query` → error;
  - equals `<you use the latest version!>` → state 0x0E;
  - equals `<server maintenance!>` → state 0x16;
  - otherwise it is a download URL. File-name extension decides the content type
    (`0x20D7B`): `.exe` → 1, `.sup` → 2, `.torrent` → 3, anything else → error
    `invalid file extension on version response (%s)`. Type 3 → the `.torrent` suffix is
    stripped to get the payload name and `start_torrent_loading` runs; types 1 and 2 →
    `start_direct_download`.
- The version string is whatever the launcher passes as argv[3]; the updater does not read
  it from disk or registry. (Its own build string `0.26g0` at `0x100BE0` is only used for
  crash reports.)

---

## 7. Self-update / file replacement (overview)

**PROVEN**

- `updater::copy()` `0x1D317` (action `copy`): takes its own path (`GetModuleFileNameW`),
  strips four path components to get the install root (so it expects to live in
  `<root>\game\binaries\x86\`; fewer components → log `copy:: counter != 4`, state 1), and
  copies from its own directory into `<root>\temp` (`0x1D490`): `survarium_launcher.exe`,
  `survarium_updater.exe`, `bugtrap.dll`, and `apatch.exe` if it exists (`0x1D16C`). Each
  copy (`0x1D1DB`) is skipped when source and target already have identical file
  information, otherwise `CopyFileW` with up to 64 attempts and a `Sleep` between them.
  Any failure → state 1; success → state 0x13. No message other than the state.
- The installer (`install` `0x1CA0F`, run on a worker thread from `start_install`
  `0x1C3FF`) is given the package file and the folders from argv. It executes package
  steps: shell commands through `comspec` / `cmd.exe` (`0x10C6CC`, `0x10C6E0`), extraction,
  binary patching by spawning `"%sapatch.exe" "%s" "%s" "%s"` (`0x10BFCC`,
  `CreateProcessW`), and transacted moves (`ktmw32` `CreateTransaction` /
  `CommitTransaction` / `RollbackTransaction`, `MoveFileTransactedA`).
- `create_shortcut` (action 7, `0x24A50`): COM `IShellLinkW` (SetPath, SetIconLocation,
  SetArguments, SetDescription) + `IPersistFile::Save` to `<argv2>.lnk`.

**LIKELY**

- The launcher runs `copy` first, then starts the copies in `<root>\temp` to do the real
  update so that the files in `game\binaries\x86` can be overwritten. (The launcher has
  the literal `copy` right next to its pipe-name code at `0x14865`, and "Copying failed.
  Retrying...".) Who starts what, and when, is on the launcher side.

**UNKNOWN / not found**

- No updater message tells the launcher to restart itself. The complete sender list
  (section 3) has nothing of that kind; `send_ask_to_restart_message` is the
  failed-install question (3.9). The launcher's `restart_launcher` must be triggered by a
  state (0x11 / 0x13 are the candidates) — to be confirmed from the launcher.
- No explicit "half-finished install" detection was found in the updater. What exists:
  installs are wrapped in a kernel transaction; downloads resume from `.fastresume` files
  (HTTP: size record; torrent: resume data, a key `unfinished` marks an incomplete one,
  `0x249D7`); a failed install deletes the package and re-runs the version check (3.9).

---

## 8. Package verification (overview)

**PROVEN** (`0xB71A7` package open; `vostok::crypt::verify_sign` `0xB8A20`,
`verify_sign_with_certificate` `0xB8D10`, `verify_sign_impl` `0xB8430`)

- A `.sup` file starts with the 4-byte magic `!sup`, a 0x10B-byte header, then a signature
  and an optional certificate blob. Signature size 0 → rejected (installer error 4,
  `0xB7589`).
- First attempt, `verify_sign`: the certificate is taken from **the updater exe's own
  Authenticode signature** — `GetModuleFileNameW` → `CryptQueryObject(…, embedded PKCS#7
  signed)` (`0xB8A3D`, `0xB8A68`) → `CertFindCertificateInStore(…, CERT_FIND_SUBJECT_STR_W,
  L"survarium.com")` (`0xB8B4D`..`0xB8B5E`) → chain check with `CertGetCertificateChain` +
  `CertVerifyCertificateChainPolicy`, requiring the code-signing usage
  `1.3.6.1.5.5.7.3.3` (`0xB8190`). No certificate is embedded as data in the exe.
- Second attempt if the first fails with a status above 9 (`0xB75C0`..`0xB75E2`),
  `verify_sign_with_certificate`: `CertCreateCertificateContext` on the certificate blob
  carried in the package, same chain check.
- `verify_sign_impl`: `CryptAcquireContextA`, `CryptImportPublicKeyInfo`,
  `CryptCreateHash`, `CryptHashData`, `CryptVerifySignatureA`; failure text
  `Resource hash mismatch`.
- Failures are reported through `on_install_catch_error` as code 6 (→ id 0x16, question
  0x0E, retry possible) or code 7 (→ id 0x17, no retry); final installer error 0x15.

Consequence: a replacement updater exe that is not signed by the original publisher cannot
reproduce the first path; it would have to do its own integrity checking.

---

## 9. What a privacy-minded replacement must NOT reproduce

**PROVEN** unless marked.

| what | evidence |
|---|---|
| Version polling: HTTP GET to the update server from argv[12] every 60 s while "latest" / "maintenance", every 3 s while failing | section 6, `0x20FC4`, `0x20C97` |
| Direct HTTP downloads from the URL the server returns (with `Range:` resume) | `start_direct_download` `0x21919` |
| BitTorrent: libtorrent session with peer id prefix `LT`, listening on the port from argv; **DHT, local service discovery, UPnP and NAT-PMP are all started** (`0x228AF`..`0x228D6` → `0x325F4` DHT object 0x638 bytes, `0x32868` LSD, `0x32984` UPnP, `0x32890` NAT-PMP) | `start_libtorrent_session` `0x2276D`; UPnP/LSD identified by their multicast constants in the constructors `0x81CB9`, `0x963EB`; DHT / NAT-PMP assignment is LIKELY (by elimination and object size) |
| Router port mapping (UPnP SOAP `AddPortMapping`, NAT-PMP) and SSDP / LSD multicast on the LAN | same |
| Trackers named in the downloaded `.torrent`; no tracker or DHT bootstrap host is hard-coded in the exe (no such strings) | string scan |
| Seeding: uploads every package in `downloads\torrent\*.sup.torrent` to strangers whenever the state is "latest" in torrent mode, during installs, and in `seed` mode | `start_seeding` `0x1E921`, callers `0x21013`, `0x1C389`, `0x1B5CF` |
| Crash reporting through BugTrap (`BugTrap.dll` / `BugTrapN.dll` loaded from disk): support server `http://crasheat.survarium.com/process.php` port 80, support e-mail `updater_crash_reports@survarium.com`, support URL `http://support.survarium.com`, log file attached | `0xBA7B0`, `0x110A3`, `0x110D1`, `0xBA536` |
| Log files: `<user data folder>\…\<name><timestamp>.log` (format `%s/%s%s%d%s%s%s%02d%02d%02d-%02d%02d%02d%s`), containing the full command line and the Windows user name | `0x100CCC`, `0x1B187`, `0x1B00C` |
| Process enumeration with `OpenProcess(PROCESS_ALL_ACCESS)` on every process | `0x22B9A` |
| Running shell commands and `apatch.exe` from a downloaded package | section 7 |
| Shortcut creation (`.lnk`) | `0x24A50` |
| Modal `MessageBox` dialogs when the pipe is not connected | `0x24561`, `0x2468C`, `0x24552` |

Not present in the updater (**PROVEN** by the import table and string scan): registry
access (no `Reg*` import), service control (no SCM import), WinINet / WinHTTP (it uses raw
Winsock through boost::asio), named mutexes.

---

## 10. Open unknowns

1. Meaning of argv 6, 7, 9, 11 of update (5, 6, 8, 10 of install): two folders, one file,
   one string handed to the package installer. The launcher side should name them.
2. Exact suffix after `vostok_updater_pipe-` and whether the launcher creates the pipe in
   message or byte mode (does not matter to the original updater).
3. Which state makes the launcher restart itself / start the game.
4. Whether the launcher reads the `info` payload of ids 0x14 / 0x15.
5. Real id of the seeding-debug toggle (default branch) and who sends 0x1E.
6. Meaning of the two u32 fields from libtorrent `session_status` in id 0x18
   (offsets +0x20 and +0x94 of the status object, `0x1BA05`, `0x1BA17`).
7. Behaviour when a required argv is missing (reads past the argv array) — not traced.
8. Whether `eta` in progress messages is really 0 during downloads (member is not
   initialised by the constructor code read at `0x1AAFC`..`0x1AD4B`).
