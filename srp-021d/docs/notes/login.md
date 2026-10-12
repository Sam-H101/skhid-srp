# 0.21d scoping: everything before the lobby

Scope: launcher and startup, login server, browser server, sessions, presence and sign-out pings, server addresses, certificates and version checks.

Builds compared:

- **0.100b:** `S/survarium_disasm.txt`, image base 0x400000. 0.100b addresses below are from this disasm. The decomp is at `tools/vostok/sources/vostok/network/sources/login_client_impl*.cpp`.
- **0.21d:** `S/v021d_disasm/survarium_disasm.txt`, image base 0x10000, build date 2014-04-24.

Enum names and struct layouts for 0.21d come from parsing the LF_ENUMERATE and LF_MEMBER records in `survarium.pdb`. The launcher and updater were disassembled with dumpbin into `S/scope021d/launcher_disasm/{launcher,updater}.txt`.

Legend: **[V]** = verified in the disassembly. **[I]** = inferred from code, not live-tested. **[U]** = unverified.

---

## 0. TL;DR

| Area | 0.100b → 0.21d | srp effort |
|---|---|---|
| Finding servers | Same `-client=host:port` key. Same browser HTTP step on port 80, which now adds `&ver=0.21d`. | **Unchanged** for the browser server. |
| Login request | Grew by 11 bytes: a 12-byte version field `"0.21d"` (was 8 bytes, `"0.100b"`), a `connect_from_eula_wnd` byte and a u32 `online_queue_order_id`. The email field now holds up to 99 chars. | **Small change**: parse it. The current code works by accident. |
| Login reply 0x08 | Gains 12 trailing bytes: lobby IPv4+port and messaging IPv4+port. Those are used only when `-client` is absent. | **Small change**: append the 12 bytes. Probably optional with `-client`. |
| New login replies | 0x15 `sign_in_eula_check_failed`, 0x16 `sign_in_added_to_online_queue`. | None. srp never needs to send them. |
| TLS and certificate | Same OpenSSL 1.0.0g, SSLv23 method, `verify_peer`, same cert path. The shipped cert is byte-identical to 0.100b's original (expired 2014). | **Unchanged**. Copy srp's re-issued cert over it as for 0.100b. |
| Password | Same: `[u8 len][plaintext]` over TLS, at most 47 chars. | Unchanged |
| Presence pings | **Rewritten.** UDP to the *login TCP port* (not 25100), every 10 s, 5 bytes `[0x03][u32 session_id]`. A queue variant `[0x04][u32 queue_id]` also exists. | **Small change**: new port and 5-byte format. |
| Sign-out | Byte-identical: `[0x02][u32 session]`, then TLS, then `[len][password]`, with no answer. | Unchanged |
| Launcher | The new `survarium_launcher.exe`, updater and service are not needed. The game exe runs standalone. | None |
| New startup constraint | Single-instance mutex `survarium_or_its_installer_is_already_running`. | None for srp. It blocks two clients on one PC. |
| Steam, anti-cheat, HWID, auth tokens | None found. | — |

---

## 1. How each client finds its servers

### Command line

Both builds use the same command-line key for the login server:

- 0.21d: `s_net_login_client`, set up by the dynamic initializer at 0x68C0EA.
- 0.100b: same key, set up at 0x7D79C0.
- Key name `client`, help text "connect to server" **[V]**. Usage is `-client=host:port`.

In 0.21d, `game::on_base_resources_created` (0x1528D0 onward) does this **[V]**:

- If `-client` is set as a string containing `':'`, it calls `create_and_assign_network_client(<value>, is_set(-spectator))`.
- Else, if the replay key (0x56D5B50) is set, it creates a `replay_network_client`.
- **Else it uses the hardcoded default `"188.93.23.27:65000"`** (0x152B1F). So the production login port in 0.21d was 65000.

0.100b hardcoded its fallbacks in a different place: lobby `188.93.23.27:25101` and messaging `:25102` inside `http_query_server_connection_info` (0x704FCA–0x704FE6).

