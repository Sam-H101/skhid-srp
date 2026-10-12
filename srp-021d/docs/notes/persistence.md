# Accounts: persistence, sign-in and passwords (2026-10-10)

srp keeps every account in an SQLite database, and checks a password at
every sign-in. The code is in `crates/session/src/persist/` (repository and
schema), `crates/session/src/password.rs` (hashes) and `SessionStore` in
`crates/session/src/lib.rs` (sign-in, saving).

Standing rule of the user: **no real passwords** anywhere, in tests, logs or
notes. Tests use obviously fake values such as `test-password-1`; srp logs
neither passwords nor their hashes.

## Settings

| Variable | Default | Meaning |
|---|---|---|
| `SRP_DATA_DIR` | `./data` | Where `srp.db` and `backups/` live; made if missing. |
| `SRP_DB` | on | `off` (or empty) keeps accounts in memory only, as before. Passwords are still checked within the run. |

At each start, before the schema is touched, an existing `srp.db` is copied
to `<dir>/backups/srp-<unix seconds>.db` with `VACUUM INTO` (so the copy
includes what the write-ahead log held). The newest 10 are kept. Restore one
by stopping srp and copying it over `srp.db` (delete `srp.db-wal` and
`srp.db-shm` too).

The log (`logs/srp.log`) says `Loaded N accounts from <path>` at start,
`Saved M accounts` after a match and on exit, and `Registered new account
<user name>` for each new account.

## Sign-in, registration, identity

- **User name.** Whatever the client sends in its account field (0.21d's
  `m_account_email`, at most 99 bytes). It need not be an email; an email is
  a user name with an `@`. It is trimmed and compared in lower case:
  `Stepan`, `stepan` and ` STEPAN ` are one account, from any IP. An empty or
  over-long name is refused.
- **Registration.** 0.21d has no sign-up screen (the client never sends
  `sign_up`). The first sign-in of a user name srp doesn't know makes the
  account, with the password given.
- **Display name.** The part of the user name before `@`, or all of it, in
  the casing it was registered with, cut to 63 bytes; `player_<id>` if that
  is empty.
- **Account ids** are handed out in order from 1 (`meta.next_account_id`).
- **Passwords.** Only an argon2id hash is kept, as a PHC string
  (`$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`, the `argon2` crate's
  defaults, random salt from the OS). The client sends the password in the
  clear inside TLS (`[u8 length][bytes]`, at most 47); the bytes are hashed as
  they come. Checking takes a few tens of milliseconds and happens with no
  lock held.
- **Lockout.** Five wrong passwords in a row for one user name make it wait
  60 s from the last one; a right password resets the count. In memory only.
- **Refusal.** A wrong password, a lockout, or a bad user name is answered
  with `0x0A invalid_user_name_or_password`, and no session is made. A bad
  user name is refused right after the sign-in message (the byte that would
  otherwise be `0x0B`); a wrong password after it came over TLS, in place of
  the `0x08` reply. The client goes back to the login screen with its
  "Invalid user name or password" text (string in `survarium.exe`; the screen
  itself was not checked against a live client).
- **Sign-out** checks no password: the session id, which only the sign-in was
  told, proves the client.
- **Session ids** start from the clock (milliseconds), and past every
  restored session (see "Sessions" below), so a client holding a session
  srp no longer keeps is refused rather than handed someone else's.

### Resetting a password

Clear the hash; the next sign-in of that user name sets the password it
gives:

```
sqlite3 data/srp.db "update accounts set password_hash = NULL where login = 'x@y'"
```

(`login` is the user name in lower case.) Do it with srp stopped, or restart
srp afterwards: a running srp keeps its own copy of the account and would
write the old hash back on the account's next save.

## What is kept

| Kept | Table / column |
|---|---|
| id, user name, password hash, display name | `accounts.id`, `login`, `password_hash`, `name` |
| the game's copy of the name | `accounts.game_name` |
| silver, gold, premium end (Unix s) | `accounts.generic_money`, `premium_money`, `premium_until` |
| rating and rating before the last match | `accounts.rating`, `rating_before` |
| next item instance id, next quest instance id | `accounts.next_item_id`, `next_quest_instance` |
| the active profile | `accounts.active_profile` (NULL: none chosen) |
| loyalty to factions 1..4 | `loyalty(faction, percent)` |
| reputations, in order | `reputations(seq, faction, points)` |
| inventory, in order, with modifications | `inventory(item_id, seq, dict_id, condition_or_stack, amount, modifications)`; modifications comma-separated |
| profiles, in order | `profiles(id, seq, name, autobuy, level, exp_total, exp_level_start, exp_next_level, revision)` |
| profile slots | `profile_slots(profile_id, slot, item_id, condition_or_stack, amount)`; the item's kind and modifications come from its inventory row |
| skills and perks, in order | `profile_skills(profile_id, seq, skill_id, points)`, `profile_perks(profile_id, seq, perk)` |
| quests, active and proposed | `quests(instance_id, seq, quest_id, games, expires_at, active)` |
| quest cooldowns | `quest_cooldowns(seq, quest_id, until)` |
| friends, ignored | `friends(seq, friend_id)`, `ignored(seq, ignored_id)` (not foreign keys: the other account may be saved later) |
| notices not yet delivered | `notices(seq, kind, text)` |
| sessions that have not ended | `sessions(session_id, account_id, last_seen)`; `last_seen` in Unix seconds (schema version 2) |

Every table but `meta` and `accounts` has `account_id`, a foreign key to
`accounts(id)` with `ON DELETE CASCADE`. Integers are `INTEGER`.

