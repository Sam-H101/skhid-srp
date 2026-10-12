# Server notices in the chat

Besides players' lines, the client reads notices from servers on its chat
("messaging") connection. A notice is an ordinary `messaging_server_message`
(sender type, sender id, sender name, channel, text). The client tells it apart
by the **sender type** (`network_client`, 0x5C96D9), then searches the text for
markers with `wcsstr`. The text is converted with `mbstowcs`, so markers are
plain ASCII.

## Stats server (sender type 8): `lobby_menu::on_stats_message_arrived` (0x746480)

| Marker | Effect |
|---|---|
| `Player [ <name> ]` | Only if `<name>` (up to the next ` ]`) is this account's name: the whole text goes to the chat, then `#e` is read. Another player's notice is dropped. |
| `#e:[<n>]` | Read only after a matching `Player [`. `_wtoi(n)` is kept at `lobby_menu+0x110` and shown as the gain on the level bar (`experience_delta` in `fill_character_data`, `CharWindow.setData`). The client then asks for money (`q_account_money`) and experience (`q_player_skills`) again. |
| `#pc:[<n>]` | Only without `Player [`: the text up to `]` (8 characters at most) goes to `root.set_games_online`, the lobby status bar's online counter (`StatusPanel.games`, `panel_games_block.tx_online`). Nothing goes to the chat. |
| `#mc:[` | Searched for, never used in this build. |

A stats notice with none of these is added to the chat as it is. The code
path for a matching `Player [` adds the text to the chat twice: once at
0x74657F, then, after reading `#e` and asking for money and experience again,
it falls into the shared tail that adds any notice (0x74661E..0x74663C). So a
reward shows twice in the chat; seen live 2026-10-10 with srp sending it once.
No text the server can send avoids it and keeps `#e`.

srp sends `#pc:[players] #mc:[matches]`, and after a match
`Player [ <name> ] match reward: +N experience, +M money[; level L: ...]. #e:[N]`
on the system channel (2), in English: the client turns chat bytes into text
with the system code page (`setlocale(LC_CTYPE, "")`), so only ASCII is sure
to read right. See `rewards.md`.

## Matchmaker (sender type 7): `lobby_menu::on_match_message_arrived` (0x7410E0)

| Marker | Effect |
|---|---|
| `#+p:[ <name> ]` with `#t:[<n>]` | `root.add_player({name, icon}, n)`: n is the team (0 is team A). `#t` must be there: the client reads it without checking. |
| `#-p:[ <name> ]` | `root.remove_player(name)` |
| `#q:[<n>]` | `root.set_place(n)`: the place on the queue screen ("Ваше место в очереди"). |

The queue screen is `flash_movies/match_making.swf` (`LobbyGameSearch`). Its
player lists only work after `set_players` gives them `dataA` and `dataB`, and
the client never calls `set_players`. So in this build `add_player` fails in
Flash, and srp sends only `#q`.

## The queue screen

`lobby_menu::on_client_status_received` acts on the status in a
`client_state` answer:

| Status | Client |
|---|---|
| 0 | the lobby; hides the queue screen |
| 1 | asks again in 1 s |
| 2 | shows the queue screen ("Создание матча!", a seconds counter), asks again in 1 s |
| 3 | nothing |

Its "leave queue" button sends `discard_playing_order` (0x27). A
`connect_to_match_server` at any time takes the client to the match.