`-spectator` (s_is_spectator) makes `connect_to_login` (0x16DD99, flag `+7A20`) connect straight to a match server. Login is skipped. This is a match-side feature only **[V]**.

### Browser HTTP step

`network_client::http_query_server_connection_info`:

| | 0.100b @0x704E90 | 0.21d @0x16DA1A |
|---|---|---|
| When `-client` is set | HTTP GET to host `server_browser_address`, path `sprintf(fmt, initial_query, type, local_ip, login_ip, ...)` | same |
| Format string | `%s&type=%d&local_ip=%s&login_ip=%s` | `%s&type=%d&local_ip=%s&login_ip=%s&ver=%s` with `"0.21d"` **[V]** |
| When `-client` is not set | hardcoded `188.93.23.27:25101` / `:25102` | `sprintf("%s:%d", m_lobby_server_ip, m_lobby_server_port)`, and the same for messaging. Both come from the login reply. **[V]** |
| `type` values | 2 = lobby, 4 = messaging | same (called from `network_client::tick` 0x1A2B25 / 0x1A2BDD) |

The HTTP client is `vostok::network_core::http_client::get` (0.21d 0x282xxx, 0.100b 0x78ABxx). The request is identical in both: `GET <path> HTTP/1.0\r\nHost: <host>\r\nAccept: */*\r\nConnection: close\r\n\r\n`. The response is checked for `HTTP/` + `200`.

- Port: 0.100b resolves service `"http"`; 0.21d passes `"80"` **[V]**. Both are port 80, and neither accepts a port inside the host string.
- The response body is parsed by `on_http_result_ready` (0.21d 0x16DB94) as `host:port` (`strchr(':')`, `sscanf("%d")`). This is the same as 0.100b.
- Type 2 then calls `tcp_packet_client::connect(host, port)`. Type 4 calls `messaging_client::connect(server_connection_info)`, which carries the login session id.

**srp `browser-server`:** `QueryParams` has no `deny_unknown_fields`, so the extra `ver=0.21d` parameter is ignored. It returns `"<lobby host>:<port>"` for both types, as before. **Works unchanged [I].**

Whether 0.21d's messaging connection can share the lobby port 1235 depends on the first-byte disambiguation. That belongs to the lobby/messaging study.

### Ports summary

| Server | 0.100b (srp) | 0.21d |
|---|---|---|
| Login TCP | from `-client` (srp 1234) | from `-client`; production default 65000 |
| Presence UDP | fixed 25100 (`_itoa_s(0x620C)` 0x7A236C) | **= login TCP port** (`m_host_port`). The constant 0x620C does not appear anywhere in the 0.21d disasm. **[V]** |
| Browser HTTP | 80 | 80 |
| Lobby / messaging | from browser reply | from browser reply (with `-client`) or from the login reply (without it) |

---

## 2. Login protocol, byte for byte

Functions:

| | 0.100b | 0.21d |
|---|---|---|
| `sign_in` | 0x7A3370 (`host, port, account_name, password`) | 0x23E87D (`sign_in_info const&`) |
| `sign_in_on_connected` | 0x7A3150 | 0x23E759 |
| `on_user_name_answer_received` | — | 0x23E430 |
| `on_sign_in_handshaked` | — | 0x23E34F |
| `on_sign_in_password_written` | — | 0x23E1CE |
| `on_sign_in_answer_received` | — | 0x23DCE1 |

`sign_in_info` (PDB) is `{ char* host; u16 port; char* account_email; char* password; bool connect_from_eula_wnd; }`.

Relevant 0.21d `login_client_impl` members (PDB offsets):

