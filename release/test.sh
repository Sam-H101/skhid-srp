#!/usr/bin/env bash
# The release scripts against stand-ins, on this machine only: ssh runs the
# command here (so the jump host, the live host and the patches servers are
# folders under a temporary one), caddy and systemctl only log, docker plays
# srp26 (a container that stops, starts and is made anew from an image ID,
# its volume a folder holding srp.db; a start writes to srp.db, makes the
# site's /patches that of the server data in place and ends the maintenance,
# or refuses, as srp does when the data does not suit it), and small HTTP
# servers stand for the patches servers and the site with its admin page.
# Nothing is sent anywhere. Every confirmation is answered by a pipe (setsid:
# no terminal to ask). ssh, curl and docker log their arguments, to check
# that no secret is in a command line.
#
#   nix shell nixpkgs#python3 nixpkgs#shellcheck -c release/test.sh
#
# Needs python3 (the HTTP servers, a ZIP) and the release srp-game-data
# (built if missing); shellcheck runs if there is one.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
command -v python3 >/dev/null || { echo "test.sh: python3 is needed" >&2; exit 1; }

if command -v shellcheck >/dev/null; then
    (cd "$here" && shellcheck -x -s bash lib.sh build-patch upload-patch add-mirror build-server-db build-zip deploy rollback setup-patches-server release.sh test.sh &&
        shellcheck -s sh host-rollback.sh)
    echo "ok   shellcheck"
else
    echo "skip shellcheck (none here)"
fi

bin=${SRP_GAME_DATA_BIN:-$root/srp-026g/target/release/srp-game-data}
if [[ ! -x $bin ]]; then
    (cd "$root" && nix develop ./srp-026g -c bash -c \
        'cd srp-026g && cargo build --quiet --release -p resources-db --bin srp-game-data')
fi
export SRP_GAME_DATA_BIN=$bin

T=$(mktemp -d)
pids=()
cleanup() {
    local pid
    for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
    rm -rf "$T"
}
trap cleanup EXIT
mkdir -p "$T/bin" "$T/patches" "$T/patches-test" "$T/site/api" "$T/live/download" "$T/work" "$T/volume" "$T/docker/tags"
# The scripts' cache (the URLs checked, the stock db's sha256) is the test's.
export XDG_CACHE_HOME=$T/cache
# The owner's go (right after the preflight, before the maintenance) is
# given beforehand; the tests of the go below wait for it.
export SKHID_RELEASE_GO=1

# The stand-ins.
cat >"$T/bin/ssh" <<'EOF'
#!/usr/bin/env bash
# ssh [-o <option>]... <host> <command>: runs the command here.
printf 'ssh %s\n' "$*" >>"$TEST_ARGS"
while [[ $1 == -o ]]; do shift 2; done
printf '%s\n' "$1" >>"$TEST_LOG"
if [[ -e $TEST_DIR/ssh-fails-$1 ]]; then exit 255; fi
exec bash -c "$2"
EOF
cat >"$T/bin/curl" <<EOF
#!/usr/bin/env bash
printf 'curl %s\n' "\$*" >>"\$TEST_ARGS"
exec $(command -v curl) "\$@"
EOF
for tool in caddy systemctl; do
    cat >"$T/bin/$tool" <<EOF
#!/usr/bin/env bash
printf '$tool %s\n' "\$*" >>"\$TEST_LOG"
if [[ -e \$TEST_DIR/$tool-fails ]]; then exit 1; fi
EOF
done
# docker: srp26. $TEST_DIR/docker: `latest` (the ID srp-026g:latest names),
# `running` (the container's image), `state` (running, stopped,
# restarting), `starts` (a count), tags/ (the other tags). A start of srp
# (`up`, `start`) first copies srp.db to the volume's backups/srp-<unix>.db
# (not with $TEST_DIR/no-backup), then appends `start <n> <image>` to it and
# makes the site's /patches that of the data in place (not with
# $TEST_DIR/patches-stale); with $TEST_DIR/refuse-start (once) it keeps
# restarting and logs that file's line (why), backing srp.db up first only
# when the accounts are the reason. `exec` runs in the running container
# (the volume is $TEST_DIR/volume); `run` is refused (no other container is
# ever run on a release's hosts). A build fails with $TEST_DIR/build-fails.
cat >"$T/bin/docker" <<'EOF'
#!/usr/bin/env bash
printf 'docker %s\n' "$*" >>"$TEST_LOG"
printf 'docker %s\n' "$*" >>"$TEST_ARGS"
live=$TEST_DIR/live/download
state=$TEST_DIR/docker
offered() {
    if [[ -f $live/server-game-data.db ]]; then
        "$SRP_GAME_DATA_BIN" info "$live/server-game-data.db" |
            sed -n 's/^  resources\.db\.\(patch_[0-9]*\) [0-9]* bytes sha256 \([0-9a-f]*\) urls \(.*\)$/\1 \2 \3/p'
    fi
}
backup() { # what srp does when it opens srp.db: VACUUM INTO backups/srp-<unix>.db
    local n max
    [[ -e $TEST_DIR/no-backup ]] && return 0
    mkdir -p "$TEST_DIR/volume/backups"
    n=$(date +%s)
    max=$(ls "$TEST_DIR/volume/backups" | sed -n 's/^srp-\([0-9]*\)\.db$/\1/p' | sort -n | tail -n 1)
    if ((n <= ${max:-0})); then n=$((max + 1)); fi
    cp "$TEST_DIR/volume/srp.db" "$TEST_DIR/volume/backups/srp-$n.db"
}
started() { # the container runs: what srp does at a start
    local n
    backup
    n=$(($(cat "$state/starts") + 1))
    echo "$n" >"$state/starts"
    echo running >"$state/state"
    echo "start $n $(cat "$state/running")" >>"$TEST_DIR/volume/srp.db"
    if [[ ! -e $TEST_DIR/patches-stale ]]; then
        offered | jq -Rn '[inputs | split(" ") | {key: .[0], value: {sha256: .[1], urls: .[2:]}}] | from_entries' >"$TEST_DIR/site/patches"
    fi
    rm -f "$TEST_DIR/state/maint"
}
case $1 in
compose)
    case " $* " in
    *" build "*)
        if [[ -e $TEST_DIR/build-fails ]]; then echo "build: cargo failed" >&2; exit 1; fi
        echo "sha256:$(date +%s%N | sha256sum | cut -c1-64)" >"$state/latest" ;;
    *" ps "*) echo c0ffee ;;
    *" stop "*) echo stopped >"$state/state" ;;
    *" start "*) started ;;
    *" logs "*)
        if [[ -s $state/refused ]]; then
            cat "$state/refused"
        else
            echo "INFO game data: /srv/srp/download/server-game-data.db (2 gameplay files, 1 projects; made of resources.db; $(offered | grep -c .) patch(es) offered at /patches)"
        fi ;;
    *" up "*)
        cp "$state/latest" "$state/running"
        if [[ -e $TEST_DIR/refuse-start ]]; then
            # srp exits 1 and says why; it never becomes healthy. It opened
            # srp.db (and so backed it up) only when the accounts are the
            # reason.
            if grep -q "cannot load the 0.26g accounts" "$TEST_DIR/refuse-start"; then backup; fi
            { echo "INFO srp starting"; cat "$TEST_DIR/refuse-start"; } >"$state/refused"
            rm "$TEST_DIR/refuse-start"
            echo restarting >"$state/state"
            rm -f "$TEST_DIR/state/maint"
        else
            rm -f "$state/refused"
            started
        fi ;;
    esac ;;
inspect)
    case "$*" in
    *"{{.Image}}"*)
        if [[ "$*" == *"image {{.Image}}"* ]]; then echo "image $(cat "$state/running")"; else cat "$state/running"; fi ;;
    *".Mounts"*) echo "srp26_srp26-data" ;;
    *".State.Status"*)
        case $(cat "$state/state") in
        running) line="running healthy 0" ;;
        restarting) line="restarting starting 3" ;;
        *) line="exited none 0" ;;
        esac
        if [[ "$*" == *".State.StartedAt"* ]]; then line+=" 2026-10-16T12:00:00Z"; fi
        echo "$line" ;;
    esac ;;
volume) echo "mount $TEST_DIR/volume" ;;
image)
    if [[ "$*" == *"latest {{.Id}}"* ]]; then echo "latest $(cat "$state/latest")"; fi ;;
tag)
    if [[ $3 == srp-026g:latest ]]; then echo "$2" >"$state/tags/latest"; cp "$state/tags/latest" "$state/latest"; else echo "$2" >"$state/tags/${3//[:\/]/_}"; fi ;;
rmi) rm -f "$state/tags/${2//[:\/]/_}" ;;
exec) # exec <id> <command> <path in /srv/srp/data>...: a running container only
    [[ $(cat "$state/state") == running ]] || { echo "Error response from daemon: container is not running" >&2; exit 1; }
    shift 2
    exec "${@//\/srv\/srp\/data/$TEST_DIR/volume}" ;;
run) echo "docker run: not on a release's hosts" >&2; exit 1 ;;
esac
EOF
chmod +x "$T/bin/"*
export PATH="$T/bin:$PATH" TEST_LOG=$T/log TEST_ARGS=$T/args TEST_DIR=$T
# release.sh and the site: a minute is a second.
export SKHID_TEST_SECONDS_PER_MINUTE=1
: >"$T/log"
: >"$T/args"
mkdir -p "$T/state"
echo "sha256:$(echo one | sha256sum | cut -c1-64)" >"$T/docker/latest"
cp "$T/docker/latest" "$T/docker/running"
echo running >"$T/docker/state"
echo 0 >"$T/docker/starts"
printf 'srp.db schema 1\n' >"$T/volume/srp.db"

serve() { # <folder> <port file>: a port answering from it, with "Server: Caddy"
    local port_file=$2
    python3 - "$1" "$port_file" >/dev/null 2>&1 <<'EOF' &
import functools, http.server, sys
class Handler(http.server.SimpleHTTPRequestHandler):
    server_version, sys_version = "Caddy", ""
    def log_message(self, *args): pass
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Handler, directory=sys.argv[1]))
open(sys.argv[2], "w").write(str(server.server_address[1]))
server.serve_forever()
EOF
    pids+=($!)
    local _
    for _ in $(seq 50); do [[ -s $port_file ]] && break; sleep 0.1; done
}
serve "$T/patches" "$T/patches.port"
serve "$T/patches-test" "$T/patches-test.port"
# A mirror: /files/<name> of $T/mirror; /hop/<k>/<name> redirects k times
# (on this host) before it is /files/<name>; /page/<name> is a web page
# (Google Drive's virus-scan page). Each request is a line of
# $T/mirror.log: its path and its Range header (- for none).
mkdir -p "$T/mirror"
python3 - "$T/mirror" "$T/mirror.port" "$T/mirror.log" >/dev/null 2>&1 <<'EOF' &
import http.server, os, sys
root, port_file, log = sys.argv[1:4]
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass
    def answer(self, status, body, kind="application/octet-stream", location=None):
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        if location:
            self.send_header("Location", location)
        self.end_headers()
        try:
            self.wfile.write(body)
        except OSError:
            pass
    def do_GET(self):
        open(log, "a").write(f"{self.path} {self.headers.get('Range', '-')}\n")
        parts = self.path.split("/")
        if len(parts) == 4 and parts[1] == "hop":
            hops = int(parts[2])
            target = f"/hop/{hops - 1}/{parts[3]}" if hops > 1 else f"/files/{parts[3]}"
            # As Google's: a redirect says it is a web page; only the last
            # answer is the file's.
            return self.answer(302, b"<html>Moved</html>", "text/html; charset=UTF-8", location=target)
        if len(parts) == 3 and parts[1] == "page":
            return self.answer(200, b"<!DOCTYPE html><html><title>Google Drive - Virus scan warning</title></html>",
                               "text/html; charset=utf-8")
        if len(parts) == 3 and parts[1] == "files" and os.path.isfile(os.path.join(root, parts[2])):
            body = open(os.path.join(root, parts[2]), "rb").read()
            asked = self.headers.get("Range", "")
            # Ranges only while root/../mirror.ranges is there (as Google
            # Drive answers them): 206 and Content-Range.
            if asked.startswith("bytes=") and os.path.exists(root + ".ranges"):
                first, last = (int(n) for n in asked[6:].split("-"))
                part = body[first:last + 1]
                self.send_response(206)
                self.send_header("Content-Type", "application/octet-stream")
                self.send_header("Content-Range", f"bytes {first}-{first + len(part) - 1}/{len(body)}")
                self.send_header("Content-Length", str(len(part)))
                self.end_headers()
                return self.wfile.write(part)
            return self.answer(200, body)
        return self.answer(404, b"not found\n", "text/plain")
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
open(port_file, "w").write(str(server.server_address[1]))
server.serve_forever()
EOF
pids+=($!)
for _ in $(seq 50); do [[ -s $T/mirror.port ]] && break; sleep 0.1; done
M=http://127.0.0.1:$(cat "$T/mirror.port")
# The site: its files, and as srp does, the launcher's places (the
# maintenance countdown of $T/state/maint), the admin page at
# /$(cat $T/state/admin-path) (its maintenance tab; the token of
# $T/state/admin-token; the answer holds the token, as the real page's
# does; every request of it, GET or POST, is a line of
# $T/state/admin-seen.log; a plan is a line of $T/state/admin-staged.log,
# saying whether a release folder was staged then), /api/stats saying a
# match runs for the first $T/state/busy asks, or (with $T/state/matches)
# the next line of that file for each ask (its last line once it is the only
# one), with "last_match_ends_in" of $T/state/ends-in when there is one (an
# srp from before it without), the launcher's matches with the "Time left"
# of each line of $T/state/page-left (minutes, or - for none yet),
# /api/client-files of the ZIP in place when asked.
python3 - "$T/site" "$T/site.port" "$T/state" "$T/live/download/Survarium-0.26g-srp-files.zip" >/dev/null 2>&1 <<'EOF' &
import functools, glob, hashlib, http.server, math, os, sys, time, urllib.parse
root, port_file, state, zip_path = sys.argv[1:5]
live = os.path.dirname(os.path.dirname(zip_path))
minute = float(os.environ.get("SKHID_TEST_SECONDS_PER_MINUTE", "60"))
def read(name, default=None):
    try:
        return open(os.path.join(state, name)).read().strip()
    except FileNotFoundError:
        return default
