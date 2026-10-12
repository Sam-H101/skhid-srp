# Client patches for 0.100b

**The 0.100b server needs no patch of `survarium.exe`.** The original client
connects as it is, once the login certificate below is in place. What else is
here is optional.

| | |
|---|---|
| Original exe | 12 235 264 bytes, sha256 `6cde21fac0fc508c140814c83d8a9d0158c0bff8f99c322cd7454c3d3103f737` |

## 1. The login certificate (needed)

- **What:** `survarium_login_server.crt` goes over the game's
  `resources/ssl/survarium_login_server.crt` (keep the old one).
- **Why:** the client verifies the login server's certificate against that
  file. The game's own ran out in 2014, so its first sign-in attempt fails
  with "certificate verify failed" (the client's second try skips the
  check). This one is the same name and key issued again (valid to 2037);
  the key is in `../certs/`, which the login server reads. It is the same
  file as `../certs/survarium_login_server.crt`.

Start the game with:

```
survarium.exe -no_splash_screen -client=<host>:1234
```

## 2. Optional: the hit report (`hit-report/`)

A small DLL, injected by a loader, for a server that wants the client's own
hit detection. Nothing of the file on disk is changed: the patch is made in
memory and taken back when the DLL unloads.

- It replaces the empty `network_client::on_player_hit_received` vtable
  entry, so the client sends the hit it detected to the match server.
- It removes the "match in process" check from the two places where the
  client attaches the local player's controls
  (`network_client::process_sync_response`, 0x005C5AB4, and
  `process_player_respawn`, 0x005C5C48: nine bytes of `cmp`/`jne` become
  `nop`s), so a player can move in a match that is kept in warmup.
- It is locked to the exe above: the PE timestamp, the image size, the
  original vtable entry and the original bytes of both sites are checked,
  and an unknown client is left alone.

`hit-report/README.md` has the wire format and how to build it
(`build.bat`, Visual Studio's compiler).

## 3. Developer patches from the SRP project (`../resources/patches/`)

Four small assembly patches that the SRP project's tool
`scripts/nasm-decompiler` builds and applies to the exe
(`cargo run --bin nasm-decompiler -- --patch-name <name> patch-survarium`;
each has the original bytes in `<name>_og.asm` and the new ones in
`<name>_patch.asm`, with its address in the first line). By their names and
sites they turn on the engine's console and the game's console
(`enable_console`, 0x6D392A; `enable_game_console`, 0x6D41CE) and change the
login menu's status handling (`login_menu_set_status`, 0x6F1E77) and the
match client's UDP setup (`udp_match_client`, 0x7477D2). They are tools for
development; the server does not need them, and they were not looked into
further here.
