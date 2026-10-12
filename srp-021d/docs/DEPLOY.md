# Hosting the 0.21d server in Docker

> **Version scope: the 0.21d server** (`srp-021d`). General steps for any
> Linux host; nothing here is about a particular machine.

## The files

| File | What it does |
|---|---|
| `Dockerfile` | Builds `srp` (login, browser, lobby, chat, match server and the web page in one process) in a Rust image and copies the binary and `certs/` into a small Debian image. Runs as a user without rights (`srp`, uid 10001). The cargo caches are build cache mounts, so a rebuild compiles only what changed |
| `docker-compose.yml` | The service `srp` and, with the profile `tls`, `caddy` (HTTPS for the web page). Ports, the data volume, the settings passed to srp, and the container's limits |
| `.dockerignore` | Keeps build output, data, logs, docs and the client patches out of the build |
| `deploy/env-021d.example` | The settings to put into `.env` beside the compose file |

## What the host needs

- Linux with Docker and the compose plugin; 1 CPU and 1 GB of RAM are enough
  for a small server.
- A public IPv4 address (or a name that points at it).
- These ports open to players: **80/tcp** (the game asks the "browser
  server" there for the lobby's address), **1234/tcp** (login, TLS) and
  **1234/udp** (presence pings), **1235/tcp** (lobby and chat), **1236/udp**
  (matches); **443/tcp** if the web page is to be public.

## First start

1. Copy this folder to the host.
2. Copy `deploy/env-021d.example` to `.env` beside `docker-compose.yml` and
   fill it in. `SRP_PUBLIC_HOST` is required. Keep `.env` out of any
   repository: it holds the admin page's token and path.
3. Put the players' download (the zip made as `client-patches/README.md`
   says) into `./download`, or set `SRP_DOWNLOAD_URL`.
4. `docker compose config` (checks the file and the settings), then
   `docker compose build srp`, then `docker compose up -d srp`.
5. For HTTPS on the web page: point a domain at the host, set
   `SRP_WEB_DOMAIN` and `SRP_WEB_TRUST_PROXY=on`, then
   `docker compose --profile tls up -d`. Caddy gets the certificate by
   itself on port 443.
6. Check: `docker compose ps` shows the container healthy (the check opens
   the lobby port), and `docker compose logs srp | head` shows the servers
   started.

Players start the game with `-client=<SRP_PUBLIC_HOST>:1234`
(`client-patches/README.md`).

## Settings

The compose file passes these to srp; every other `SRP_*` setting
(`docs/SRP-021D.md`) keeps its default unless its name is added to the
`environment:` list.

| Setting | What |
|---|---|
| `SRP_PUBLIC_HOST` | The address players reach the host at. Required |
| `SRP_ADMIN_TOKEN`, `SRP_ADMIN_PATH` | The admin page (balances, bans) is on only when both are set; it is served at `/<SRP_ADMIN_PATH>`. Make both long and random |
| `SRP_ADMIN_IPS` | The addresses that may open the admin page; everyone else gets a plain 404. Behind caddy set `SRP_WEB_TRUST_PROXY=on` and list your own public address |
| `SRP_WEB_DOMAIN` | The web page's public name, for caddy and for the redirect from port 80 |
| `SRP_DOWNLOAD_FILE`, `SRP_DOWNLOAD_URL`, `SRP_DOWNLOAD_SHA256` | The download the home page offers: a file from `./download` (handed out by srp, 5 at a time), or a https link; and its checksum |
| `SRP_GAME_URL` | Where the original game is had (the page's first step) |
| `SRP_OTHER_SERVER_URL` | A link, in every page's header, to the page of a server for another game version |
| `SRP_CPUS` | CPUs the container may use (default 2) |
| `RUST_LOG` | The log level (default `info`) |

## Data, logs, backups

- The volume `srp-data` holds `srp.db` (SQLite: accounts, profiles,
  inventory, sessions), `backups/` (a copy of the database at every start)
  and `snapshots/` (worlds of clients that fell out of step, kept to
  compare).
- `docker compose logs srp` is the server's log (json-file, 5 files of
  20 MB). A new container starts a new log: save the old one before an
  update if you want to keep it.
- Nothing copies the database off the host. Do that yourself.

## Updating

1. Run the tests (`cargo test --workspace`).
2. Copy the changed source to the host and `docker compose build srp`. The
   running server is not touched by the build, but on a host with one CPU a
   build slows a running match.
3. Restart when no match runs: `docker compose up -d srp`. The log has a
   `Match 0x.. formed` line for every match and `Match 0x.. is over` when it
   ends.

A restart ends every match and drops every lobby connection. Clients connect
again by themselves and their sessions are kept in `srp.db`; the queue and
squads are lost.

## What the container is allowed

Read-only root file system, every capability dropped, no new privileges,
512 MB of memory, 512 processes. The web page inside is plain HTTP and is
published on the host's `127.0.0.1:8088` only: reach it through caddy, never
publish that port (it carries passwords).