class Handler(http.server.SimpleHTTPRequestHandler):
    server_version, sys_version = "Caddy", ""
    def log_message(self, *args): pass
    def stats(self, online, matches):
        body = '{"online":%d,"registered":3,"matches":%d,"queued":0' % (online, matches)
        if read("ends-in") is not None:
            body += ',"last_match_ends_in":%d' % int(read("ends-in"))
        return self.answer(200, body + "}")
    def answer(self, status, body):
        data = body.encode()
        self.send_response(status)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def do_GET(self):
        if self.path == "/launcher/places/en":
            at = read("maint")
            if at is None:
                text = '<div class="cap">On the server now</div>'
            else:
                left = math.ceil(max(0.0, float(at) - time.time()) / minute)
                text = '<div class="cap warn">' + (f"Maintenance in {left} min" if left else "Maintenance: the server is down") + "</div>"
            rows = "".join("<tr><td class=m>Rudnya</td><td class=e>"
                           + ("&mdash;" if each == "-" else f"&le; {each} min") + "</td></tr>"
                           for each in (read("page-left") or "").split())
            return self.answer(200, "<table><tr><th class=m>Match</th><th class=e>Time left</th></tr>" + rows + "</table>" + text)
        if self.path == "/api/stats" and read("matches"):
            lines = read("matches").split("\n")
            if len(lines) > 1:
                open(os.path.join(state, "matches"), "w").write("\n".join(lines[1:]) + "\n")
            n = int(lines[0])
            return self.stats(3, n)
        if self.path == "/api/stats" and read("busy"):
            busy = int(read("busy"))
            open(os.path.join(state, "busy"), "w").write(str(busy - 1))
            if busy > 0:
                return self.stats(3, 1)
            return self.stats(0, 0)
        if self.path == "/api/stats" and read("ends-in") is not None:
            return self.stats(0, 0)
        if self.path == "/api/client-files":
            try:
                data = open(zip_path, "rb").read()
            except FileNotFoundError:
                return self.answer(404, "no client files are offered here\n")
            return self.answer(200, f"skhid-client-files 1\nsha256 {hashlib.sha256(data).hexdigest()}\nsize {len(data)}\n")
        if self.path == "/" + read("admin-path", "-"):
            open(os.path.join(state, "admin-seen.log"), "a").write("GET\n")
            return self.answer(200, "<form>the admin page</form>")
        return super().do_GET()
    def do_POST(self):
        open(os.path.join(state, "admin-seen.log"), "a").write(f"POST {self.path}\n")
        if self.path != "/" + read("admin-path", "-"):
            return self.answer(404, "Not found.")
        length = int(self.headers.get("Content-Length", 0))
        form = urllib.parse.parse_qs(self.rfile.read(length).decode())
        field = lambda name: form.get(name, [""])[0]
        log = open(os.path.join(state, "admin.log"), "a")
        token = read("admin-token")
        if field("token") != token:
            log.write("wrong token\n")
            return self.answer(200, "Wrong token.")
        page = f'<input type=hidden name=token value="{token}">'
        if field("tab") != "maint":
            return self.answer(200, page)
        if field("cancel"):
            was = read("maint") is not None
            if was:
                os.remove(os.path.join(state, "maint"))
            log.write("cancel\n")
            return self.answer(200, page + ("Maintenance cancelled." if was else "No maintenance was planned."))
        minutes = int(field("minutes"))
        open(os.path.join(state, "maint"), "w").write(str(time.time() + minutes * minute))
        log.write(f"plan {minutes}\n")
        staged = any(open(os.path.join(folder, "state")).read().strip() == "staging"
                     and os.path.isfile(os.path.join(folder, "server-game-data.db"))
                     and os.path.isfile(os.path.join(folder, "Survarium-0.26g-srp-files.zip"))
                     for folder in glob.glob(os.path.join(live, "release-rev*")) if os.path.isfile(os.path.join(folder, "state")))
        open(os.path.join(state, "admin-staged.log"), "a").write(f"plan {minutes} {'staged' if staged else 'NOT staged'}\n")
        return self.answer(200, page + f"Maintenance starts in {minutes} min, at 12:00 UTC.")
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Handler, directory=root))
open(port_file, "w").write(str(server.server_address[1]))
server.serve_forever()
EOF
pids+=($!)
for _ in $(seq 50); do [[ -s $T/site.port ]] && break; sleep 0.1; done
patches_port=$(cat "$T/patches.port")
test_patches_port=$(cat "$T/patches-test.port")
site_port=$(cat "$T/site.port")
P=http://127.0.0.1:$patches_port

# The lists start empty (their comment lines only), whatever the real ones hold.
grep "^#" "$here/patches.tsv" >"$T/patches.tsv"
grep "^#" "$here/patches-test.tsv" >"$T/patches-test.tsv"
cat >"$T/release.env" <<EOF
SKHID_JUMP=jump
SKHID_LIVE=live
SKHID_LIVE_DIR=$T/live
SKHID_SITE=http://127.0.0.1:$site_port
SKHID_PATCHES_SSH=patches
SKHID_PATCHES_DIR=$T/patches
SKHID_PATCHES_URL=$P
SKHID_PATCHES_CADDYFILE=$T/Caddyfile
SKHID_PATCH_LIST=$T/patches.tsv
SKHID_TEST_PATCH_LIST=$T/patches-test.tsv
SKHID_STOCK_DB=$T/resources.db
SKHID_RELEASE_LOCK=$T/lock
SKHID_BACKUP_DIR=$T/backups
EOF
export SKHID_RELEASE_ENV=$T/release.env

failures=0
ok() { echo "ok   $*"; }
fail() { echo "FAIL $*"; failures=$((failures + 1)); }
# expect <what> <pattern> <command...>: the command fails, saying <pattern>.
expect_refused() {
    local what=$1 pattern=$2 out
    shift 2
    if out=$(setsid -w "$@" </dev/null 2>&1); then
        fail "$what: ran ($out)"
    elif grep -q -- "$pattern" <<<"$out"; then
        ok "$what"
    else
        fail "$what: $out"
    fi
}
yes_to() { { yes || true; } | setsid -w "$@" 2>&1; }
check() { # <what> <test...>
    local what=$1
    shift
    if "$@"; then ok "$what"; else fail "$what"; fi
}
has() { grep -q -- "$2" <<<"$1"; }
quiet() { "$@" >/dev/null; }
rows() { grep -v '^#' "$1" | cut -f1 | tr '\n' ' ' | sed 's/ $//'; }
changed=0
changed_env() { # <sed expressions>: a copy of release.env changed so; prints its path
    changed=$((changed + 1))
    sed "$1" "$T/release.env" >"$T/changed-$changed.env"
    printf '%s\n' "$T/changed-$changed.env"
}

# The stock db: an archive of a dictionary and a project.
mkdir -p "$T/stock/gameplay/items" "$T/stock/projects/p/day"
echo "dictionary" >"$T/stock/gameplay/db_static_dictionaries"
echo "old item" >"$T/stock/gameplay/items/a.options"
# A project of nothing: a binary config of an empty table (srp-game-data
# keeps a project cut to its server objects, and refuses one that is not a
# config).
printf '\x18\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x03\0\0\0' >"$T/stock/projects/p/day/client_project"
"$bin" pack --dir "$T/stock" --out "$T/resources.db" >/dev/null

# build-patch
mkdir -p "$T/p3/gameplay/items"
echo "new item" >"$T/p3/gameplay/items/a.options"
echo "added" >"$T/p3/gameplay/items/b.options"
out=$(cd "$T/work" && "$here/build-patch" "$T/p3" 3 2>&1)
check "build-patch packs a folder" test -f "$T/work/resources.db.patch_3"
check "build-patch prints its sha256" has "$out" "sha256 $(sha256sum <"$T/work/resources.db.patch_3" | cut -d' ' -f1)"
expect_refused "build-patch never overwrites" "never overwritten" bash -c "cd $T/work && $here/build-patch $T/p3 3"
for bad in 0 256 03 x; do
    expect_refused "build-patch refuses revision $bad" "1 to 255" "$here/build-patch" "$T/p3" "$bad" --out "$T/x"
done
expect_refused "build-patch refuses another name" "must be called resources.db.patch_3" \
    "$here/build-patch" "$T/p3" 3 --out "$T/work/patch3.db"
check "--help is the usage" has "$("$here/build-patch" --help)" "usage: release/build-patch"

# upload-patch
patch=$T/work/resources.db.patch_3
sha3=$(sha256sum <"$patch" | cut -d' ' -f1)
out=$(echo n | setsid -w "$here/upload-patch" "$patch" 2>&1 || true)
check "upload-patch: a no uploads nothing" test ! -e "$T/patches/resources.db.patch_3.tmp" -a ! -e "$T/patches/resources.db.patch_3"
check "upload-patch shows the command" has "$out" "cat > $T/patches/resources.db.patch_3.tmp"
check "upload-patch: a no adds no row" test -z "$(rows "$T/patches.tsv")"
out=$("$here/upload-patch" --dry-run "$patch" 2>&1)
check "upload-patch --dry-run writes nothing" test -z "$(ls -A "$T/patches")"
check "upload-patch --dry-run adds no row" test -z "$(rows "$T/patches.tsv")"
out=$(yes_to "$here/upload-patch" "$patch")
check "upload-patch publishes" cmp -s "$patch" "$T/patches/resources.db.patch_3"
check "upload-patch: mode 644" test "$(stat -c %a "$T/patches/resources.db.patch_3")" = 644
check "upload-patch: no .tmp left" test ! -e "$T/patches/resources.db.patch_3.tmp"
check "upload-patch checks the URL" has "$out" "published: $P/resources.db.patch_3 (200"
check "upload-patch adds the row to the list" grep -qxF "$(printf 'resources.db.patch_3\t%s\t%s\t%s' "$sha3" "$(stat -c %s "$patch")" "$P/resources.db.patch_3")" "$T/patches.tsv"
expect_refused "upload-patch: the same file again" "with these same bytes" "$here/upload-patch" "$patch"
mkdir -p "$T/other" && echo other >"$T/p3/gameplay/items/c.options"
"$bin" pack --dir "$T/p3" --out "$T/other/resources.db.patch_3" >/dev/null
expect_refused "upload-patch never replaces a patch" "OTHER bytes" "$here/upload-patch" "$T/other/resources.db.patch_3"
check "the published patch is the first one" cmp -s "$patch" "$T/patches/resources.db.patch_3"
mkdir -p "$T/p2/gameplay" && echo two >"$T/p2/gameplay/two" && "$bin" pack --dir "$T/p2" --out "$T/other/resources.db.patch_2" >/dev/null
expect_refused "upload-patch refuses a patch below the list's last" "not above every patch" "$here/upload-patch" "$T/other/resources.db.patch_2"
cp "$patch" "$T/work/resources.db.patch_03"
expect_refused "upload-patch refuses a name the game does not mount" "must be resources.db.patch_<n>" \
    "$here/upload-patch" "$T/work/resources.db.patch_03"
head -c 100 /dev/zero >"$T/work/resources.db.patch_4"
expect_refused "upload-patch refuses what is not an archive" "not an archive" "$here/upload-patch" "$T/work/resources.db.patch_4"
# patch_12 (the menus): on the patches server already, not in the list (a
# run that stopped): upload-patch adds only the row.
mkdir -p "$T/p12/gameplay/menus" "$T/r12"
echo "menus" >"$T/p12/gameplay/menus/main"
"$bin" pack --dir "$T/p12" --out "$T/r12/resources.db.patch_12" >/dev/null
cp "$T/r12/resources.db.patch_12" "$T/patches/"
out=$(yes_to "$here/upload-patch" "$T/r12/resources.db.patch_12")
check "upload-patch: published, not listed: adds the row only" bash -c "grep -q 'published already, with these same bytes' <<<\"\$1\" && ! grep -q 'cat >' <<<\"\$1\"" _ "$out"
check "upload-patch: the list is 3, 12" test "$(rows "$T/patches.tsv")" = "resources.db.patch_3 resources.db.patch_12"
# --test: its own settings, never the production patches.
expect_refused "upload-patch --test needs the test patches server" "SKHID_TEST_PATCHES_SSH is not set" \
    "$here/upload-patch" --test "$T/r12/resources.db.patch_12"
cat >>"$T/release.env" <<EOF
SKHID_TEST_PATCHES_SSH=patches-test
SKHID_TEST_PATCHES_DIR=$T/patches-test
SKHID_TEST_PATCHES_URL=http://127.0.0.1:$test_patches_port
EOF
expect_refused "upload-patch --test refuses the production URL" "is the production patches URL" \
    env SKHID_RELEASE_ENV="$(changed_env "s|^SKHID_TEST_PATCHES_URL=.*|SKHID_TEST_PATCHES_URL=$P/|")" "$here/upload-patch" --test "$T/r12/resources.db.patch_12"