| Offset | Member |
|---|---|
| +0x188 | `m_session_id` |
| +0x18C | `m_online_queue_order_id` (initialised to 0xFFFFFFFF) |
| +0x190 | `m_online_order_place` |
| +0x194 | `m_online_orders_total` |
| +0x198 | `m_lobby_server_port` |
| +0x19A | `m_messaging_server_port` |
| +0x19C | `m_host_port` |
| +0x19F | `m_host[64]` |
| +0x1DF | `m_password[48]` |
| +0x20F | `m_account_email[100]` |
| +0x273 | `m_host_ip[16]` |
| +0x283 | `m_lobby_server_ip[16]` |
| +0x293 | `m_messaging_server_ip[16]` |
| +0x2A3 | `m_data[167]` |
| +0x34A | ping send buffer, 64 bytes |
| +0x38A | ping receive buffer, 64 bytes |
| +0x3CA / +0x4CA | browser address and initial query, 256 bytes each |

### Client → server, step 1 (plain TCP, one `async_write`)

| Field | 0.100b | 0.21d |
|---|---|---|
| type | `01` | `01` |
| email length | u8 (field 32 bytes, so ≤31) | u8 (field 100 bytes, so ≤99) |
| email | raw | raw |
| version | **8 bytes** `"0.100b\0\0"` (`strings::copy`, 0x7A325F) | **12 bytes** `"0.21d"`, zero-padded (`strcpy_s(buf,12,"0.21d")`, 0x23E7CF) |
| — | — | **u8 `connect_from_eula_wnd`** (0 normally, 1 after the EULA window) |
| — | — | **u32 LE `online_queue_order_id`** (0xFFFFFFFF unless rejoining the queue) |
| total | 10 + n | **19 + n** (capped at 167) |

srp reads `1 + len + 6` bytes (`sign_in_length`). It also **does not check the version**: it only logs it. For 0.21d it would log `"0.21d\0"`. The remaining 11 bytes are normally read in the same `read()` and ignored. If they arrived in a later segment, they would be fed into `ssl.accept` and break the handshake.

**Small change:** pick the trailer size from the version prefix. `"0.100b"` → 8 bytes; `"0.21d"` → 12 + 1 + 4.

### Server → client, step 2 (plain): one byte

Both builds read exactly 1 byte (`async_read`):

- `0x0B` → TLS handshake.
- `0x0A` → "invalid user name or password".
- Anything else → passed to the UI as an error code.

0.21d logic at 0x23E547 is the same as 0.100b's **[V]**. srp sends `0x0B`. **Unchanged.**

### TLS

TLS setup is the same in both builds **[V]**:

- `SSLv23_method`.
- `load_verify_file("../../resources/ssl/survarium_login_server.crt")`, string at 0.21d 0x82A40C, path relative to the CWD.
- `verify_peer`, with `verify_ssl_certificate` simply returning `preverified` (0.21d 0x23B2A5, 0.100b 0x579690).
- Both builds statically link **OpenSSL 1.0.0g (18 Jan 2012)**, so the client speaks TLS 1.0 at most.

The 0.21d install's `resources/ssl/survarium_login_server.crt` is byte-identical to 0.100b's original. It has md5 e96aef3d…, the same as `Survarium/resources/ssl/survarium_login_server.crt.expired-2014`, serial E21DC76CFA518B97, and expired 2014-05-02. srp's re-issued `certs/survarium_login_server.crt` has the same public key (pubkey md5 88d60605…) and is valid to 2037.

**Unchanged:** copy srp's cert into `<0.21d>/resources/ssl/` and run with CWD = `binaries/x86`. Note the folder is `x86` in 0.21d, versus `win32` in 0.100b.

### Client → server, step 3 (TLS)

Both builds send `[u8 len][password bytes]`, plaintext, with no hashing **[V]**. **Unchanged.**

### Server → client, step 4 (TLS)

The client does one `async_read_some`:

- 0.100b: up to **70** bytes.
- 0.21d: up to **82** bytes (`push 52h`, 0x23E31F).

The whole reply must therefore arrive in one TLS record. srp writes it with one `write_all`, so it does.

0.21d server message enum (PDB `vostok::login::server::messages_enum`):

