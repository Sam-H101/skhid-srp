## SRP local test report — Windows 11, all open PRs merged (#16–#28 + #22)

**Environment**
- Windows 11 Pro (build 26300), x64. Regional format `uk-UA` (see finding 5)
- Rust `1.101.0-nightly (a30aa9064 2026-10-08)`, MSVC 14.29 (VS 2019 Build Tools), x86 `cl` 19.29 for the hit-report patch
- OpenSSL 3.5.9 (slproweb Win64, `OPENSSL_DIR`/`OPENSSL_STATIC`/`OPENSSL_LIB_DIR` per README); `Cargo.lock` unchanged from upstream
- Client: `survarium_setup_v0100b.exe` (sha256 `4450f1dd…4a99` ✅), `survarium.exe` sha256 `6cde21fa…f737`, PE timestamp `0x518e2ed6` ✅
- Servers: `cargo run --bin srp` from repo root, default `127.0.0.1` binds (TCP 80/1234/1235, UDP 1236), no `SRP_*` vars

**Commits tested**
- base `0.100b` @ `883ef5c71`
- `pr/28` @ `59763dec7` (contains #16–#27) merged with `pr/22` @ `8b299484f` → `c76eee3a8` (clean merge)
- plus one local fix to `client-patches/hit-report/loader.cpp` (finding 1)

**Build / unit tests**
- `cargo build --bin srp` ✅ (only warning: unused workspace dep `sqlx`)
- `client-patches/hit-report/build.bat` (x86) ✅, warning-free at `/W4`; DLL + loader are PE32 i386
- `cargo test` ✅ 51 passed, 0 failed, 3 ignored (match-server: 29 passed / 1 ignored)

**Live test results (single client, DLL loaded via loader)**
| # | Test | Result |
|---|---|---|
| 1 | Login → lobby (profiles, inventory, shop prices) | ✅ pass — **but the first login attempt after every game start fails**; pressing Enter again works (finding 2) |
| 2 | Equip / unequip / move items | ✅ pass |
| 3 | Ready → match loads `level_03_evn` | ✅ pass |
| 4 | Spawn and move (WASD) with #22 DLL | ✅ pass (without the DLL: camera only, as expected) |
| 5 | Hits → 25 dmg → death → 10 s respawn | ⏭️ not tested — needs a 2nd player; client allows only one instance per PC ("Survarium application already running") |
| 6 | Second client on same PC | ❌ not possible — same single-instance check |
| – | Session stability | ❌ player dropped from the match after the window lost focus / stalled (finding 3) |
| – | Match never ends | expected (no timer/round end) |

**Findings**

1. **PR #22 loader quotes every game argument; the engine rejects it.** `append_argument` wraps each arg in `"…"`, so the game gets `"-no_splash_screen"` and fails with *Command line argument `no_splash_screen"' has not been registered*. The engine's parser (`vostok/core/sources/command_line.cpp`) only accepts a quoted exe path and quoted values (`-key="value"`), not a quoted `-key`. Local fix: quote only argv[1] and args containing whitespace:
   ```cpp
   bool const quote = !used || !length || strpbrk(argument, " \t");
   if (!quote) {
       memcpy(destination, argument, length + 1);
       return true;
   }
   ```
2. **First TLS login after each game start always fails; a retry in the same window succeeds.** Server side: `login-server/src/lib.rs:138` `ssl.accept(stream).unwrap()` panics — the client (OpenSSL 1.0.0g, TLS 1.0) hangs up right after `ServerHelloDone` without an alert. Captured flight is valid (TLS 1.0, `ECDHE-RSA-AES256-SHA`, secp256r1, MD5+SHA1 SKE signature verified against the login cert). Identical failure with a Python/OpenSSL 3.0 server, so it is client-side state on the first attempt (possibly OpenSSL 1.0.x's slow Windows RNG seeding — unconfirmed). Workaround: press Enter again.
3. **Match server reaps a player after 5 s of client silence, with no way back.** `IDLE_TIMEOUT = 5 s` (`match_server.rs:42`, PR #21). Reproduced twice: once after alt-tab, once with no user action. After the reap, every packet from the same client is logged as `dropping malformed connection attempt` / `ignoring pre-registration match message` (thousands of lines), and the client eventually shows **"match server connection lost"**. The client never sent `initiate_disconnection`. A longer timeout and/or re-adopting a known `session_id`/address would fix it.
4. **Lobby: every connection ends in `Connection handler panicked`.** `lobby_server::run` is `-> !` with `tcp_client.read(...).unwrap()`, so a normal client disconnect (incl. switching to the match) is logged as a panic. Noisy but harmless thanks to `catch_unwind`.
5. **Cyrillic UI shows boxes unless the Windows *regional format* uses CP1251.** The engine does `setlocale(LC_CTYPE, "")` (`core_entry_point.cpp:156`) and converts the CP1251 localization with `mbstowcs_s` (`text_translator.cpp:51`). With regional format `en-US` (CP1252) the strings become Latin-1 mojibake that the embedded SWF fonts can't draw → boxes, even though the system locale is CP1251. Switching regional format to `uk-UA`/`ru-RU` fixes it. Worth a README note.
6. Smaller notes:
   - README still documents four separate binaries; `srp` (single process) is the real entry point. CLAUDE.md still says match is a separate process and bind defaults to `0.0.0.0` (code: `127.0.0.1`).
   - Windows PowerShell 5.1 splits an unquoted `-client=127.0.0.1:1234` at the dot (`-client=127 .0.0.1:1234`) and the game exits silently — quote it in PowerShell.
   - Ctrl-C in the `srp` terminal only stops the browser thread, which the supervisor restarts; the process has to be killed.
   - OpenSSL 4.0.x needs `openssl-sys` ≥ 0.9.11x (0.9.109 refuses 4.x at build time). I did not get a clean live result on 4.0 (only first-attempt logins were tried there, see finding 2), so 3.x is what was tested.

**Crashes**: no server process crash. Contained panics: login TLS accept (finding 2), lobby read-on-disconnect (finding 4). No client crash.