expect_refused "upload-patch --test refuses the production folder" "is the production patches folder" \
    env SKHID_RELEASE_ENV="$(changed_env "s|^SKHID_TEST_PATCHES_SSH=.*|SKHID_TEST_PATCHES_SSH=patches|; s|^SKHID_TEST_PATCHES_DIR=.*|SKHID_TEST_PATCHES_DIR=$T/patches/|")" \
    "$here/upload-patch" --test "$T/r12/resources.db.patch_12"
published_live=$(ls "$T/patches")
out=$(yes_to "$here/upload-patch" --test "$T/r12/resources.db.patch_12")
check "upload-patch --test publishes on the test patches server" cmp -s "$T/r12/resources.db.patch_12" "$T/patches-test/resources.db.patch_12"
check "upload-patch --test: its row in the test list" test "$(rows "$T/patches-test.tsv")" = "resources.db.patch_12"
check "upload-patch --test: the production list and folder as they were" \
    test "$(rows "$T/patches.tsv")|$(ls "$T/patches")" = "resources.db.patch_3 resources.db.patch_12|$published_live"
check "upload-patch --test says so" has "$out" "On the TEST patches server"

# build-server-db
sha12=$(sha256sum <"$T/r12/resources.db.patch_12" | cut -d' ' -f1)
"$bin" pack --dir "$T/p2" --out "$T/patches/resources.db.patch_9" >/dev/null
out=$(cd "$T/work" && "$here/build-server-db" --out "$T/work/sgd.db" 2>&1) || fail "build-server-db: $out"
check "build-server-db builds" test -f "$T/work/sgd.db"
check "build-server-db prints the offered list" \
    has "$out" "  patch_3 sha256 $sha3 urls $P/resources.db.patch_3"
check "build-server-db: a published file not in the list is not offered" bash -c "! grep -q patch_9 <<<\"\$1\"" _ "$out"
check "the data is made of the stock db and the listed patches" quiet "$bin" check "$T/work/sgd.db" --db "$T/resources.db" \
    --patch "$patch=$P/resources.db.patch_3" --patch "$T/r12/resources.db.patch_12=$P/resources.db.patch_12"
rm "$T/patches/resources.db.patch_9"
expect_refused "build-server-db never overwrites" "never overwritten" "$here/build-server-db" --out "$T/work/sgd.db"
expect_refused "build-server-db refuses another server's list" "is not http://127.0.0.1:$site_port/resources.db.patch_3" \
    env SKHID_RELEASE_ENV="$(changed_env "s|^SKHID_PATCHES_URL=.*|SKHID_PATCHES_URL=http://127.0.0.1:$site_port|")" "$here/build-server-db" --out "$T/work/sgd2.db"
printf 'resources.db.patch_7\t%s\t5\t%s\n' "$sha3" "$P/resources.db.patch_7" >"$T/bad.tsv"
expect_refused "build-server-db needs each listed patch at 200" "answered 404, not 200" \
    env SKHID_RELEASE_ENV="$(changed_env "s|^SKHID_PATCH_LIST=.*|SKHID_PATCH_LIST=$T/bad.tsv|")" "$here/build-server-db" --out "$T/work/sgd2.db"
printf 'resources.db.patch_3\t%s\t%s\t%s\n' "$sha12" "$(stat -c %s "$patch")" "$P/resources.db.patch_3" >"$T/bad.tsv"
expect_refused "build-server-db checks the served sha256 against the list" "the list says\|says $sha12" \
    env SKHID_RELEASE_ENV="$(changed_env "s|^SKHID_PATCH_LIST=.*|SKHID_PATCH_LIST=$T/bad.tsv|")" "$here/build-server-db" --out "$T/work/sgd2.db"
printf 'resources.db.patch_12\t%s\t1\t%s\nresources.db.patch_3\t%s\t1\t%s\n' "$sha12" "$P/resources.db.patch_12" "$sha3" "$P/resources.db.patch_3" >"$T/bad.tsv"
expect_refused "build-server-db refuses a list out of order" "not above the row before it" \
    env SKHID_RELEASE_ENV="$(changed_env "s|^SKHID_PATCH_LIST=.*|SKHID_PATCH_LIST=$T/bad.tsv|")" "$here/build-server-db" --out "$T/work/sgd2.db"
check "build-server-db: no file left by a refusal" test ! -e "$T/work/sgd2.db"
out=$("$here/build-server-db" --test --out "$T/work/sgd-test.db" 2>&1) || fail "build-server-db --test: $out"
check "build-server-db --test: the test list, the test URLs" has "$out" "  patch_12 sha256 $sha12 urls http://127.0.0.1:$test_patches_port/resources.db.patch_12"

# Mirrors: add-mirror, a row of several URLs, build-server-db of it. On a
# copy of the list (the live one stays that of sgd.db, which deploy takes).
size3=$(stat -c %s "$patch")
cp "$patch" "$T/mirror/resources.db.patch_3"
# The patch's size, other bytes: an older file under the mirror's name.
{ head -c $((size3 - 1)) "$patch"; printf '\377'; } >"$T/mirror/wrong_3"
wrong3=$(sha256sum <"$T/mirror/wrong_3" | cut -d' ' -f1)
cp "$T/patches.tsv" "$T/mirrored.tsv"
menv=$(changed_env "s|^SKHID_PATCH_LIST=.*|SKHID_PATCH_LIST=$T/mirrored.tsv|")
am=(env SKHID_RELEASE_ENV="$menv" "$here/add-mirror")
check "--help is the usage" has "$("$here/add-mirror" --help)" "usage: release/add-mirror"
expect_refused "add-mirror: a patch not in the list" "is not in $T/mirrored.tsv" "${am[@]}" resources.db.patch_5 "$M/files/x"
expect_refused "add-mirror: not a patch's name" "a patch's name is resources.db.patch_<n>" "${am[@]}" patch_3 "$M/files/x"
for bad in "ftp://127.0.0.1/x" "http://user:pw@127.0.0.1/x" "http://127.0.0.1/a b" "http://127.0.0.1/a#b" "http://127.0.0.1/" "127.0.0.1/x"; do
    expect_refused "add-mirror: a URL the updater does not take ($bad)" "is not a URL the players' updater takes" \
        "${am[@]}" resources.db.patch_3 "$bad"
done
expect_refused "add-mirror: a URL the row has" "is in the row of resources.db.patch_3 already" \
    "${am[@]}" resources.db.patch_3 "$P/resources.db.patch_3"
expect_refused "add-mirror: a web page in the place of the file" "answers with a web page (text/html; charset=utf-8)" \
    "${am[@]}" resources.db.patch_3 "$M/page/resources.db.patch_3"
expect_refused "add-mirror: other bytes" "gave sha256 $wrong3, not $sha3" "${am[@]}" resources.db.patch_3 "$M/files/wrong_3"
expect_refused "add-mirror: no such file" "answers 404, not 200" "${am[@]}" resources.db.patch_3 "$M/files/none"
expect_refused "add-mirror: six redirects, as the updater refuses them" "hop/6/resources.db.patch_3 answers" \
    "${am[@]}" resources.db.patch_3 "$M/hop/6/resources.db.patch_3"
check "add-mirror: a refusal changes no row" cmp -s "$T/patches.tsv" "$T/mirrored.tsv"
check "add-mirror: a refusal is not in the cache" bash -c "! grep -q 127.0.0.1:${M##*:}/ $T/cache/skhid-release/patch-urls.checked 2>/dev/null"
out=$("${am[@]}" resources.db.patch_3 "$M/hop/5/resources.db.patch_3" 2>&1) || fail "add-mirror: $out"
check "add-mirror: five redirects are followed, the URL added to the row" grep -qxF \
    "$(printf 'resources.db.patch_3\t%s\t%s\t%s %s' "$sha3" "$size3" "$P/resources.db.patch_3" "$M/hop/5/resources.db.patch_3")" "$T/mirrored.tsv"
check "add-mirror: every other line as it was" test "$(grep -v '^resources.db.patch_3' "$T/mirrored.tsv")" = "$(grep -v '^resources.db.patch_3' "$T/patches.tsv")"
check "add-mirror: the URL is in the cache" grep -qxF "$sha3 $size3 $M/hop/5/resources.db.patch_3" "$T/cache/skhid-release/patch-urls.checked"
out=$("${am[@]}" resources.db.patch_3 "$M/files/resources.db.patch_3" 2>&1) || fail "add-mirror: $out"
check "add-mirror: says the order" has "$out" "now has 3 URLs, tried in this order"
check "add-mirror: says to commit, and the next release" has "$out" "commit it; players get it with the next release"
: >"$T/mirror.log"
out=$(env SKHID_RELEASE_ENV="$menv" "$here/build-server-db" --out "$T/work/sgd-mirrors.db" 2>&1) || fail "build-server-db, mirrors: $out"
check "build-server-db: the data offers the mirrors in their order" has "$out" \
    "  patch_3 sha256 $sha3 urls $P/resources.db.patch_3 $M/hop/5/resources.db.patch_3 $M/files/resources.db.patch_3"
check "build-server-db: a mirror checked before is asked for its first bytes only" \
    bash -c "test -s $T/mirror.log && ! grep -v ' bytes=0-12\$' $T/mirror.log"
check "build-server-db: says so" has "$out" "mirror $M/files/resources.db.patch_3: as checked before"
check "the data is made of the list with its mirrors" quiet "$bin" check "$T/work/sgd-mirrors.db" --db "$T/resources.db" \
    --patch "$patch=$P/resources.db.patch_3 $M/hop/5/resources.db.patch_3 $M/files/resources.db.patch_3" \
    --patch "$T/r12/resources.db.patch_12=$P/resources.db.patch_12"
mv "$T/mirror/resources.db.patch_3" "$T/mirror/kept_3"
expect_refused "build-server-db: a mirror that no longer gives the patch" \
    "resources.db.patch_3: $M/hop/5/resources.db.patch_3 answers 404 now (it gave the file before)" \
    env SKHID_RELEASE_ENV="$menv" "$here/build-server-db" --out "$T/work/sgd-m2.db"
mv "$T/mirror/kept_3" "$T/mirror/resources.db.patch_3"
# The list's own rules for a row's URLs.
nine=$(for n in 1 2 3 4 5 6 7 8; do printf ' %s/files/m%s' "$M" "$n"; done)
for case in "$P/resources.db.patch_3  $M/x|one space apart" "$P/resources.db.patch_3 $M/x $M/x|is there twice" \
    "$P/resources.db.patch_3$nine|9 URLs; the players' updater takes at most 8" \
    "$P/resources.db.patch_3 ftp://127.0.0.1/x|ftp://127.0.0.1/x is not a URL the players' updater takes" \
    "$M/x $P/resources.db.patch_3|$M/x is not $P/resources.db.patch_3"; do
    printf 'resources.db.patch_3\t%s\t%s\t%s\n' "$sha3" "$size3" "${case%%|*}" >"$T/bad.tsv"
    expect_refused "the list refuses a row: ${case#*|}" "${case#*|}" \
        env SKHID_RELEASE_ENV="$(changed_env "s|^SKHID_PATCH_LIST=.*|SKHID_PATCH_LIST=$T/bad.tsv|")" "$here/build-server-db" --out "$T/work/sgd2.db"
done