| Value | Name |
|---|---|
| 0x08 | `connection_info_message_type` (renamed from `servers_connection_info…`) |
| 0x09 | `password_request` |
| 0x0A | `invalid_user_name_or_password` |
| 0x0B | `valid_user_name` |
| 0x0C | `sign_in_attempt_interval_violated` |
| 0x0D | `sign_out_successful` |
| 0x0E | `occupied_user_name` |
| 0x0F | `send_sign_up_info` |
| 0x10 | `sign_up_successful` |
| 0x11 | `user_banned` |
| 0x12 | `user_restricted_by_access_level` |
| 0x13 | `sign_in_user_already_signed_in` |
| 0x14 | `sign_in_invalid_version` |
| **0x15** | **`sign_in_eula_check_failed`** (new) |
| **0x16** | **`sign_in_added_to_online_queue`** (new) |
| 0x1F | `invalid_message_type` |

0.21d client enum: `sign_up=0`, `sign_in=1`, `sign_out=2`, **`ping_online=3`**, **`ping_queue=4`**, `invalid=7`.

#### Layout of reply 0x08

0.100b layout (decomp, srp current):

```
08 | u8 L1 | browser_address[L1] | u8 L2 | initial_query[L2] | u32 LE session_id
```

0.21d layout (0x23DEDB–0x23DFB3) **[V]**:

```
08 | u8 L1 | browser_address[L1] | u8 L2 | initial_query[L2] | u32 LE session_id
   | u32 lobby_ipv4 | u16 LE lobby_port | u32 messaging_ipv4 | u16 LE messaging_port
```

How the IPv4 is encoded: the client reads a raw u32, applies `htonl`, and the result becomes `address_v4::to_string`. So the wire carries the IP's host-order integer in little-endian, for example 127.0.0.1 = `01 00 00 7F`, i.e. `u32::from(ip).to_le_bytes()` **[I]**. The ports are raw LE u16 with no `ntohs` **[V]**.

Size limit: L1 + L2 ≤ 51 in 0.21d (82 − 19). It was ≤ 63 in 0.100b. srp's `"127.0.0.1"` + `"/hello?unused=1"` is 24.

The lobby and messaging fields are only read in the **non-`-client`** branch of `http_query_server_connection_info`. With `-client`, srp's 0.100b-format reply would leave them as stale `m_data` bytes, which are never used. So the current srp reply likely works as-is **[I, not live-tested]**. Appending the 12 bytes (`config.lobby_server` for both) is cheap and makes it correct.

After 0x08 the client does three things:

1. `connect`s its ping UDP socket to `<login peer IP>:<m_host_port>` (UDP resolver, 0x23DFC9–0x23E0AD).
2. Sets state = 3 (signed_in) and closes TCP.
3. Calls back with 8, then calls `schedule_ping`.

`network_client::on_connected_to_login` (0.21d 0x16D2C6) maps message 8 to: set the lobby and messaging "need connect" flags, then `game::switch_to_lobby` **[V]**. Other codes go to `switch_to_login(status)`: 0x15 → status 9, which shows the EULA window; 0x16 → status 0x0A, "In online queue Nth of M".

#### Layout of reply 0x16 (queue), for completeness

```
16 | 1 byte skipped | u32 online_queue_order_id | u32 place | u32 total
```

The client stays in state 2 and pings `[04][u32 queue_id]`. It listens for 9-byte UDP answers `[?][u32 place][u32 total]` (`process_ping_answer` 0x245E8F). When the place reaches 0, `login_menu::update_online_queue` (0x170536) signs in again with the stored queue id.

**srp never needs to send 0x15, 0x16 or 0x14.**

### Session id

Both builds take the session id from the 0x08 reply as a u32 LE. In 0.21d `on_http_result_ready` copies it from `login+0x188` into the lobby `server_connection_info` (0x16DBB0). It also goes into sign-out and into `ping_online`. Same role as in 0.100b.

