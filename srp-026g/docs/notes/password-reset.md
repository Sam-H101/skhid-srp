# Password reset by the admin

The rules the code and the tests follow. Code: `SessionStore::reset_password`
(`crates/session`), `servers/web-server/src/password_reset.rs`. Server-only:
no exe or content change, `REVISION` unchanged, no `simulation_changes` row.

- **Why:** a player who forgot their password asks the owner. Until
  2026-10-18 that was srp stopped, `srp.db` backed up, `UPDATE accounts SET
  password_hash = NULL WHERE login = '<login>'` by hand, srp started again.
  srp keeps the accounts in memory, so an edit of the database while it runs
  is written over; the reset is done in srp instead, while it runs.
- **Rule (owner's decision, 2026-10-18):** the reset clears the password
  (`password_hash` NULL). The player's next sign-in, with any password, sets
  it as the new one (`adopt_password`); from then on only that one is taken.
  Whoever signs in first takes the account, so reset when the player is
  ready to sign in. If the player finds it taken, reset again.
- **Effect at once:** written to `srp.db` at once (not at the saver's next
  pass); the account's sessions end, as a ban ends them (the 0.26g lobby
  signs the game out within a second), so nobody signed in with the old
  password plays on; its wrong-password lockout is forgotten. Sessions are
  the only sign-in tokens srp has (a restored session row goes with it). A
  match the account is in plays on; its result is still paid.
- **Who:** the admin only: the admin page's token, path and address list
  (`SRP_ADMIN_TOKEN`, `SRP_ADMIN_PATH`, `SRP_ADMIN_IPS`), the same as every
  admin action. The token travels in the form, never in a cookie, so there
  is nothing to forge across sites. Wrong tokens count towards the same
  lockout (5 in a row: a minute).
- **Match:** by the user name the player signs in with (`accounts.login`),
  trimmed, in any case, like the table's `COLLATE NOCASE`. Never by the
  character name.
- **The page (tab Password):** type the user name, Find; the page shows the
  account it matched (user name, character name, account id in decimal and
  hex as the logs write it, last sign-in, last address, whether a game is
  signed in now) and a "Reset the password of <login>" button. That button
  resets only if the user name still means the account shown. Answers: the
  reset done (sessions ended), no account with that user name, or the
  account changed. The last sign-in is kept in memory only (no schema
  change): after a restart it reads "not since srp started" until the next
  sign-in.
- **A script (one POST, no confirm step):** `tab=reset_password` and
  `login=<user name>` with the token, as release.sh sends its maintenance.
  One line of text back:

  | Answer | HTTP |
  |---|---|
  | `ok <login> <account id>` | 200 |
  | `no such login` | 404 |
  | `no login` | 400 |
  | `wrong token` (none or wrong) | 403 |
  | `locked: too many wrong tokens` | 429 |
  | `error: …` (nothing changed) | 500 |

  An address not in `SRP_ADMIN_IPS`, or a page without a token, gets the
  plain "not found" page (404), as for every admin request.
- **Log:** `Admin from <address>: reset the password of <login> (account
  0x…); N session(s) ended; its next sign-in sets the new one`. No password,
  hash or token is written anywhere.

## From the owner's machine, through the jump host

As release.sh asks the admin page (`admin_request`): the path and token come
from the live `.env`, curl runs on the jump host (its address is in the live
`SRP_ADMIN_IPS`, and `SRP_WEB_TRUST_PROXY` is on) and reads its URL and form
from a config on stdin, so neither the token nor the secret path is in a
command line or printed. Run in bash in the main checkout:

```bash
. private/release/release.env    # SKHID_JUMP, SKHID_LIVE, SKHID_LIVE_DIR, SKHID_SITE
login='player@example.org'       # the user name the player signs in with (no " or \)
admin=$(ssh "$SKHID_JUMP" "ssh $SKHID_LIVE \"grep -E '^SRP_ADMIN_(PATH|TOKEN)=' $SKHID_LIVE_DIR/.env\"")
path=$(sed -n 's/^SRP_ADMIN_PATH=//p' <<<"$admin")
token=$(sed -n 's/^SRP_ADMIN_TOKEN=//p' <<<"$admin")
printf 'url = "%s/%s"\ndata-urlencode = "token=%s"\ndata = "tab=reset_password"\ndata-urlencode = "login=%s"\n' \
    "${SKHID_SITE%/}" "${path#/}" "$token" "$login" |
    ssh "$SKHID_JUMP" "curl -sS --max-time 30 -w '%{http_code}\n' -K -"
unset admin path token
```

It prints the answer and then the HTTP status, e.g. `ok player@example.org
42` and `200`. (If the `.env` quotes its values, take the quotes off, as
release.sh's `read_admin` does.) Tell the player to sign in now with the
password they want.