# setup-patches-server
printf 'other.example {\n\timport stalker\n}\n' >"$T/Caddyfile"
cp "$T/Caddyfile" "$T/Caddyfile.before"
mv "$T/patches" "$T/patches.kept"
expect_refused "setup --check: something to do" "would do the above" "$here/setup-patches-server" --check
check "setup --check changes nothing" cmp -s "$T/Caddyfile" "$T/Caddyfile.before"
check "setup --check makes no folder" test ! -e "$T/patches"
mv "$T/patches.kept" "$T/patches"
touch "$T/caddy-fails"
out=$(yes_to "$here/setup-patches-server" || true)
check "setup: a failed caddy validate puts the Caddyfile back" cmp -s "$T/Caddyfile" "$T/Caddyfile.before"
rm "$T/caddy-fails"
touch "$T/systemctl-fails"
out=$(yes_to "$here/setup-patches-server" || true)
check "setup: a failed reload puts the Caddyfile back" cmp -s "$T/Caddyfile" "$T/Caddyfile.before"
check "setup: ... and says so" has "$out" "validate or reload failed: the backup is back"
rm "$T/systemctl-fails"
: >"$T/log"
out=$(yes_to "$here/setup-patches-server")
check "setup appends the block" test "$(grep -cxF "$P {" "$T/Caddyfile")" = 1
check "setup keeps the other site as it was" cmp -s "$T/Caddyfile.before" <(head -n 3 "$T/Caddyfile")
check "setup keeps a dated backup" bash -c "ls $T/Caddyfile.bak-*-before-skhid-patches >/dev/null"
check "setup reloads caddy" grep -q "systemctl reload caddy" "$T/log"
check "setup probes the patches" has "$out" "patches there: 2, each at 200"
out=$("$here/setup-patches-server" --check 2>&1)
check "setup --check after: nothing to do" has "$out" "nothing to do"
out=$(echo n | setsid -w "$here/setup-patches-server" 2>&1)
check "setup again: nothing to do, nothing asked" bash -c "! grep -q 'Go ahead' <<<'$out' && grep -q 'nothing to do' <<<'$out'"
expect_refused "setup --test --check: the test site is another block" "would do the above" \
    env SKHID_RELEASE_ENV="$(changed_env "s|^SKHID_PATCHES_CADDYFILE=.*||; \$aSKHID_TEST_PATCHES_CADDYFILE=$T/Caddyfile")" "$here/setup-patches-server" --test --check
cp "$T/Caddyfile" "$T/Caddyfile.good"
printf 'http://127.0.0.1:%s {\n\tfile_server\n}\n' "$patches_port" >>"$T/Caddyfile"
expect_refused "setup refuses the block twice" "2 times" "$here/setup-patches-server" --check
printf 'other.example {\n}\n:%s {\n\tfile_server\n}\n' "$patches_port" >"$T/Caddyfile"
expect_refused "setup refuses another site on the port" "another site on port" "$here/setup-patches-server" --check

# deploy
python3 - "$T/work" <<'EOF'
import sys, zipfile
at = 0x380862 - 0x11000 + 0x400
for revision in (2, 11, 12, 13, 14, 15, 16, 17, 18):
    exe = bytearray(at + 16)
    exe[at:at + 6] = bytes([0xC6, 0x44, 0x24, 0x27, revision, 0x90])
    with zipfile.ZipFile(f"{sys.argv[1]}/r{revision}.zip", "w") as z:
        z.writestr("game/binaries/x86/survarium_skhid.exe", bytes(exe))
        z.writestr("game/binaries/x86/updater_skhid.exe", b"updater")
EOF
cp "$T/work/r11.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
echo "old data" >"$T/live/download/server-game-data.db"
# The original menus' blobs the updater's restore to stock fetches.
mkdir -p "$T/live/download/resources"
for blob in 0ff118a8 ab6f22d8 805ea5f8; do echo "$blob" >"$T/live/download/resources/${blob}0000"; done
printf 'SRP_ADMIN_TOKEN=very-secret\nSRP_MIN_PATCH_REVISION=11\nOTHER=1' >"$T/live/.env"
zip12=$T/work/r12.zip
printf '{"patch_3": {"url": "%s/resources.db.patch_3", "sha256": "%s"}}' "$P" "$sha3" >"$T/site/patches"
echo '{"online":0,"registered":3,"matches":0,"queued":0}' >"$T/site/api/stats"
"$bin" build --db "$T/resources.db" --patch "$patch=$P/resources.db.patch_3" --out "$T/work/sgd-no12.db" >/dev/null
tree() { (cd "$T/live" && find . -path "./release-rev*" -prune -o -type f -exec sha256sum {} + | sort); }
folder() { find "$T/live" -maxdepth 1 -name "release-rev$1-*-$2" | head -n 1; } # <revision> <label>
before=$(tree)
db_before=$(cat "$T/volume/srp.db")
: >"$T/log"
out=$("$here/deploy" --dry-run --zip "$zip12" --server-data "$T/work/sgd.db" --revision 12 2>&1) || fail "deploy --dry-run: $out"
check "deploy --dry-run changes nothing" test "$(tree)" = "$before"
check "deploy --dry-run runs nothing there" test ! -s "$T/log"
check "deploy --dry-run shows the restart" has "$out" "docker compose up -d --no-deps --force-recreate srp26"
check "deploy --dry-run shows the release folder" has "$out" "mkdir release-rev12-[0-9]\{8\}-[0-9]\{4\} "
expect_refused "deploy refuses a ZIP of another revision" "says patch revision 12, not 13" \
    "$here/deploy" --zip "$zip12" --server-data "$T/work/sgd.db" --revision 13
expect_refused "deploy refuses data offering a later patch" "offers patch_3, above revision 2" \
    "$here/deploy" --zip "$T/work/r2.zip" --server-data "$T/work/sgd.db" --revision 2
expect_refused "deploy refuses revision 12 without patch_12 (the menus)" "does not offer patch_12 (the menus)" \
    "$here/deploy" --zip "$zip12" --server-data "$T/work/sgd-no12.db" --revision 12
expect_refused "deploy refuses data that is not the list's" "does not offer exactly the patches of" \
    "$here/deploy" --zip "$T/work/r11.zip" --server-data "$T/work/sgd-no12.db" --revision 11
mv "$T/live/download/resources/ab6f22d80000" "$T/ab6f.kept"
expect_refused "deploy refuses when an original blob is gone from download/resources" "lacks the original blobs .*lobby_menu ab6f22d8" \
    "$here/deploy" --zip "$zip12" --server-data "$T/work/sgd.db" --revision 12
mv "$T/ab6f.kept" "$T/live/download/resources/ab6f22d80000"
check "deploy: the refusals changed nothing" test "$(tree)" = "$before"
: >"$T/args"
out=$(yes_to "$here/deploy" --zip "$zip12" --server-data "$T/work/sgd.db" --revision 12 --label test) || fail "deploy: $out"
F=$(folder 12 test)
check "deploy makes the release folder release-rev12-<UTC YYYYMMDD-HHMM>-test" bash -c "[[ \${1##*/} =~ ^release-rev12-[0-9]{8}-[0-9]{4}-test$ ]]" _ "$F"
check "deploy puts the ZIP" cmp -s "$zip12" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "deploy puts the server data" cmp -s "$T/work/sgd.db" "$T/live/download/server-game-data.db"
check "deploy: the new files stay in the folder (hard links)" test "$F/Survarium-0.26g-srp-files.zip" -ef "$T/live/download/Survarium-0.26g-srp-files.zip" -a "$F/server-game-data.db" -ef "$T/live/download/server-game-data.db"
check "deploy keeps the old ZIP in the folder" cmp -s "$T/work/r11.zip" "$F/old-Survarium-0.26g-srp-files.zip"
check "deploy keeps the old data in the folder" grep -q 'old data' "$F/old-server-game-data.db"
check "deploy: old.sha and new.sha" bash -c "grep -q ' download/server-game-data.db$' $F/old.sha && grep -q \"^$(sha256sum <"$zip12" | cut -d' ' -f1)  Survarium-0.26g-srp-files.zip$\" $F/new.sha"
check "deploy keeps the image IDs" test "$(cat "$F/image.old")|$(cat "$F/image.new")" = "$(cat "$T/docker/running")|$(cat "$T/docker/running")"
check "deploy keeps srp's backup from the new start: srp.db before the release" bash -c "test -f $F/\$(cat $F/srp-db.name) && cmp -s $F/\$(cat $F/srp-db.name) <(printf '%s\n' \"\$1\")" _ "$db_before"
name=$(cat "$F/srp-db.name" 2>/dev/null || echo none)
check "deploy copies it to the jump host (folder 700, file 600)" bash -c "cmp -s $F/$name $T/backups/$name && test \$(stat -c %a $T/backups) = 700 -a \$(stat -c %a $T/backups/$name) = 600"
check "deploy: ... with its line in SHA256SUMS there" grep -q "  $name\$" "$T/backups/SHA256SUMS"
check "deploy: ... the sha256 compared on both ends" has "$out" "srp.db off the live host: jump:$T/backups/$name, sha256 [0-9a-f]* on both ends"
check "deploy runs no other container" bash -c "! grep -q '^docker run' $T/log"
check "deploy writes the rollback in the folder" test -f "$F/rollback.sh" -a -f "$F/rollback.env"
check "deploy: the folder's state is released" grep -qx released "$F/state"
check "deploy: the folder's release.log, without .env" bash -c "grep -q 'deployed revision 12' $F/release.log && ! grep -q very-secret $F/release.log"
check "deploy tags the running image by its ID" test "$(cat "$T"/docker/tags/srp-026g_before-rev12-*-test)" = "$(cat "$T/docker/running")"
check "deploy sets the revision" grep -qx "SRP_MIN_PATCH_REVISION=12" "$T/live/.env"
check "deploy keeps the rest of .env" test "$(grep -v '^SRP_MIN_PATCH_REVISION=' "$T/live/.env")" = $'SRP_ADMIN_TOKEN=very-secret\nOTHER=1'
check "deploy never prints .env" bash -c "! grep -q very-secret <<<\"\$1\"" _ "$out"
check "deploy makes srp26 anew" grep -q "docker compose up -d --no-deps --force-recreate srp26" "$T/log"
check "deploy: the ZIP goes in after the restart" bash -c "a=\$(grep -n 'up -d --no-deps' $T/args | head -n1 | cut -d: -f1); z=\$(grep -n 'mv -f download/.Survarium-0.26g-srp-files.zip.new' $T/args | head -n1 | cut -d: -f1); test -n \"\$a\" -a -n \"\$z\" && ((a < z))"
check "deploy checks /patches and /api/client-files" has "$out" "/api/client-files: the new ZIP"
check "deploy goes through the jump host" bash -c "grep -qx jump $T/log && grep -qx live $T/log"
check "deploy leaves nothing half-done" bash -c "! ls $T/live/download/.*.new $F/*.part 2>/dev/null"
check "deploy frees the lock" test ! -e "$T/lock"
sed -i '/^SRP_MIN_PATCH_REVISION=/d' "$T/live/.env"
out=$(yes_to "$here/deploy" --zip "$zip12" --server-data "$T/work/sgd.db" --revision 12 --label again) || fail "deploy again: $out"
check "deploy adds the setting when missing, on a line of its own" grep -qx "SRP_MIN_PATCH_REVISION=12" "$T/live/.env"
mkdir "$T/lock" && echo "someone@else, since then" >"$T/lock/owner"
expect_refused "deploy waits for no other release (the lock)" "holds the lock on the jump host: someone@else" \
    "$here/deploy" --zip "$T/work/r13.zip" --server-data "$T/work/sgd.db" --revision 13
check "deploy: ... and leaves the other's lock" test -f "$T/lock/owner"
rm -r "$T/lock"
# /patches that is not the data's: srp's log, then the rollback.
touch "$T/patches-stale"
echo '{}' >"$T/site/patches"
starts=$(cat "$T/docker/starts")
out=$(yes_to "$here/deploy" --zip "$T/work/r13.zip" --server-data "$T/work/sgd.db" --revision 13 --label wrong || true)
rm "$T/patches-stale"
F=$(folder 13 wrong)
check "deploy says when /patches is not the data's" has "$out" "/patches is {}, not"
check "deploy: ... shows srp's log" has "$out" "srp26's last log lines:"
check "deploy: ... rolls back" has "$out" "ROLLING BACK"
check "deploy: ... the ZIP never moved" cmp -s "$zip12" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "deploy: ... the setting back" grep -qx "SRP_MIN_PATCH_REVISION=12" "$T/live/.env"
check "deploy: ... srp.db back from the new start's backup (its writes gone)" bash -c "! grep -q '^start $((starts + 1)) ' $T/volume/srp.db && grep -q '^start $((starts + 2)) ' $T/volume/srp.db"
check "deploy: ... srp26 running again" test "$(cat "$T/docker/state")" = running
check "deploy: ... the folder says rolled-back" grep -qx rolled-back "$F/state"
check "deploy: ... nothing half-done" bash -c "! ls $T/live/download/.*.new 2>/dev/null"
out=$(echo n | setsid -w "$here/deploy" --zip "$zip12" --server-data "$T/work/sgd.db" --revision 12 2>&1 || true)
check "deploy: a no stops before any write" has "$out" "stopped: not confirmed"

# release.sh
# main: a checkout of its own (its origin a bare repository here) with the
# REVISION and the srp-026g files the image is built of; the live tree is
# main's. build-zip is a stand-in (the ZIP of main's revision); a minute is
# a second.
git_t() { git -c user.name=test -c user.email=test@example.org -c init.defaultBranch=main "$@"; }
git_t init -q --bare "$T/origin.git"
git_t clone -q "$T/origin.git" "$T/repo" 2>/dev/null
src=$T/repo/srp-026g
mkdir -p "$src/client-patches" "$src/certs" "$src/crates/a/src" "$src/resources/binary-config-examples" \
    "$src/resources/game-026g" "$src/scripts" "$src/servers/srp/src" "$src/docs"
for f in Cargo.toml Cargo.lock Dockerfile .dockerignore docker-compose.yml certs/login.crt crates/a/src/lib.rs \
    resources/binary-config-examples/a.cfg resources/game-026g/README scripts/run.sh servers/srp/src/main.rs docs/notes.md; do
    echo "$f 1" >"$src/$f"
done
cat >"$src/Dockerfile" <<'DOCKERFILE'
FROM rust:1-slim-bookworm AS build
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY servers servers
COPY scripts scripts
COPY resources/binary-config-examples resources/binary-config-examples
COPY resources/game-026g resources/game-026g
COPY resources/not-in-main resources/not-in-main
FROM debian:bookworm-slim
COPY --from=build /srp /usr/local/bin/srp
COPY certs ./certs
DOCKERFILE
commit() { # <revision> <message>
    sed -i "s/^REVISION = .*/REVISION = $1/" "$src/client-patches/patch_026g.py"
    git_t -C "$T/repo" add -A && git_t -C "$T/repo" commit -qm "$2" && git_t -C "$T/repo" push -q origin HEAD:main 2>/dev/null
}
echo "REVISION = 12" >"$src/client-patches/patch_026g.py"
commit 12 "revision 12"
git -C "$T/repo" archive HEAD:srp-026g Cargo.toml Cargo.lock Dockerfile .dockerignore docker-compose.yml certs crates \
    resources/binary-config-examples resources/game-026g scripts servers | tar -x -C "$T/live"
cat >"$T/build-zip" <<'STANDIN'
#!/usr/bin/env bash
# build-zip --ref <commit> --work <dir> [--test]: the ZIP of r<main's REVISION>.zip.
while (($#)); do
    case $1 in
    --work) work=$2; shift 2 ;;
    --ref) ref=$2; shift 2 ;;
    *) shift ;;
    esac
