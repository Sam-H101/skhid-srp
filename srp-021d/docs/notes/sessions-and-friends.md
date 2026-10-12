# Sessions, presence, refusals and friend lists

How the 0.100b client keeps its login session alive, signs out, takes a
refused session, and uses its friend and ignore lists, from `survarium.exe`
(addresses are the PDB-symbolled disassembly's) with the original server
sources (`tools/vostok/sources/vostok/login_server/`) where they agree. What
srp does with each is at the end of each section.

## Presence pings (UDP 25100)

> **0.21d:** the client pings `[0x03][u32 session id]` every 10 s to the login
> server's IP at the login TCP port number, and never stops; srp now listens
> there by default. See `v021d/login.md` §4. The text below is 0.100b.

Set up in `login_client_impl::on_sign_in_answer_received` when
`servers_connection_info` (0x08) arrives: the session id is kept at
`this+0x16C` (0x7A2355), and a UDP socket is connected to **the login
server's IP, port 25100** (`_itoa_s(0x620C)` at 0x7A236C; the port is fixed,
whatever `-client=` says). The first ping goes out at once.

`login_client_impl::ping` (0x79847A..0x798513) sends the 4-byte session id,
little-endian, with no header. After a send it re-arms its timer for **1 s**.
It never reads an answer. A failed send retries at once; after 10 failures in
a row it stops quietly (state 1, 0x798459): no dialog.

The original login server kept each session's last ping
(`set_last_activity_time`) and every `collect_interval` (60 s) signed out the
sessions unseen for `collect_no_ping_period` (600 s) (`login_server.cpp`,
`query_processor.cpp`, `entry_point.cpp`).

**srp:** the login server's `run_presence` listens on UDP 25100
(`SRP_LOGIN_UDP_PORT`, `SRP_LOGIN_UDP_BIND`; by default where the login server
binds) and counts a 4-byte datagram of a known session as a sighting; every
60 s `SessionStore::end_idle_sessions` ends those unseen for 600 s. Lobby and
chat connections hold their session (seen while open), and leaving a match
counts as a sighting, so a scripted client without pings keeps its session
while it plays. A session in a match or the queue never idles out.

## Sign-out (TCP, login port)

`login_client_impl::sign_out` (0x7A1CDC..0x7A1CF7) runs only when signed in
(state 3). It connects to the login server, writes `[0x02][u32 session id]` in
the clear, then at once does the TLS handshake and writes `[u8 len][password]`,
closes, and never reads anything. It is sent when the game quits (the
`login_client_impl` destructor), and when the lobby fails to connect more than
three times in a row (`network_client::tick`, 0x5C6239), before the client
goes back to the login screen.

**srp:** reads exactly the 4-byte id (the handshake follows with no answer in
between, so a longer read would eat it), then the password over TLS
(unchecked, never logged), then `SessionStore::end_session`. An ended session
can't open a lobby or chat connection; one still in a match stays until it
leaves, so its match result still reaches the account.

## Refusing a session: `invalid_session_id` (0x31), `invalid_password` (0x32)

The lobby sign-in carries only `[0x26][u32 session id]` (`lobby_client::
on_connected`, 0x5C85A5): no password ever reaches the lobby.

- Before `connection_successful`, `lobby_client::sign_in_on_packet_received`
  (0x5C8120) reads the type alone: 0x30 signs in; anything else is logged
  ("Lobby client received unknown SignIn message") and it keeps waiting.
- After it, `network_client::on_lobby_packet_received` (0x5C6B78) does
  `sub ecx,33h; jmp [ecx*4+5C732Ch]` with no range check: the table covers
  0x33..0x37 only, so 0x31, 0x32 (or another 0x30) **crash the client**.
- A closed connection: `lobby_client::on_error` (0x5C8440) counts a failure;
  `network_client::tick` waits 5 s (0x1388) and asks the browser again, and
  after more than 3 failures signs out and calls `switch_to_login`.

The real wrong-password path is the login server's
`invalid_user_name_or_password` (0x0A), at sign-in.

The chat client has no refusal: `messaging_client::sign_in_on_packet_received`
(0x799B20) takes only 0xCB and logs anything else. On a closed connection
`messaging_client::on_error` (0x766450) adds "Lost connection to messaging
server" to the chat and sets a reconnect flag, and `network_client::tick`
reconnects it 3 s later (0x5C63A8), with no limit.

**srp:** both messages are serialized as their type byte, and the lobby never
sends one after sign-in. A session the store doesn't know, or one that has
ended, gets `invalid_session_id` before `connection_successful` and a closed
connection; the chat just closes. So after an srp restart the client goes back
to the login screen instead of playing on a throwaway account. The standalone
`lobby-server` binary, which has no login server, still serves any session as
the default account.

## Friend and ignore lists (chat connection)

All of it is on the chat ("messaging") connection; the lobby protocol has none.
The client sends `[0xC4][action]...` (`messaging_client::*`, 0x5C8E9x..0x5C92Bx):

| Action | Body | Sent |
|---|---|---|
| 0 `add_friend` | `u32 account id` | Flash `add_friend(id)` from search results |
| 1 `remove_friend` | `u32 account id` | the cross on a friend's row |
| 2 `add_ignorable` | `u32 account id` | Flash `add_ignore(id)` |
| 3 `remove_ignorable` | `u32 account id` | the cross on an ignored row |
| 4 `find_players` | `u8 len` + name | the search box, 3 characters or more |
| 5 `query_friend_list` | – | after the chat sign-in, and after an accepted change |
| 6 `query_ignore_list` | – | the same |
| 7 `update_friends_status` | – | 10 s after each answer to 5 or 7 |

It reads `[0xCC][action]...` (`on_packet_received`, 0x5C9780):

| Action | Body |
|---|---|
| 5 | `u16 n`, n × (`u32 id`, `u8 len` + name, `u8 online`) (`read_friend_list`, 0x766090) |
| 6, 4 | `u16 n`, n × (`u32 id`, `u8 len` + name) (0x765FE0, 0x765F30) |
| 7 | `u16 n`, n × (`u32 id`, `u8 online`) (0x766220); an id not on the list only logs "Friend list out of sync." |
| 0..3 | `u8 result`: `'4'` (0x34) accepted, and the client asks for that list again; anything else is only logged |

Names go into a 32-byte buffer at item+0x10 with no length check (0x766119),
so at most 31 bytes. Any nonzero online byte is online; the lobby shows
"lobby" or "offline" only (`fill_friend_list`, 0x741D0C), though the Flash has
texts for "game" and "away". There are no invitations: the chat menu's
`add_friend_name`, `add_ignore_name` and `ask_join_game` have no handler in
the binary. `accept_message_from` (0x5C8E60) drops chat lines whose sender
type is 5 (a player) and whose sender id is on the ignore list.

**srp:** each account keeps its `friends` and `ignored` (in memory, with the
account; at most 200 each). Adding someone else's known account is accepted
at once (again: still once); removing is always accepted. A search finds up
to 50 other accounts whose name contains the text, ignoring ASCII case. A
friend is online while it has a client in the chat. Ignoring and friendship
are independent (the original rule is unknown).
