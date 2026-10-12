# What never goes into this repository

This repository is public. The servers it holds are hosted for real players,
and the research behind them was done from live logs. So the rule for
everything that is kept here (pages, notes, code comments, test data, commit
messages):

- **No player's user name.** A player is written by label: "player A",
  "player B (Poland)". The same label means the same player on every page.
  The key is kept privately by the operator and is not here. A country (from
  the connection's address) may stand beside a label; a provider's name, an
  address or anything else that singles a person out may not.
- **No IP address** of a player, of an operator or of a host. Hosts are named
  by their public names (`skhid.link`, `026g.skhid.link`). Test data uses
  documentation addresses (`203.0.113.x`, `198.51.100.x`, `100.64.0.x`).
  One address is kept on purpose: `188.93.23.27`, the original game's own
  server, which is written in the game's exe.
- **No secret.** The admin page's token and path, and every other setting of
  that kind, live only in a host's `.env`, which is never committed. The
  example files (`deploy/env-*.example`) have the names without values.
  Passwords and their digests are never logged by the servers and never
  pasted from a client's config.
- **No part of the game.** No game binary, patched or not, no `resources.db`,
  no dump of the disassembly, no PDB. The patches are published as tools that
  a player runs on their own copy of the original exe. The small game data
  files under `resources/binary-config-examples/` are what the code needs to
  build.
- **No database, no logs, no saved worlds** of a live server.

Before a change is committed, search it for the account names of the hosted
servers and for IPv4 addresses.

The login server's certificate and key in each `certs/` folder are not a
secret: the game pins that certificate, so every server for these clients
must use the same key, and it has been public in the SRP project from the
start.