done
if [[ -e $TEST_DIR/build-zip-fails ]]; then echo "build-zip: the build failed" >&2; exit 1; fi
# A slow build: the seconds of $TEST_DIR/build-zip-sleep.
if [[ -s $TEST_DIR/build-zip-sleep ]]; then sleep "$(cat "$TEST_DIR/build-zip-sleep")"; fi
revision=$(git -C "$TEST_DIR/repo" show "$ref:srp-026g/client-patches/patch_026g.py" | sed -n 's/^REVISION = //p')
mkdir -p "$work"
cp "$TEST_DIR/work/r$revision.zip" "$work/Survarium-0.26g-srp-files.zip"
echo "  CHANGED  game/binaries/x86/survarium_skhid.exe"
echo "updater_skhid.exe: unchanged (the same file as live)"
STANDIN
chmod +x "$T/build-zip"
mkdir -p "$T/game" "$T/tmp"
cat >>"$T/release.env" <<ENV
SKHID_GAME=$T/game
SKHID_GAME_SERVER=127.0.0.1:1234
SKHID_LOCALIZATION=english
SKHID_MIN_FREE_MB=1
SKHID_LIVE_MIN_FREE_MB=1
ENV
export SKHID_TEST_REPO=$T/repo SKHID_TEST_BUILD_ZIP=$T/build-zip XDG_CACHE_HOME=$T/cache TMPDIR=$T/tmp
SKHID_TEST_STOCK_SHA256=$(sha256sum <"$T/resources.db" | cut -d' ' -f1)
export SKHID_TEST_STOCK_SHA256
token=tok3n-$RANDOM$RANDOM$RANDOM
admin_path=hidden-$RANDOM$RANDOM
printf '%s\n' "$token" >"$T/state/admin-token"
printf '%s\n' "$admin_path" >"$T/state/admin-path"
cp "$T/work/r11.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
# The first release with server data: there is none on the live host yet,
# nor /patches.
rm "$T/live/download/server-game-data.db" "$T/site/patches"
printf 'SRP_ADMIN_PATH=%s\nSRP_ADMIN_TOKEN="%s"\nSRP_WEB_TRUST_PROXY=on\nSRP_MIN_PATCH_REVISION=11\nSRP_DAILY_BONUS=5\nOTHER=1\n' \
    "$admin_path" "$token" >"$T/live/.env"
ln -sf "$T/live/download/Survarium-0.26g-srp-files.zip" "$T/site/download"
echo '{"online":0,"registered":3,"matches":0,"queued":0}' >"$T/site/api/stats"
touch "$T/state/admin.log"
: >"$T/outputs"
run() { # <how: yes | no | wait | quiet> release.sh <args...>: its output into $out (and $T/outputs)
    local how=$1
    shift
    : >"$T/log"
    : >"$T/args.run"
    case $how in
    yes) out=$({ yes || true; } | setsid -w "$here/release.sh" "$@" 2>&1) ;;
    no) out=$(echo n | setsid -w "$here/release.sh" "$@" 2>&1) ;;
    wait) out=$({ sleep 6; } | setsid -w "$here/release.sh" "$@" 2>&1) ;;
    quiet) out=$(setsid -w "$here/release.sh" "$@" </dev/null 2>&1) ;;
    esac
    local status=$?
    printf '%s\n' "$out" >>"$T/outputs"
    return "$status"
}
admin_log() { tr '\n' ' ' <"$T/state/admin.log" | sed 's/ $//'; }
offers() { "$bin" info "$T/live/download/server-game-data.db" 2>/dev/null | grep -q "resources.db.$1 "; }
nothing_staged() { ! ls "$T/live/download/".*.new "$T"/live/release-rev*/*.part "$T"/live/release-rev*/src >/dev/null 2>&1 && ! grep -qxE 'staging|swapping' "$T"/live/release-rev*/state; }
first_line() { grep -n -- "$1" "$T/args" | tail -n +"${2:-1}" | head -n 1 | cut -d: -f1; }

# A whole release, no new content (patch_3 and patch_12 listed), the server
# code as live.
: >"$T/args"
run yes --minutes 2 --label a || fail "release.sh: $out"
check "release: plans the maintenance from the jump host" test "$(admin_log)" = "plan 2"
check "release: sees the countdown on the launcher page" has "$out" 'the launcher says "Maintenance in [12] min"'
check "release: finds the server code as live" has "$out" "the live tree is main's"
check "release: ... and builds no image" bash -c "! grep -q ' build ' $T/log"
check "release: checks the listed patches on the patches server" has "$out" "the offered-patch list: 2 patch(es), each on the patches server"
check "release: the new ZIP is in place" cmp -s "$T/work/r12.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "release: the new server data is in place (patch_3)" offers patch_3
check "release: ... and patch_12" offers patch_12
check "release: the old ZIP is kept in its folder" bash -c "cmp -s $T/work/r11.zip $(folder 12 a)/old-Survarium-0.26g-srp-files.zip"
check "release: srp.db as it was before the release, in the folder" bash -c "F=$(folder 12 a); test -s \$F/\$(cat \$F/srp-db.name)"
check "release: ... and on the jump host" bash -c "F=$(folder 12 a); cmp -s \$F/\$(cat \$F/srp-db.name) $T/backups/\$(cat \$F/srp-db.name)"
check "release: the folder's changed.txt" grep -q "the players' ZIP" "$(folder 12 a)/changed.txt"
check "release: the folder's release.log" grep -q "released revision 12" "$(folder 12 a)/release.log"
check "release: no old data to keep (there was none)" test ! -e "$(folder 12 a)/old-server-game-data.db"
check "release: the rollback removes the data (there was none)" grep -qx "HAVE_DATA=0" "$(folder 12 a)/rollback.env"
check "release: SRP_MIN_PATCH_REVISION=12" grep -qx "SRP_MIN_PATCH_REVISION=12" "$T/live/.env"
check "release: the rest of .env as it was" test "$(grep -vc '^SRP_MIN_PATCH_REVISION=' "$T/live/.env")" = 5
check "release: restarts srp26" grep -q "docker compose up -d --no-deps --force-recreate srp26" "$T/log"
check "release: stops srp26 before it moves anything" bash -c "s=\$(grep -n 'compose stop srp26' $T/args | head -n1 | cut -d: -f1); b=\$(grep -n 'mv -f download/.server-game-data.db.new' $T/args | head -n1 | cut -d: -f1); test -n \"\$s\" -a -n \"\$b\" && ((s < b))"
check "release: the ZIP goes in after the restart" bash -c "a=\$(grep -n 'up -d --no-deps' $T/args | head -n1 | cut -d: -f1); z=\$(grep -n 'mv -f download/.Survarium-0.26g-srp-files.zip.new' $T/args | head -n1 | cut -d: -f1); test -n \"\$a\" -a -n \"\$z\" && ((a < z))"
check "release: waits for no match" has "$out" "5. no match is running"
check "release: checks /patches and /api/client-files" has "$out" "/api/client-files: the new ZIP"
check "release: checks the log's game data line" has "$out" "log: INFO game data: .* 2 patch(es) offered"
check "release: prints the rollback" has "$out" "release/rollback release-rev12-[0-9-]*-a"
check "release: the rollback puts the running image back, by its ID" grep -qx "OLD_IMAGE=sha256:[0-9a-f]*" "$(folder 12 a)/rollback.env"
check "release: leaves nothing staged" nothing_staged
check "release: the maintenance ended with the restart" test ! -e "$T/state/maint"
check "release: took the lock on the jump host" grep -q "^ssh jump mkdir $T/lock" "$T/args"
check "release: ... and freed it" test ! -e "$T/lock"

# New content and new server code.
echo "crates 2" >"$src/crates/a/src/lib.rs"
printf 'services:\n  srp26:\n    environment:\n      - SRP_DAILY_BONUS\n' >"$src/docker-compose.yml"
commit 13 "revision 13, server code"
mkdir -p "$T/p13/gameplay/items" "$T/r13"
echo "item 13" >"$T/p13/gameplay/items/d.options"
"$bin" pack --dir "$T/p13" --out "$T/r13/resources.db.patch_13" >/dev/null
: >"$T/state/admin.log"
running_before=$(cat "$T/docker/running")
run yes --resources "$T/r13/resources.db.patch_13" --minutes 2 --label b || fail "release.sh --resources: $out"
check "release --resources: publishes the patch" cmp -s "$T/r13/resources.db.patch_13" "$T/patches/resources.db.patch_13"
check "release --resources: adds its row to the list" test "$(rows "$T/patches.tsv")" = "resources.db.patch_3 resources.db.patch_12 resources.db.patch_13"
check "release --resources: the server data offers it" offers patch_13
check "release --resources: /patches offers it" grep -q patch_13 "$T/site/patches"
check "release --resources: says to commit the list" has "$out" "Commit $T/patches.tsv"
check "release: finds the changed server code" has "$out" "CHANGED   crates/a/src/lib.rs"
check "release: tags the running image by its ID" grep -q "docker tag $running_before srp-026g:before-rev13-[0-9-]*-b" "$T/log"
check "release: builds main's source, staged in the folder" grep -q "docker compose --env-file $T/live/.env -f release-rev13-[0-9-]*-b/src/docker-compose.yml build srp26" "$T/log"
check "release: runs the new image" test "$(cat "$T/docker/running")" != "$running_before"
check "release: the source is in place after" grep -qx "crates 2" "$T/live/crates/a/src/lib.rs"
check "release: the old source is kept in the folder" bash -c "tar -xzOf $(folder 13 b)/old-src.tgz crates/a/src/lib.rs | grep -qx 'crates/a/src/lib.rs 1'"
check "release: the rollback puts the image back" grep -qx "OLD_IMAGE=$running_before" "$(folder 13 b)/rollback.env"
check "release: the rollback has srp.db of before the release" test -s "$(folder 13 b)/$(cat "$(folder 13 b)/srp-db.name")"
check "release: leaves nothing staged (the source either)" nothing_staged
check "release: SRP_MIN_PATCH_REVISION=13" grep -qx "SRP_MIN_PATCH_REVISION=13" "$T/live/.env"

# Refused in the preflight, before anything.
: >"$T/state/admin.log"
expect_refused "release: a patch already listed" "is in $T/patches.tsv already: run without --resources" \
    "$here/release.sh" --resources "$T/r13/resources.db.patch_13"
expect_refused "release: main's revision not above live" "is not above the live exe's 13" "$here/release.sh"
echo "crates 3" >"$src/crates/a/src/lib.rs"
commit 14 "revision 14, server code"
mkdir -p "$T/p14/gameplay/items" "$T/r14"
echo "item 14" >"$T/p14/gameplay/items/e.options"
"$bin" pack --dir "$T/p14" --out "$T/r14/resources.db.patch_14" >/dev/null
expect_refused "release: a patch of another revision" "must be called resources.db.patch_14" \
    "$here/release.sh" --resources "$T/r13/resources.db.patch_13"
cp "$T/r14/resources.db.patch_14" "$T/patches/"
expect_refused "release: a patch published but not listed" "already published, these same bytes, but not in" \
    "$here/release.sh" --resources "$T/r14/resources.db.patch_14"
rm "$T/patches/resources.db.patch_14"
expect_refused "release: --yes needs --i-know" "goes with --i-know" "$here/release.sh" --yes
touch "$T/ssh-fails-live"
expect_refused "release: says the live host cannot be reached" "ssh to the live host (live through jump) fails" "$here/release.sh"
rm "$T/ssh-fails-live"
python3 -c 'import time; print(time.time() + 600)' >"$T/state/maint"
expect_refused "release: refuses when a maintenance is planned already" "a maintenance is planned already" "$here/release.sh"
rm "$T/state/maint"
mkdir "$T/lock" && echo "someone@else, since then" >"$T/lock/owner"
expect_refused "release: refuses while another release runs (the lock)" "holds the lock on the jump host: someone@else" "$here/release.sh"
check "release: ... and leaves the other's lock" test -f "$T/lock/owner"
rm -r "$T/lock"
cp "$T/docker/latest" "$T/latest.kept"
echo "sha256:$(echo hand | sha256sum | cut -c1-64)" >"$T/docker/latest"
expect_refused "release: refuses when srp-026g:latest is not the running image" "a restart would run an image no release made" "$here/release.sh"
cp "$T/latest.kept" "$T/docker/latest"
grep -v patch_12 "$T/patches.tsv" >"$T/no12.tsv"
expect_refused "release: revision 14 needs patch_12 (the menus) listed" "needs resources.db.patch_12 (the menus)" \
    env SKHID_RELEASE_ENV="$(changed_env "s|^SKHID_PATCH_LIST=.*|SKHID_PATCH_LIST=$T/no12.tsv|")" "$here/release.sh"
sed "s/^\(resources.db.patch_3\t\)[0-9a-f]*/\1$sha12/" "$T/patches.tsv" >"$T/wrong.tsv"
expect_refused "release: a listed patch that is not so on the patches server" "patch_3 .* is not so on the patches server" \
    env SKHID_RELEASE_ENV="$(changed_env "s|^SKHID_PATCH_LIST=.*|SKHID_PATCH_LIST=$T/wrong.tsv|")" "$here/release.sh"
cat >>"$T/release.env" <<ENV
SKHID_TEST=test-host
SKHID_TEST_DIR=$T/live
SKHID_TEST_SITE=http://127.0.0.1:$site_port
SKHID_TEST_GAME_SERVER=127.0.0.1:2234
ENV
expect_refused "release --test refuses the production patches" "is the production patches URL" \
    env SKHID_RELEASE_ENV="$(changed_env "s|^SKHID_TEST_PATCHES_URL=.*|SKHID_TEST_PATCHES_URL=$P|")" "$here/release.sh" --test --resources "$T/r14/resources.db.patch_14"