---

## 3. Browser server

| | 0.100b | 0.21d |
|---|---|---|
| Request | `GET {initial_query}&type={2\|4}&local_ip={ip}&login_ip={ip}` | `GET {initial_query}&type={2\|4}&local_ip={ip}&login_ip={ip}&ver=0.21d` |
| Host / port | `browser_address` from login reply : 80 | same |
| Response | HTTP/1.x 200, body `host:port` | same |
| Retry | reconnect every 5 s (lobby) / 3 s (messaging); after >3 lobby failures, `sign_out` + back to login | same thresholds (0x1388 / 0xBB8 / `cmp …,3`) **[V]** |

`initial_query` is srp's `/hello?unused=1`, which makes the path start with `/`. **srp browser-server: unchanged.**

---

## 4. Session, presence and sign-out

### Presence pings

| | 0.100b (`login_client_impl::ping` 0x79847A) | 0.21d (`ping` 0x245DA3, `schedule_ping` 0x245D3D) |
|---|---|---|
| Destination | login IP : **UDP 25100** | login IP : **UDP `m_host_port`**, i.e. the same number as the login TCP port (1234 under srp) |
| Payload | 4 bytes `u32 session_id` | **5 bytes** `03` + `u32 LE session_id` when signed in (state 3); `04` + `u32 queue_id` while queued (state 2, id ≠ −1) |
| Interval | 1 s | **10 s** (`expires_from_now(0x989680 µs)`). The first ping comes 10 s after sign-in; the constructor timer is 1 s. **[I]** for µs resolution, which is the boost default. |
| Failure handling | stops after 10 failed sends | `on_ping_sent` (0x245D97) always reschedules, so errors are ignored and it never stops |
| Answers | none | only in queue state (`start_ping_answer_acception` is called only on 0x16) |

srp's `run_presence` binds `SRP_LOGIN_UDP_PORT` (default 25100) and accepts exactly 4-byte datagrams. **Under 0.21d it hears nothing.** Sessions would then live only on the lobby/chat/match sightings that `SessionStore` already counts.

**Small change:**

- Bind UDP on the login port. `SRP_LOGIN_UDP_PORT=1234` works today: UDP and TCP may share the number.
- Accept `[0x03][u32]` (5 bytes) in addition to the 4-byte form.
- Optionally ignore `[0x04]…`.

No reply is needed.

### Sign-out

Functions in 0.21d: `sign_out_on_connected` 0x24194D, `on_sign_out_written` 0x2417DD, `on_sign_out_handshaked` 0x2416FD, `on_sign_out_password_written` 0x2415EE.

Sequence: `[02][u32 LE session_id]` in the clear, then **no read**, straight into the TLS handshake. Then `[u8 len][password]`, then close, with no answer read. This is byte-identical to 0.100b (`login_client_impl::sign_out` 0x7A1CDC). srp's `handle_sign_out` (`read_exact(4)` then `accept_tls`) **works unchanged**.

When it is sent: the network-client tick (0.21d 0x1A2AE9) after more than 3 failed lobby reconnects, and on quit. Same as 0.100b.

---

## 5. What else is new in 0.21d before the lobby

