# The test server (test.skhid.link)

A second 0.26g server on a host of its own, for trying `main` before the
players' server gets it. It is updated from GitHub by hand: Actions ->
"Deploy the test server" -> Run workflow, and then the approval of a
maintainer. Every page of its web site says that it is the test server
(`SRP_TEST_SERVER=on`), and its accounts are its own: nothing of the players'
server is on that host.

Players start the game against it with `-server=test.skhid.link:1234`.

## How an update gets there

```
Run workflow (anyone with write access)
  -> the job waits in the `test-server` environment for a maintainer
  -> the runner on the test host starts: sudo /usr/local/sbin/srp-test-deploy
  -> the script fetches `main` itself, builds the image, starts it
```

The job gives the script nothing: no branch, no commit, no file. Whoever
starts it, and from whatever workflow, what is deployed is the `main` of this
repository at that moment.

## Why it is made this way

The repository is public and the runner is our own machine. Anyone who can
push a branch can write a workflow that runs on that runner, without any
approval; the approval of the `test-server` environment guards only the job
that names the environment. So the host itself is what limits a job:

- the runner runs as a user of its own (`ghrunner`), with no password, in no
  group, without Docker;
- that user may run one command as root, with no arguments:
  `srp-test-deploy` (this folder);
- the script reads nothing from its caller. It builds from a checkout of
  `main` that only root writes, and starts the containers with a compose
  file and a `.env` that only root writes (`/opt/srp-test`, mode 700). What
  a container may reach on the host is therefore not decided by anything in
  the repository: a change of `docker-compose.yml` reaches the test server
  only when an operator copies it there;
- the build runs the repository's `Dockerfile` inside Docker's builder, which
  has the 0.26g folder and nothing of the host.

What remains: a workflow pushed to a branch can run commands on the test host
as `ghrunner` (the network, the processor, files anyone may read there). It
cannot reach Docker, the server's data or settings, or root. And `main`
itself takes changes only through pull requests.

## Setting it up (once, as root on the host)

1. Docker with the compose plugin, `git`, `curl`.
2. `/opt/srp-test` (mode 700, root's): a copy of `srp-026g/docker-compose.yml`,
   a `download/` folder (the test ZIP, its blobs, and the server's game data
   of that ZIP, `server-game-data.db`: srp does not start without it,
   `DEPLOY.md`, "The game's files"), and a `.env` (mode 600) made from
   `deploy/env-026g.example` with at least:
   `SRP_PUBLIC_HOST` and `SRP_WEB_DOMAIN` (the test server's name),
   `SRP26_LOGIN_PORT=1234`, `SRP26_LOBBY_PORT=1235`, `SRP26_MATCH_PORT=1236`,
   `SRP_TEST_SERVER=on`, `SRP_OTHER_SERVER_URL` (the players' server's page),
   `SRP_WEB_TRUST_PROXY=on`, and an admin token, path and address list of its
   own (never the players' server's).
   The `download/` folder is the one thing in there the server reads, and
   the server in the container is not root: give the folder mode 755 and a
   ZIP in it mode 644. A folder only root can read gives no error at the
   start, only the log line `SRP_DOWNLOAD_FILE ... is not a file; /download
   is off`, a page with no download and `404` at `/api/client-files`. The
   ZIP is `Survarium-0.26g-srp-files.zip`, made for this server's own
   address (`make_client_026g.py --pack-zip --server=<its name>:1234
   --resources-db=<final resources.db> --resources-out=<blobs>`, the blobs into
   `download/resources/`, mode 644, before the ZIP): the
   launcher and the updater in it ask the site they were made for, so the
   players' ZIP is of no use here. A new ZIP put in its place is seen
   without a restart.
3. `srp-test-deploy` to `/usr/local/sbin/`, owner root, mode 755. Run it once
   as root: it clones the repository to `/opt/srp-test/src`, builds and
   starts the server (compose project `srptest`, with the `tls` profile).
4. The runner: `useradd --create-home ghrunner` and `passwd -l ghrunner`;
   as that user, unpack GitHub's runner (check the archive's sha256 against
   the release page) and register it for this repository with the label
   `srp-test`; install it as a service that runs as `ghrunner`
   (`./svc.sh install ghrunner`).
5. `/etc/sudoers.d/ghrunner`, mode 440, one line:

   ```
   ghrunner ALL=(root) NOPASSWD: /usr/local/sbin/srp-test-deploy ""
   ```

   (the `""` allows the command with no arguments only).

On GitHub (repository settings): the environment `test-server` with the
maintainers as required reviewers and `main` as its only branch; workflows of
pull requests from forks need approval for all outside contributors; `main`
takes pull requests only.

## Keeping it

- `docker-compose.yml` and `srp-test-deploy` on the host are copies. After a
  change of either in the repository, an operator copies the new one there.
- The data (accounts, logs, the worlds kept to compare) is in the Docker
  volume `srptest_srp26-data`. To start from nothing: stop the project and
  remove that volume.
- The server restarts whenever a deploy brings a new image, with no regard
  for a match that runs there.
- A deploy says it is done only when the container is healthy and the login
  port answers inside it. The health check of the compose file is the web
  page's port alone, and the login server can fail while the page answers:
  on the first evening its certificate was readable by root only (the
  checkout had been made under a tight umask, and the image keeps the
  files' modes), and two deploys called that healthy. The script now sets
  its own umask and makes the checkout readable before it builds.