expect_refused "release --test needs its own patches server" "SKHID_TEST_PATCHES_DIR is not set" \
    env SKHID_RELEASE_ENV="$(changed_env "/^SKHID_TEST_PATCHES_DIR=/d")" "$here/release.sh" --test
check "release: the refusals planned nothing" test ! -s "$T/state/admin.log"
check "release: ... and published nothing" test ! -e "$T/patches/resources.db.patch_14" -a ! -e "$T/patches-test/resources.db.patch_14"

# The dry run.
before=$(tree)
published_before=$(ls "$T/patches")
run quiet --dry-run --resources "$T/r14/resources.db.patch_14" --minutes 2 || fail "release.sh --dry-run: $out"
check "release --dry-run: no file changes on the live host" test "$(tree)" = "$before"
check "release --dry-run: no patch is published" test "$(ls "$T/patches")" = "$published_before"
check "release --dry-run: no row is added" test "$(rows "$T/patches.tsv")" = "resources.db.patch_3 resources.db.patch_12 resources.db.patch_13"
check "release --dry-run: plans no maintenance" test ! -s "$T/state/admin.log"
check "release --dry-run: runs no docker but reads" bash -c "! grep -qE '^docker (compose .* (up|stop|start|build) |tag |run |rmi )' $T/log"
check "release --dry-run: takes no lock" test ! -e "$T/lock"
check "release --dry-run: shows the POST, not its secrets" has "$out" "POST http://127.0.0.1:$site_port/<SRP_ADMIN_PATH> token=<SRP_ADMIN_TOKEN> tab=maint minutes=2"
check "release --dry-run: shows what /patches will offer" has "$out" "  patch_14 [0-9a-f]* $P/resources.db.patch_14"
check "release --dry-run: shows the swap and the restart" has "$out" "docker compose up -d --no-deps --force-recreate srp26"
check "release --dry-run: shows the build" has "$out" "build srp26"
check "release --dry-run: says so" has "$out" "dry run: nothing was planned, uploaded or changed"

# The preflight checks every URL of every row, mirrors too: once whole,
# then (the cache, across runs) by their first bytes.
rm -f "$T/cache/skhid-release/patch-urls.checked"
: >"$T/mirror.log"
export SKHID_RELEASE_ENV
SKHID_RELEASE_ENV=$(changed_env "s|^SKHID_PATCH_LIST=.*|SKHID_PATCH_LIST=$T/mirrored.tsv|")
run quiet --dry-run --minutes 2 || fail "release.sh --dry-run, mirrors: $out"
check "release: checks every URL of the list" has "$out" \
    "the offered-patch list: 2 patch(es), each on the patches server (patches) with its sha256, and its 4 URL(s) giving it"
check "release: ... each mirror downloaded whole the first time" \
    bash -c "grep -qx '/files/resources.db.patch_3 -' $T/mirror.log && grep -qx '/hop/5/resources.db.patch_3 -' $T/mirror.log"
check "release: ... and kept in the cache" test "$(grep -c " $M/" "$T/cache/skhid-release/patch-urls.checked")" = 2
check "release: shows the mirrors /patches will offer" has "$out" \
    "  patch_3 $sha3 $P/resources.db.patch_3 $M/hop/5/resources.db.patch_3 $M/files/resources.db.patch_3"
: >"$T/mirror.log"
run quiet --dry-run --minutes 2 || fail "release.sh --dry-run, mirrors again: $out"
check "release: the next run asks the mirrors for their first bytes only" \
    bash -c "test -s $T/mirror.log && ! grep -v ' bytes=0-12\$' $T/mirror.log"
mv "$T/mirror/resources.db.patch_3" "$T/mirror/kept_3"
run quiet --dry-run --minutes 2 || true
check "release: a mirror that no longer gives the patch is a problem" has "$out" \
    "  - $T/mirrored.tsv: resources.db.patch_3: $M/files/resources.db.patch_3 answers 404 now (it gave the file before)"
# Replaced since by a file of another size that starts as a patch does:
# seen by the size the answer gives, a 200's Content-Length or a 206's
# Content-Range.
head -c $((size3 - 1)) "$T/mirror/kept_3" >"$T/mirror/resources.db.patch_3"
run quiet --dry-run --minutes 2 || true
check "release: a mirror whose file is now of another size is a problem (200)" has "$out" \
    "  - $T/mirrored.tsv: resources.db.patch_3: $M/files/resources.db.patch_3 says the file is $((size3 - 1)) bytes now, not $size3 (it gave the file before)"
touch "$T/mirror.ranges"
run quiet --dry-run --minutes 2 || true
check "release: a mirror whose file is now of another size is a problem (206)" has "$out" \
    "  - $T/mirrored.tsv: resources.db.patch_3: $M/files/resources.db.patch_3 says the file is $((size3 - 1)) bytes now, not $size3 (it gave the file before)"
mv "$T/mirror/kept_3" "$T/mirror/resources.db.patch_3"
: >"$T/mirror.log"
run quiet --dry-run --minutes 2 || fail "release.sh --dry-run, mirrors with ranges: $out"
check "release: a mirror that answers its first bytes with a 206 of the row's size is good" has "$out" \
    "the offered-patch list: 2 patch(es), each on the patches server (patches) with its sha256, and its 4 URL(s) giving it"
check "release: ... asked for its first bytes only" \
    bash -c "test -s $T/mirror.log && ! grep -v ' bytes=0-12\$' $T/mirror.log"
rm -f "$T/mirror.ranges"
printf 'resources.db.patch_3\t%s\t%s\t%s %s\nresources.db.patch_12\t%s\t%s\t%s\n' "$sha3" "$size3" "$P/resources.db.patch_3" \
    "$M/page/resources.db.patch_3" "$sha12" "$(stat -c %s "$T/r12/resources.db.patch_12")" "$P/resources.db.patch_12" >"$T/page.tsv"
SKHID_RELEASE_ENV=$(changed_env "s|^SKHID_PATCH_LIST=.*|SKHID_PATCH_LIST=$T/page.tsv|")
run quiet --dry-run --minutes 2 || true
check "release: a mirror that is a web page is a problem" has "$out" \
    "resources.db.patch_3: $M/page/resources.db.patch_3 answers with a web page"
SKHID_RELEASE_ENV=$T/release.env

# A failure before the swap: the build of the image.
touch "$T/build-fails"
run yes --minutes 2 --label c || true
rm "$T/build-fails"
check "release: a failed build stops before the swap" has "$out" "STOPPED before the swap"
check "release: ... cancels the maintenance, planned before the build" test "$(admin_log)" = "plan 2 cancel" -a ! -e "$T/state/maint"
check "release: ... removes the staged files" nothing_staged
check "release: ... puts the image tag back" grep -q "docker tag $(cat "$T/docker/running") srp-026g:latest" "$T/log"
check "release: ... srp-026g:latest is the running image" test "$(cat "$T/docker/latest")" = "$(cat "$T/docker/running")"
check "release: ... leaves the live files as they were" test "$(tree)" = "$before"
check "release: ... restarts nothing" bash -c "! grep -q ' up ' $T/log"

# A no at the question before the swap.
: >"$T/state/admin.log"
run no --minutes 1 --label g || true
check "release: a no at the swap cancels the maintenance" test "$(admin_log)" = "plan 1 cancel"
check "release: ... swaps nothing" has "$out" "nothing was swapped"
check "release: ... removes the staged files" nothing_staged
check "release: ... leaves the live files as they were" test "$(tree)" = "$before"

# No answer at the question before the swap.
: >"$T/state/admin.log"
SKHID_SWAP_ANSWER_MINUTES=2 run wait --minutes 1 --label h || true
check "release: no answer at the swap is a no" has "$out" "no answer in 2 min is a no"
check "release: ... swaps nothing" has "$out" "nothing was swapped"
check "release: ... cancels the maintenance" test "$(admin_log)" = "plan 1 cancel"
check "release: ... leaves the live files as they were" test "$(tree)" = "$before"

# srp makes no backup at its start (it never opened srp.db): the release does
# not go on without it, and the rollback keeps srp.db as it is.
: >"$T/state/admin.log"
touch "$T/no-backup"
lines_before=$(grep -c . "$T/volume/srp.db")
run yes --minutes 1 --label k || true
rm "$T/no-backup"
check "release: no new backup of srp.db, no release" has "$out" "srp made no backup of srp.db at its start"
check "release: ... rolled back" has "$out" "rolled back: srp26"
check "release: ... srp.db kept as it is" bash -c "grep -q 'srp.db: kept as it is (the new srp made no backup' <<<\"\$1\" && test \$(grep -c . $T/volume/srp.db) -eq $((lines_before + 2))" _ "$out"
check "release: ... srp26 running" test "$(cat "$T/docker/state")" = running
check "release: ... the ZIP never moved" cmp -s "$T/work/r13.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "release: ... nothing staged" nothing_staged
check "release: ... the setting as it was" grep -qx "SRP_MIN_PATCH_REVISION=13" "$T/live/.env"

# A match that does not end.
: >"$T/state/admin.log"
echo 1000 >"$T/state/busy"
before=$(tree)
SKHID_MATCH_WAIT_MINUTES=2 run yes --minutes 1 --label e || true
check "release: a match still running stops it" has "$out" "match(es) still running 2 min after the planned time"
check "release: ... cancels the maintenance" test "$(admin_log)" = "plan 1 cancel"
check "release: ... removes the staged files" nothing_staged
check "release: ... its folder says stopped" grep -qx stopped "$(folder 14 e)/state"
check "release: ... leaves the live files as they were" test "$(tree)" = "$before"
rm -f "$T/state/busy"

# srp refuses to start on the new data, before it opens srp.db: its log, then
# the rollback of it all (srp.db, untouched, kept).
: >"$T/state/admin.log"
echo "ERROR srp: SRP_MIN_PATCH_REVISION is 13, below resources.db.patch_14 of /srv/srp/download/server-game-data.db. Set SRP_MIN_PATCH_REVISION=14 or more" >"$T/refuse-start"
running_before=$(cat "$T/docker/running")
data_before=$(sha256sum <"$T/live/download/server-game-data.db")
starts=$(cat "$T/docker/starts")
run yes --resources "$T/r14/resources.db.patch_14" --minutes 1 --label f || true
rm -f "$T/refuse-start"
check "release: srp refusing to start fails the release" has "$out" "srp26 is not up: restarting"
check "release: ... shows why, from srp's log" has "$out" "srp refused to start:"
check "release: ... the line that says why" has "$out" "  > ERROR srp: SRP_MIN_PATCH_REVISION is 13"
check "release: ... rolls back" has "$out" "rolled back: srp26 .*/patches .* as before"
check "release: ... the ZIP never moved" cmp -s "$T/work/r13.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "release: ... the server data back" test "$(sha256sum <"$T/live/download/server-game-data.db")" = "$data_before"
check "release: ... the setting back" grep -qx "SRP_MIN_PATCH_REVISION=13" "$T/live/.env"
check "release: ... the image back, by its ID" test "$(cat "$T/docker/running")|$(cat "$T/docker/latest")" = "$running_before|$running_before"
check "release: ... the source back" grep -qx "crates 2" "$T/live/crates/a/src/lib.rs"
check "release: ... srp.db as it was (the refused start never opened it)" bash -c "grep -q '^start $((starts + 1)) $running_before' $T/volume/srp.db && test \$(grep -c '^start ' $T/volume/srp.db) -le $((starts + 1))"
check "release: ... nothing staged" nothing_staged
check "release: ... the folder says rolled-back" grep -qx rolled-back "$(folder 14 f)/state"
check "release: ... the patch stays published and listed" bash -c "test -f $T/patches/resources.db.patch_14 && grep -q '^resources.db.patch_14' $T/patches.tsv"
check "release: ... the lock freed" test ! -e "$T/lock"

# srp refuses after it opened srp.db (and backed it up): the rollback puts
# that backup back.
: >"$T/state/admin.log"
echo "ERROR srp: cannot load the 0.26g accounts: the kept accounts name what the server's game data (x) lacks; item 99" >"$T/refuse-start"
starts=$(cat "$T/docker/starts")
run yes --minutes 1 --label m || true
rm -f "$T/refuse-start"
check "release: a refusal after srp.db was opened rolls back too" has "$out" "rolled back: srp26"
check "release: ... with srp's backup of that start" has "$out" "srp.db: as it was before the release (srp-[0-9]*.db)"
check "release: ... srp.db: the rollback's start, after the one before" bash -c "grep -q '^start $((starts + 1)) ' $T/volume/srp.db"

# A match that ends a little after the planned time (patch_14 listed now).
: >"$T/state/admin.log"
echo 3 >"$T/state/busy"
run yes --minutes 1 --label d || fail "release.sh with a match: $out"
rm -f "$T/state/busy"
check "release: waits while a match runs" has "$out" "1 match(es) still running; waiting"
check "release: ... then releases" cmp -s "$T/work/r14.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "release: ... with the new patch" offers patch_14
check "release: ... without cancelling" test "$(admin_log)" = "plan 1"