- **EULA:** handled entirely through server message 0x15 plus `sign_in_info.connect_from_eula_wnd`. `login_menu::set_status` calls `root.show_eula_wnd` (0x170449), and the EULA text comes from `resources/flash_movies/login_men…` (`/eula`). If srp always replies 0x08, the EULA never appears and nothing is needed.
- **Online queue:** messages 0x16, `ping_queue`, and UDP answers (above). Not needed.
- **Single-instance mutex (new):** `WinMain` (0x12644) calls `CreateMutexA("survarium_or_its_installer_is_already_running")`. On `ERROR_ALREADY_EXISTS` it shows a MessageBox and calls `TerminateProcess`. 0.100b has no such mutex (string absent from the 0.100b exe). **You cannot run two 0.21d clients on one Windows session.** That matters for local multi-client tests: use a second machine or VM, or close the mutex handle (e.g. with Process Explorer) **[U, workaround untested]**.
- **Launcher (`survarium_launcher.exe`, new; not required):**
  - `on_play_button_clicked` (launcher 0x17450) calls `get_game_launch_parameters` (0x17320). That reads HKLM `Software\Vostok Games\Survarium` values `file to execute`, `arguments` and `working folder`.
  - It then `CreateProcessW`s `"<file>" -localization=<english|russian|ukrainian> <arguments>`.
  - The registry values are written by the installer, which is not available, so the production `arguments` content is **[U]**. Since survarium.exe defaults to `188.93.23.27:65000`, they probably held no `-client`.
  - The launcher also checks CPU/RAM/VRAM and talks to `survarium_service.exe` over `\\.\pipe\survarium_service_to_launcher`. It runs `survarium_updater.exe`, which calls `GET /version_checker/?update_type=%s&version=%s&download_type=%s` on the registry "update server" and downloads by HTTP/torrent (libtorrent 0.16.11).
  - None of this is needed: **survarium.exe has no launcher handshake, token, pipe or registry dependency** in WinMain. Its only imports are KERNEL32; the rest is loaded dynamically. So run `survarium.exe -client=127.0.0.1:1234` from `binaries\x86`, as for 0.100b. Whether `-localization` must be passed is **[U]**; it presumably defaults.
- **Return path into the launcher:** `game::exit_and_launch_launcher` (CreateProcess `survarium_launcher.exe`) is reached only from `lobby_client::read_online_stats` (0.21d ~0x16xxxx, line 520047). That is lobby territory, possibly an "update required" or maintenance signal. **[U]**, for the lobby study.
- **HTTP calls from survarium.exe:** only the browser query. `http_client::get` has a single caller. Crash reports go to `http://crasheat.survarium.com/process.php` through BugTrap, and only on crash.
- **Not present** (string, import and symbol searches over the 0.21d exe and PDB): Steam, anti-cheat (PunkBuster, XIGNCODE, GameGuard), hardware id (`GetVolumeInformation`, `GetAdaptersInfo`), registry reads, region selection, auth tokens.
  - The `region` hits are rendering or player-region UI only.
  - The `token` hits are the Scaleform/Boost parser only.

---

## 6. Per-item verdict for srp

| Item | Verdict | What to do |
|---|---|---|
| Server discovery (`-client`, browser, port 80) | **Unchanged** | Launch with `-client=<srp>:1234`. |
| Browser `/hello` | **Unchanged** | Extra `ver` is ignored. Returning the lobby address for type 2 and 4 still works if the lobby port handles the 0.21d messaging protocol (separate study). |
| Sign-in request parse | **Small change** (about 10 lines) | Version-aware trailer: 8 bytes for 0.100b, 17 for 0.21d. Optionally log `connect_from_eula_wnd` and queue id. |
| Version check | **Nothing** (srp never rejects) | If one build of srp must serve both clients, dispatch on the version string. Reply 0x14 is available for a mismatch. |
| 0x0B / TLS / password | **Unchanged** | Copy `certs/survarium_login_server.crt` into the 0.21d `resources/ssl/`. |
| Reply 0x08 | **Small change** (about 6 lines) | Append `lobby_ip u32(le of host-order) + lobby_port u16le + msg_ip + msg_port`. Probably unnecessary with `-client` [I]. Keep L1 + L2 ≤ 51. |
| Presence pings | **Small change** | UDP on the login port; parse `[03][u32]`; 10 s cadence is fine against the 10-min idle limit. |
| Sign-out | **Unchanged** | — |
| Queue, EULA, launcher, updater | **Not needed** | — |
| Two local clients | **Blocked by the client** | Single-instance mutex (above). |

Overall: the pre-lobby path needs **small, contained changes**: one parser, one reply trailer and the ping listener. No rewrite is needed.