**Not kept**, on purpose: a session's connections, the matchmaking queue and
formed matches (a match in progress ends with the process), match reports
and results not yet shown, squads, a Play waiting for its squad
(`Account::play`), and the wrong-password counts. They end with the
process, as they would when a client restarts.

A compile-time guard keeps this list complete: `AccountRecord::of` and the
save (`write_account`) take each struct (session `Account`, game `Account`,
`Profile`, `Item`, `QuestInstance`) apart field by field with no `..`, so a
new field doesn't compile until it is saved or named as not kept. The load
builds each struct with every field named too. The round-trip tests compare
whole structs.

## Saving

- Whatever changes an account calls `SessionStore::mark_dirty(id)`: every
  lobby message (`Connection::answer`, which covers buying, selling, moves
  and the ammo dialog, autobuy, the active profile, skills, rerolls, quest
  changes and premium), the lobby sign-in (quest refresh), delivered notices,
  friend and ignore changes in the chat, account creation and a password set
  at sign-in, and each rewarded player of a match.
- The saver thread (`SessionStore::spawn_saver`, started by `srp`) wakes
  every 2 s, or at once after an account is made. For each marked account it
  copies the account out under its own lock (one at a time, never two),
  compares it with what it last wrote, and if it changed writes it in one
  transaction (the account row upserted, its child rows deleted and
  inserted). A write that fails is logged and retried on the next pass.
- `SessionStore::flush()` does the same at once: after a match's rewards, and
  on Ctrl+C or the console closing (the `ctrlc` handler in `srp`). Killing
  srp otherwise loses at most the last 2 s.
- On loading, each account's next item and quest instance ids are moved past
  the ids in use, `next_account_id` past every account, and a slot whose item
  is no longer in the inventory is emptied; a repaired account is written
  back.

## Sessions

A session that has not ended survives a restart, so a client in the lobby
reconnects to the restarted srp with its old session id instead of being
sent back to the login screen. A row only maps a session id to an account:
it holds no secret beyond the id the client already shows.

- **Written** when the session is made (sign-in): `create_session` puts its
  id in the saver's set of new sessions and wakes the saver, as account
  creation does. The saver writes sessions after the accounts in the same
  pass (a row needs its account's), copying the sessions out first so none
  is written with the sessions map locked.
- **Last sighting.** Pings and connections write nothing. On each pass the
  saver writes a kept session's `last_seen` again when at least
  `SESSION_SAVE_INTERVAL` (60 s) passed since it was last written and the
  client was seen since; a session held by a lobby or chat connection counts
  as seen at the pass.
- **Deleted** when it ends: sign-out (`end_session`) and the idle collector
  (`end_idle_sessions`, 600 s, `SESSION_IDLE_LIMIT`) put its id in the set
  of ended sessions; the saver deletes the row on its next pass, after that
  pass's writes.
- **Restored** at startup, after the accounts (`Restored N sessions` in the
  log), unless its `last_seen` is more than `SESSION_RESTORE_LIMIT` (1 hour,
  wall clock) ago or its account is gone; those rows are deleted. A restored
  session counts as seen at startup, with no connection, so the downtime
  doesn't count against it: its client has the whole idle window to come
  back, and it ends after that like any other. The next session id is the
  larger of the clock-based first id and the highest restored id + 1.
- With `SRP_DB=off`, sessions are in memory only, as before.

The constants are in `crates/session/src/lib.rs`: `SAVE_INTERVAL` (2 s),
`SESSION_SAVE_INTERVAL` (60 s), `SESSION_IDLE_LIMIT` (600 s),
`SESSION_RESTORE_LIMIT` (3600 s).

## Schema versions and migrations

| Version | Step |
|---|---|
| 1 | accounts and everything they own |
| 2 | `sessions(session_id, account_id, last_seen)` |

`PRAGMA user_version` is the number of steps of `MIGRATIONS` (in
`crates/session/src/persist/sqlite.rs`) applied. At open, missing steps run
in order, each in its own transaction that also sets the version. A database
with a version higher than the code knows is refused with an error naming
both; run a newer srp or restore a backup.

To change the schema: append a step to `MIGRATIONS` (never edit a step that
has shipped), adjust `load_children`/`write_account`, and keep the test
`an_older_database_loads_and_its_counters_are_repaired`, which builds a
version 1 file with rows written by hand, passing (and
`a_version_1_database_gains_the_sessions_and_keeps_its_accounts`, which
upgrades it to version 2). Foreign keys are on
(`PRAGMA foreign_keys = ON`); WAL mode, `synchronous = NORMAL`.

Postgres can replace SQLite behind the `AccountRepository` trait
(`load_all`, `save_account`, `next_account_id`, `load_sessions`,
`save_session`, `delete_session`, `describe`);
`MemoryRepository` is the in-memory one the tests use.

## Inspecting

```
sqlite3 data/srp.db ".tables"
sqlite3 data/srp.db "select id, login, name, generic_money, premium_money, rating from accounts"
sqlite3 data/srp.db "select p.id, p.name, p.autobuy, p.level from profiles p join accounts a on a.id = p.account_id where a.login = 'stepan'"
sqlite3 data/srp.db "pragma user_version"
sqlite3 data/srp.db "select printf('%x', session_id), account_id, datetime(last_seen, 'unixepoch') from sessions"
```

Reading while srp runs is safe (WAL). Edit only with srp stopped: srp
writes its in-memory copy of an account over the row on its next save.