# release/rollback: the folders, then one rolled back keeping srp.db.
out=$("$here/rollback" 2>&1) || fail "rollback (the list): $out"
check "rollback lists the release folders with their state" has "$out" "release-rev14-[0-9-]*-d  released"
D_FOLDER=$(basename "$(folder 14 d)")
starts=$(cat "$T/docker/starts")
out=$(yes_to "$here/rollback" --keep-db "$D_FOLDER") || fail "rollback --keep-db: $out"
check "rollback --keep-db: the ZIP back" cmp -s "$T/work/r13.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "rollback --keep-db: srp.db as it was" grep -q "^start $starts " "$T/volume/srp.db"
check "rollback --keep-db: the setting back" grep -qx "SRP_MIN_PATCH_REVISION=13" "$T/live/.env"
check "rollback --keep-db: the folder says rolled-back" grep -qx rolled-back "$(folder 14 d)/state"
expect_refused "rollback refuses what is not a release folder" "not the name of a release folder" "$here/rollback" "../etc"
# Rolled back again while players own items the old data lacks: no loop, the
# advice to roll forward, and the way back to the release.
echo "ERROR srp: cannot load the 0.26g accounts: the kept accounts name what the server's game data (x) lacks; item 14" >"$T/refuse-start"
out=$(yes_to "$here/rollback" --keep-db "$D_FOLDER" || true)
rm -f "$T/refuse-start"
check "rollback: srp refusing the old files: roll forward" has "$out" "roll forward with a fixed patch"
check "rollback: ... srp26 stopped, not looping" test "$(cat "$T/docker/state")" = stopped
check "rollback: ... the folder says rollback-refused" grep -qx rollback-refused "$(folder 14 d)/state"
back=$(grep -A1 'To put the release back as it was:' <<<"$out" | tail -n 1)
(bash -c "$back") >/dev/null 2>&1 || fail "the way back to the release: $back"
check "rollback: ... its way back puts the release in place again" bash -c "cmp -s $T/work/r14.zip $T/live/download/Survarium-0.26g-srp-files.zip && grep -qx SRP_MIN_PATCH_REVISION=14 $T/live/.env && test \$(cat $T/docker/state) = running"

# --test: the test environment's compose project and patches server, no copy
# of srp.db to the jump host (it is there).
run quiet --test --dry-run --resources "$T/r14/resources.db.patch_14" || true
check "release --test: compose project srptest" has "$out" "docker compose -p srptest up -d --no-deps --force-recreate srp26"
check "release --test: the TEST patches server" has "$out" "On the TEST patches server"
check "release --test: no copy of srp.db to the jump host" bash -c "! grep -q 'copied to the jump host' <<<\"\$1\"" _ "$out"

# --no-maintenance: no countdown, the admin page never asked; prepared at
# once, swapped as soon as "matches":0 (and once more right before the stop).
commit 15 "revision 15"
: >"$T/state/admin.log"
: >"$T/state/admin-seen.log"
before=$(tree)
run quiet --no-maintenance --dry-run || fail "release.sh --no-maintenance --dry-run: $out"
check "release --no-maintenance --dry-run: the preflight needs no admin page" has "$out" "the admin page: not needed (--no-maintenance)"
check "release --no-maintenance --dry-run: plans no maintenance" has "$out" "no maintenance planned (--no-maintenance: the admin page is never asked)"
check "release --no-maintenance --dry-run: ... shows no POST" bash -c "! grep -q 'POST ' <<<\"\$1\"" _ "$out"
check "release --no-maintenance --dry-run: the swap once \"matches\":0, up to 120 min" has "$out" 'wait for "matches":0 on .*/api/stats (up to 120 min), then y/N'
check "release --no-maintenance --dry-run: \"matches\":0 once more, right before the stop" bash -c \
    "grep -A3 'matches\":0 once more right before the stop' <<<\"\$1\" | grep -q 'compose stop srp26'" _ "$out"
check "release --no-maintenance --dry-run: says the lobby reconnects" has "$out" '~30 s "server sync" and reconnect'
check "release --no-maintenance --dry-run: changes nothing" test "$(tree)" = "$before"
check "release --no-maintenance --dry-run: asks no admin page" test ! -s "$T/state/admin-seen.log"
expect_refused "release: --minutes and --no-maintenance refused together" "and --no-maintenance has none" \
    "$here/release.sh" --no-maintenance --minutes 5
expect_refused "release: --match-wait is a number" "match-wait: a whole number" "$here/release.sh" --no-maintenance --match-wait soon

# The limit passes with a match still running: a clean stop.
echo 1000 >"$T/state/busy"
run yes --no-maintenance --match-wait 2 --label nme || true
rm -f "$T/state/busy"
check "release --no-maintenance: a match past --match-wait stops it" has "$out" "match(es) still running after 2 min of waiting (--match-wait): nothing was swapped"
check "release --no-maintenance: ... removes the staged files" nothing_staged
check "release --no-maintenance: ... its folder says stopped" grep -qx stopped "$(folder 15 nme)/state"
check "release --no-maintenance: ... leaves the live files as they were" test "$(tree)" = "$before"
check "release --no-maintenance: ... restarts nothing" bash -c "! grep -qE ' (up|stop) ' $T/log"
check "release --no-maintenance: ... cancels no maintenance (none was planned)" bash -c "! grep -qE '^maintenance: ' <<<\"\$1\"" _ "$out"
check "release --no-maintenance: ... the clock counts from the go" bash -c "grep -q 'since the go\]' <<<\"\$1\" && ! grep -q 'planned time\]' <<<\"\$1\"" _ "$out"
check "release --no-maintenance: ... frees the lock" test ! -e "$T/lock"
check "release --no-maintenance: ... asked no admin page" test ! -s "$T/state/admin-seen.log"

# A match, then none, then one begun during the y/N: the check before the
# stop waits for it, and the stop comes right after a "matches":0.
printf '1\n1\n0\n2\n0\n' >"$T/state/matches"
args_before=$(grep -c '' "$T/args")
run yes --no-maintenance --label nm || fail "release.sh --no-maintenance: $out"
rm -f "$T/state/matches"
check "release --no-maintenance: waits while a match runs" has "$out" "1 match(es) still running; waiting"
check "release --no-maintenance: ... the check before the stop waits for the new one" bash -c \
    "grep -q '2 match(es) still running; waiting' <<<\"\$1\" && grep -q 'matches\":0 once more: stopping srp26 now' <<<\"\$1\"" _ "$out"
# The ssh to the jump host that stops srp26 (its command quoted: \ before a
# space), and what came just before it.
stop_at=$(tail -n +"$((args_before + 1))" "$T/args" | grep -n 'compose.\{1,2\}stop.\{1,2\}srp26' | head -n 1 | cut -d: -f1)
check "release --no-maintenance: ... the stop right after /api/stats said 0" \
    bash -c "tail -n +$((args_before + 1)) $T/args | sed -n '$((${stop_at:-1} - 1))p' | grep -q '^curl .*/api/stats$'"
check "release --no-maintenance: ... then releases" cmp -s "$T/work/r15.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "release --no-maintenance: ... SRP_MIN_PATCH_REVISION=15" grep -qx "SRP_MIN_PATCH_REVISION=15" "$T/live/.env"
check "release --no-maintenance: ... its folder says released" grep -qx released "$(folder 15 nm)/state"
check "release --no-maintenance: ... asked no admin page" test ! -s "$T/state/admin-seen.log"
check "release --no-maintenance: ... planned nothing" test ! -s "$T/state/admin.log"
check "release --no-maintenance: ... frees the lock" test ! -e "$T/lock"

# The default: the go, then at once the maintenance, planned to start 30 s
# after the last running match ends, as /api/stats says
# (last_match_ends_in), in whole minutes; from an srp without it, the
# launcher page's "Time left", else a whole match. Then, during the
# countdown, everything prepared and staged; the swap at the later of the
# two. The dry run shows that order, and the minutes it would plan now.
commit 16 "revision 16"
: >"$T/state/admin.log"
: >"$T/state/admin-seen.log"
: >"$T/state/admin-staged.log"
plan_line() { grep -m1 'plan the maintenance in' <<<"$out" || true; }
line_of() { grep -n -m1 -- "$2" <<<"$1" | cut -d: -f1; } # <text> <pattern>
in_order() { # <text> <pattern>...: each found, each after the one before
    local text=$1 at last=0
    shift
    for pattern; do
        at=$(line_of "$text" "$pattern")
        [[ -n $at ]] && ((at > last)) || return 1
        last=$at
    done
}
echo 300 >"$T/state/ends-in"
run quiet --dry-run || fail "release.sh --dry-run, a match ending in 5 min: $out"
check "release: a match ending in 5 min: the maintenance in 6 min" bash -c "grep -q 'in 6 min (now: /api/stats: the last match ends in 5:00 at the latest (last_match_ends_in 300), then 30 s' <<<\"\$1\"" _ "$(plan_line)"
check "release --dry-run: prints the order: the lock and the go, the plan, the staging, the swap" in_order "$out" \
    '^\[dry run\] 2. take the release lock .*, then wait for the go' '^\[dry run\] 3. right after the go: plan the maintenance' \
    '^\[dry run\] 4. during the countdown, build here' '^\[dry run\] 5. at the later of the planned time and the end of the staging' \
    'docker compose up -d --no-deps --force-recreate srp26'
echo 299 >"$T/state/ends-in"
run quiet --dry-run || true
check "release: ... 4:59 to the last match's end: 6 too (rounded up)" bash -c "grep -q 'in 6 min' <<<\"\$1\"" _ "$(plan_line)"
echo 0 >"$T/state/ends-in"
run quiet --dry-run || true
check "release: no match running: 1 min" bash -c "grep -q 'in 1 min (now: /api/stats: no match runs (last_match_ends_in 0), then 30 s' <<<\"\$1\"" _ "$(plan_line)"
check "release --dry-run: ... shows the POST" has "$out" "tab=maint minutes=1 "
check "release --dry-run: ... asks the admin page nothing but the preflight's GET" test "$(sort -u "$T/state/admin-seen.log")" = GET
rm "$T/state/ends-in"
run quiet --dry-run || true
check "release: an srp without last_match_ends_in, \"matches\":0: 1 min, and it says why" bash -c "grep -q 'in 1 min (now: /api/stats has no last_match_ends_in (an srp from before it); \"matches\":0' <<<\"\$1\"" _ "$(plan_line)"
printf '2\n2\n' >"$T/state/matches"
printf '5\n9\n' >"$T/state/page-left"
run quiet --dry-run || true
check "release: ... matches running: the launcher page's most \"Time left\" + 1" bash -c "grep -q 'in 10 min (now: /api/stats has no last_match_ends_in .*: 2 match(es), the last ending in <= 9 min, then 30 s' <<<\"\$1\"" _ "$(plan_line)"
printf '2\n2\n' >"$T/state/matches"
printf '5\n-\n' >"$T/state/page-left"
run quiet --dry-run || true
check "release: ... one still loading (no time left): a whole match and 30 s, 16 min" bash -c "grep -q 'in 16 min (now: .*(a match with no time left yet (still loading)): a whole match, 15 min, then 30 s' <<<\"\$1\"" _ "$(plan_line)"
printf '3\n3\n' >"$T/state/matches"
printf '5\n9\n' >"$T/state/page-left"
run quiet --dry-run || true
check "release: ... the page lists fewer than run: 16 min" bash -c "grep -q 'in 16 min (now: .*(2 time(s) left for 3 match(es))' <<<\"\$1\"" _ "$(plan_line)"
rm -f "$T/state/matches" "$T/state/page-left"
check "release --dry-run: none of these planned anything" test ! -s "$T/state/admin.log"

# A stop after the plan (build-zip fails, during the countdown): the
# maintenance cancelled.
echo 300 >"$T/state/ends-in"
: >"$T/state/admin-seen.log"
before=$(tree)
touch "$T/build-zip-fails"
run yes --label stop || true
rm "$T/build-zip-fails"
check "release: a stop during the countdown (build-zip failed)" has "$out" "STOPPED before the swap"
check "release: ... the admin page: the preflight's GET, the plan, the cancel" test "$(tr '\n' ' ' <"$T/state/admin-seen.log" | sed 's/POST [^ ]*/POST/g; s/ $//')" = "GET POST POST"
check "release: ... the maintenance planned, then cancelled" test "$(admin_log)" = "plan 6 cancel" -a ! -e "$T/state/maint"
check "release: ... says so" has "$out" "^maintenance: cancelled"
check "release: ... leaves the live files as they were" test "$(tree)" = "$before"
check "release: ... frees the lock" test ! -e "$T/lock"

# Ctrl-C while it waits for the planned time (to bash only, as a trap during
# a sleep of a minute once waited for the sleep): cleaned up in a few
# seconds. A minute is 20 s here, so each wait is 20 s.
: >"$T/state/admin.log"
set -m
SKHID_TEST_SECONDS_PER_MINUTE=20 "$here/release.sh" --yes --i-know --minutes 3 --label int >"$T/int.out" 2>&1 </dev/null &
int_pid=$!
set +m
for _ in $(seq 300); do grep -q '5. waiting for the planned time' "$T/int.out" && break; sleep 0.1; done
sleep 1
kill -INT "$int_pid"
int_at=$SECONDS
for _ in $(seq 150); do kill -0 "$int_pid" 2>/dev/null || break; sleep 0.1; done
int_took=$((SECONDS - int_at))
if kill -0 "$int_pid" 2>/dev/null; then kill -9 "$int_pid"; fi
wait "$int_pid" 2>/dev/null || true
out=$(cat "$T/int.out")
printf '%s\n' "$out" >>"$T/outputs"
check "release: Ctrl-C in the wait for the planned time ends it in a few seconds (${int_took} s)" test "$int_took" -le 4
check "release: ... it was waiting, the maintenance planned" bash -c "grep -q 'maintenance planned' <<<\"\$1\" && grep -q 'interrupted' <<<\"\$1\"" _ "$out"
check "release: ... cancels the maintenance" test "$(admin_log)" = "plan 3 cancel" -a ! -e "$T/state/maint"
check "release: ... removes the staged files" nothing_staged
check "release: ... its folder says stopped" grep -qx stopped "$(folder 16 int)/state"
check "release: ... leaves the live files as they were" test "$(tree)" = "$before"
check "release: ... frees the lock" test ! -e "$T/lock"

# Ctrl-C during the staging (the build of the ZIP), to the whole process
# group as a terminal sends it: the plan made before is cancelled.
: >"$T/state/admin.log"
echo 30 >"$T/build-zip-sleep"
set -m
"$here/release.sh" --yes --i-know --minutes 60 --label stg >"$T/stg.out" 2>&1 </dev/null &
stg_pid=$!
set +m
for _ in $(seq 300); do grep -q '4. building the ZIP' "$T/stg.out" && break; sleep 0.1; done
sleep 1
kill -INT -- "-$stg_pid"
stg_at=$SECONDS
for _ in $(seq 150); do kill -0 "$stg_pid" 2>/dev/null || break; sleep 0.1; done
stg_took=$((SECONDS - stg_at))
if kill -0 "$stg_pid" 2>/dev/null; then kill -9 -- "-$stg_pid"; fi
wait "$stg_pid" 2>/dev/null || true
rm -f "$T/build-zip-sleep"
out=$(cat "$T/stg.out")
printf '%s\n' "$out" >>"$T/outputs"
check "release: Ctrl-C during the staging, after the plan, ends it in a few seconds (${stg_took} s)" test "$stg_took" -le 5
check "release: ... it was building, the maintenance planned" bash -c "grep -q 'maintenance planned' <<<\"\$1\" && grep -q 'interrupted' <<<\"\$1\"" _ "$out"
check "release: ... cancels the maintenance" test "$(admin_log)" = "plan 60 cancel" -a ! -e "$T/state/maint"
check "release: ... says so" has "$out" "^maintenance: cancelled"
check "release: ... staged nothing (no folder yet)" test -z "$(folder 16 stg)"
check "release: ... leaves the live files as they were" test "$(tree)" = "$before"
check "release: ... frees the lock" test ! -e "$T/lock"

# A closed terminal (HUP to the whole group, the release.log copy too) and
# a second Ctrl-C during the cleanup: the cleanup still finishes (the plan
# cancelled, the lock freed); a maintenance left planned would put the
# server down with nothing swapped.
for how in hup twice; do
    : >"$T/state/admin.log"
    echo 30 >"$T/build-zip-sleep"
    set -m
    "$here/release.sh" --yes --i-know --minutes 60 --label "$how" >"$T/$how.out" 2>&1 </dev/null &
    sig_pid=$!
    set +m
    for _ in $(seq 300); do grep -q '4. building the ZIP' "$T/$how.out" && break; sleep 0.1; done
    sleep 1
    if [[ $how == hup ]]; then
        kill -HUP -- "-$sig_pid"
    else
        kill -INT -- "-$sig_pid"
        sleep 0.2
        kill -INT -- "-$sig_pid" 2>/dev/null || true
    fi
    for _ in $(seq 150); do kill -0 "$sig_pid" 2>/dev/null || break; sleep 0.1; done
    if kill -0 "$sig_pid" 2>/dev/null; then kill -9 -- "-$sig_pid"; fi
    wait "$sig_pid" 2>/dev/null || true
    rm -f "$T/build-zip-sleep"
    printf '%s\n' "$(cat "$T/$how.out")" >>"$T/outputs"
    check "release: $how during the staging: the plan is still cancelled" test "$(admin_log)" = "plan 60 cancel" -a ! -e "$T/state/maint"
    check "release: ... and the lock freed" test ! -e "$T/lock"
    check "release: ... the live files as they were" test "$(tree)" = "$before"
done

# Without the go given beforehand: it waits for it right after the
# preflight, having built, staged and asked the admin page nothing; the go
# file lets it plan at once. Then Ctrl-C (the plan is cancelled, as any
# stop).
: >"$T/state/admin.log"
: >"$T/state/admin-seen.log"
echo 300 >"$T/state/ends-in"
set -m
SKHID_RELEASE_GO='' SKHID_TEST_SECONDS_PER_MINUTE=20 "$here/release.sh" --yes --i-know --label go >"$T/go.out" 2>&1 </dev/null &
go_pid=$!
set +m
for _ in $(seq 600); do grep -q "waiting for the go" "$T/go.out" && break; sleep 0.1; done
sleep 3
check "release: without the go given, it waits for it after the preflight" grep -q "2. waiting for the go" "$T/go.out"
check "release: ... having asked the admin page nothing but the preflight's GET" test "$(cat "$T/state/admin-seen.log")" = GET
check "release: ... built and staged nothing" bash -c "! grep -q '4. building' $T/go.out && test -z \"\$(find $T/live -maxdepth 1 -name 'release-rev16-*-go')\""
go_file=$(sed -n "s/^Go: answer y here, or run: touch //p" "$T/go.out" | head -n 1)
check "release: ... names its go file, in the cache" bash -c "[[ \$1 == $T/cache/skhid-release/go-* ]]" _ "$go_file"
touch "$go_file"
for _ in $(seq 300); do grep -q "3. planning the maintenance in 6 min" "$T/go.out" && break; sleep 0.1; done
check "release: ... the go file lets it plan, at once" grep -q "3. planning the maintenance in 6 min" "$T/go.out"
check "release: ... and it takes the file away" test ! -e "$go_file"
for _ in $(seq 300); do grep -q "5. waiting for the planned time" "$T/go.out" && break; sleep 0.1; done
check "release: ... then it stages, during the countdown" in_order "$(cat "$T/go.out")" \
    '^go: ' '3. maintenance planned' '4. staging in the release folder' '5. waiting for the planned time'
kill -INT "$go_pid"
for _ in $(seq 150); do kill -0 "$go_pid" 2>/dev/null || break; sleep 0.1; done
if kill -0 "$go_pid" 2>/dev/null; then kill -9 "$go_pid"; fi
wait "$go_pid" 2>/dev/null || true
printf "%s\n" "$(cat "$T/go.out")" >>"$T/outputs"
check "release: ... stopped after the go, the plan cancelled" test "$(admin_log)" = "plan 6 cancel" -a ! -e "$T/state/maint"
check "release: ... frees the lock" test ! -e "$T/lock"

# Ctrl-C while it waits for the go: nothing at all.
before=$(tree)
: >"$T/state/admin.log"
: >"$T/state/admin-seen.log"
set -m
SKHID_RELEASE_GO='' "$here/release.sh" --yes --i-know --label nogo >"$T/nogo.out" 2>&1 </dev/null &
nogo_pid=$!
set +m
for _ in $(seq 600); do grep -q "waiting for the go" "$T/nogo.out" && break; sleep 0.1; done
kill -INT "$nogo_pid"
for _ in $(seq 150); do kill -0 "$nogo_pid" 2>/dev/null || break; sleep 0.1; done
if kill -0 "$nogo_pid" 2>/dev/null; then kill -9 "$nogo_pid"; fi
wait "$nogo_pid" 2>/dev/null || true
printf "%s\n" "$(cat "$T/nogo.out")" >>"$T/outputs"
check "release: Ctrl-C before the go: nothing planned, nothing cancelled" test ! -s "$T/state/admin.log"
check "release: ... nothing built or staged" bash -c "! grep -qE '4\\. |STOPPED before the swap' $T/nogo.out"
check "release: ... the live files as they were" test "$(tree)" = "$before"
check "release: ... frees the lock" test ! -e "$T/lock"

# The whole default run: a match ending in 5 min. The go, then the plan (6
# min), with nothing staged yet, then the staging, then the swap.
: >"$T/state/admin.log"
: >"$T/state/admin-seen.log"
: >"$T/state/admin-staged.log"
echo 300 >"$T/state/ends-in"
run yes --label auto || fail "release.sh, the default: $out"
check "release: the default plans the maintenance in 6 min (a match ending in 5)" test "$(admin_log)" = "plan 6"
check "release: ... the go, then the plan, then the staging, then the swap" in_order "$out" \
    '^go: given beforehand' '3. planning the maintenance in 6 min' '3. maintenance planned' '4. building the server data' \
    '4. staging in the release folder' '5. the swap'
check "release: ... planned before anything was staged (the admin page saw no staged folder)" test "$(cat "$T/state/admin-staged.log")" = "plan 6 NOT staged"
check "release: ... the admin page: the preflight's GET, then the plan" test "$(tr '\n' ' ' <"$T/state/admin-seen.log" | sed 's/ POST [^ ]*/ POST/; s/ $//')" = "GET POST"
check "release: ... says Play is locked, and that it builds now" has "$out" 'the launcher says "Maintenance in 6 min"; Play is locked; building and staging now'
check "release: ... then releases" cmp -s "$T/work/r16.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "release: ... SRP_MIN_PATCH_REVISION=16" grep -qx "SRP_MIN_PATCH_REVISION=16" "$T/live/.env"
check "release: ... the folder's rollback" test -f "$(folder 16 auto)/rollback.sh" -a -f "$(folder 16 auto)/rollback.env"
rm -f "$T/state/ends-in"

# Staging that ends after the planned time: the swap at once.
commit 17 "revision 17"
: >"$T/state/admin.log"
echo 3 >"$T/build-zip-sleep"
run yes --minutes 1 --label late || fail "release.sh, staged late: $out"
rm -f "$T/build-zip-sleep"
check "release: staged after the planned time: the swap at once" has "$out" "5. staged after the planned time: the swap at once"
check "release: ... no wait for the planned time" bash -c "! grep -q 'waiting for the planned time' <<<\"\$1\"" _ "$out"
check "release: ... then releases" cmp -s "$T/work/r17.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "release: ... without cancelling" test "$(admin_log)" = "plan 1"

# --now: no maintenance, no notice, no wait.
commit 18 "revision 18"
expect_refused "release: --now and --minutes refused together" "and --now has none" "$here/release.sh" --now --minutes 5
: >"$T/state/admin.log"
: >"$T/state/admin-seen.log"
before=$(tree)
run quiet --dry-run --now --no-maintenance || fail "release.sh --now --dry-run: $out"
check "release --now: --no-maintenance with it is accepted, and said to be the same" has "$out" "note: --now plans no maintenance anyway"
check "release --now --dry-run: says there is no notice" has "$out" "maintenance: NONE, NO NOTICE (--now)"
check "release --now --dry-run: prints the order: the go, the staging, the swap at once" in_order "$out" \
    '^\[dry run\] 2. take the release lock .*, then wait for the go' '^\[dry run\] 3. no maintenance planned (--now' \
    '^\[dry run\] 4. after the go, build here' '^\[dry run\] 5. once staged: the swap at once, NO wait for matches' \
    'the stop ends without results (--now)' 'compose stop srp26'
check "release --now --dry-run: no POST, no admin page" bash -c "! grep -q 'POST ' <<<\"\$1\" && test ! -s $T/state/admin-seen.log" _ "$out"
check "release --now --dry-run: changes nothing" test "$(tree)" = "$before"
# A refused start with --now: rolled back by itself all the same.
echo "ERROR srp: SRP_MIN_PATCH_REVISION is 13, below resources.db.patch_14. Set SRP_MIN_PATCH_REVISION=14 or more" >"$T/refuse-start"
run yes --now --label nowf || true
rm -f "$T/refuse-start"
check "release --now: srp refusing to start: rolled back" has "$out" "rolled back: srp26 .*/patches .* as before"
check "release --now: ... the ZIP never moved" cmp -s "$T/work/r17.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
# A match running: swapped anyway, and it says how many it ended.
echo 1000 >"$T/state/busy"
run yes --now --label now || fail "release.sh --now: $out"
rm -f "$T/state/busy"
check "release --now: with a match running, swaps anyway" cmp -s "$T/work/r18.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "release --now: ... says, right before the stop, the match it ends" in_order "$out" \
    '5. the swap' '--now: 1 match(es) running: the stop ends them now, WITHOUT RESULTS' 'the server is up on the new data'
check "release --now: ... and at the end how many it ended" has "$out" "--now: the stop ended 1 match(es) without results"
check "release --now: ... kept in the folder's release.log" grep -q -- "--now: the stop ended 1 match(es) without results" "$(folder 18 now)/release.log"
check "release --now: ... never waited for a match" bash -c "! grep -q 'still running; waiting' <<<\"\$1\"" _ "$out"
check "release --now: ... planned nothing, asked no admin page" test ! -s "$T/state/admin.log" -a ! -s "$T/state/admin-seen.log"
check "release --now: ... SRP_MIN_PATCH_REVISION=18" grep -qx "SRP_MIN_PATCH_REVISION=18" "$T/live/.env"
check "release --now: ... frees the lock" test ! -e "$T/lock"

# No secret in any output or command line.
check "release: the admin token is in no output" bash -c "! grep -qF -- '$token' $T/outputs"
check "release: the admin path is in no output" bash -c "! grep -qF -- '$admin_path' $T/outputs"
check "release: ... nor in a folder's release.log" bash -c "! grep -qF -e '$token' -e '$admin_path' $T/live/release-rev*/release.log"
check "release: no setting's value is printed" bash -c "! grep -qF 'SRP_DAILY_BONUS=5' $T/outputs"
check "release: neither is in a command line of ssh, curl or docker" bash -c "! grep -qF -e '$token' -e '$admin_path' $T/args $T/log"
check "release: the admin page was asked through curl's stdin, on the jump host" grep -q "^ssh jump curl .* -K -$" "$T/args"
check "release: no other container was ever run" bash -c "! grep -q '^docker run' $T/log"

echo
if ((failures)); then
    echo "$failures FAILED"
    exit 1
fi
echo "all passed"
