#!/usr/bin/env bash
# The release scripts against stand-ins, on this machine only: ssh runs the
# command here (so the jump host, the live host and the patches server are
# folders under a temporary one), docker, caddy and systemctl only log
# (docker "restarts" the site: /patches and /api/client-files of what is in
# place), and two small HTTP servers stand for the patches server and the
# site with its admin page. Nothing is sent anywhere. Every confirmation is
# answered by a pipe (setsid: no terminal to ask). ssh, curl and docker log
# their arguments, to check that no secret is in a command line.
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
    (cd "$here" && shellcheck -x -s bash lib.sh build-patch upload-patch build-server-db build-zip deploy setup-patches-server release.sh test.sh)
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
mkdir -p "$T/bin" "$T/patches" "$T/site/api" "$T/live/download" "$T/work"

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
if [[ $tool == caddy && -e \$TEST_DIR/caddy-fails ]]; then exit 1; fi
EOF
done
# docker: logs; `compose ... up` with $TEST_DIR/restarts makes the site's
# /patches and /api/client-files those of the files in place and ends the
# maintenance, as a restart of srp does; a build fails with
# $TEST_DIR/build-fails.
cat >"$T/bin/docker" <<'EOF'
#!/usr/bin/env bash
printf 'docker %s\n' "$*" >>"$TEST_LOG"
printf 'docker %s\n' "$*" >>"$TEST_ARGS"
live=$TEST_DIR/live/download
offered() {
    "$SRP_GAME_DATA_BIN" info "$live/server-game-data.db" |
        sed -n 's/^  resources\.db\.\(patch_[0-9]*\) [0-9]* bytes sha256 \([0-9a-f]*\) url \(.*\)$/\1 \2 \3/p'
}
case " $* " in
*" build "*)
    if [[ -e $TEST_DIR/build-fails ]]; then echo "build: cargo failed" >&2; exit 1; fi ;;
*" ps -q "*) echo c0ffee ;;
" inspect "*) echo "running healthy 2026-10-16T12:00:00Z" ;;
*" logs "*) echo "INFO game data: /srv/srp/download/server-game-data.db (2 gameplay files, 1 projects; made of resources.db; $(offered | grep -c .) patch(es) offered at /patches)" ;;
*" up "*)
    if [[ -e $TEST_DIR/restarts ]]; then
        offered | jq -Rn '[inputs | split(" ") | {key: .[0], value: {sha256: .[1], url: .[2]}}] | from_entries' >"$TEST_DIR/site/patches"
        printf 'skhid-client-files 1\nsha256 %s\nsize %s\n' "$(sha256sum <"$live/Survarium-0.26g-srp-files.zip" | cut -d' ' -f1)" \
            "$(stat -c %s "$live/Survarium-0.26g-srp-files.zip")" >"$TEST_DIR/site/api/client-files"
        rm -f "$TEST_DIR/state/maint"
    fi ;;
esac
EOF
chmod +x "$T/bin/"*
export PATH="$T/bin:$PATH" TEST_LOG=$T/log TEST_ARGS=$T/args TEST_DIR=$T
# release.sh and the site: a minute is a second.
export SKHID_TEST_SECONDS_PER_MINUTE=1
: >"$T/log"
: >"$T/args"
mkdir -p "$T/state"

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
# The site: its files, and as srp does, the launcher's places (the
# maintenance countdown of $T/state/maint), the admin page at
# /$(cat $T/state/admin-path) (its maintenance tab; the token of
# $T/state/admin-token; the answer holds the token, as the real page's
# does), /api/stats saying a match runs for the first $T/state/busy asks.
python3 - "$T/site" "$T/site.port" "$T/state" >/dev/null 2>&1 <<'EOF' &
import functools, http.server, math, os, sys, time, urllib.parse
root, port_file, state = sys.argv[1:4]
minute = float(os.environ.get("SKHID_TEST_SECONDS_PER_MINUTE", "60"))
def read(name, default=None):
    try:
        return open(os.path.join(state, name)).read().strip()
    except FileNotFoundError:
        return default
class Handler(http.server.SimpleHTTPRequestHandler):
    server_version, sys_version = "Caddy", ""
    def log_message(self, *args): pass
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
            return self.answer(200, "<table></table>" + text)
        if self.path == "/api/stats" and read("busy"):
            busy = int(read("busy"))
            open(os.path.join(state, "busy"), "w").write(str(busy - 1))
            if busy > 0:
                return self.answer(200, '{"online":3,"registered":3,"matches":1,"queued":0}')
            return self.answer(200, '{"online":0,"registered":3,"matches":0,"queued":0}')
        if self.path == "/" + read("admin-path", "-"):
            return self.answer(200, "<form>the admin page</form>")
        return super().do_GET()
    def do_POST(self):
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
        return self.answer(200, page + f"Maintenance starts in {minutes} min, at 12:00 UTC.")
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Handler, directory=root))
open(port_file, "w").write(str(server.server_address[1]))
server.serve_forever()
EOF
pids+=($!)
for _ in $(seq 50); do [[ -s $T/site.port ]] && break; sleep 0.1; done
patches_port=$(cat "$T/patches.port")
site_port=$(cat "$T/site.port")

cat >"$T/release.env" <<EOF
SKHID_JUMP=jump
SKHID_LIVE=live
SKHID_LIVE_DIR=$T/live
SKHID_SITE=http://127.0.0.1:$site_port
SKHID_PATCHES_SSH=patches
SKHID_PATCHES_DIR=$T/patches
SKHID_PATCHES_URL=http://127.0.0.1:$patches_port
SKHID_PATCHES_CADDYFILE=$T/Caddyfile
SKHID_STOCK_DB=$T/resources.db
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

# The stock db: an archive of a dictionary and a project.
mkdir -p "$T/stock/gameplay/items" "$T/stock/projects/p/day"
echo "dictionary" >"$T/stock/gameplay/db_static_dictionaries"
echo "old item" >"$T/stock/gameplay/items/a.options"
: >"$T/stock/projects/p/day/client_project"
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
out=$(echo n | setsid -w "$here/upload-patch" "$patch" 2>&1 || true)
check "upload-patch: a no uploads nothing" test ! -e "$T/patches/resources.db.patch_3.tmp" -a ! -e "$T/patches/resources.db.patch_3"
check "upload-patch shows the command" has "$out" "cat > $T/patches/resources.db.patch_3.tmp"
out=$("$here/upload-patch" --dry-run "$patch" 2>&1)
check "upload-patch --dry-run writes nothing" test -z "$(ls -A "$T/patches")"
out=$(yes_to "$here/upload-patch" "$patch")
check "upload-patch publishes" cmp -s "$patch" "$T/patches/resources.db.patch_3"
check "upload-patch: mode 644" test "$(stat -c %a "$T/patches/resources.db.patch_3")" = 644
check "upload-patch: no .tmp left" test ! -e "$T/patches/resources.db.patch_3.tmp"
check "upload-patch checks the URL" has "$out" "published: http://127.0.0.1:$patches_port/resources.db.patch_3 (200"
expect_refused "upload-patch: the same file again" "with these same bytes" "$here/upload-patch" "$patch"
mkdir -p "$T/other" && echo other >"$T/p3/gameplay/items/c.options"
"$bin" pack --dir "$T/p3" --out "$T/other/resources.db.patch_3" >/dev/null
expect_refused "upload-patch never replaces a patch" "OTHER bytes" "$here/upload-patch" "$T/other/resources.db.patch_3"
check "the published patch is the first one" cmp -s "$patch" "$T/patches/resources.db.patch_3"
cp "$patch" "$T/work/resources.db.patch_03"
expect_refused "upload-patch refuses a name the game does not mount" "must be resources.db.patch_<n>" \
    "$here/upload-patch" "$T/work/resources.db.patch_03"
head -c 100 /dev/zero >"$T/work/resources.db.patch_4"
expect_refused "upload-patch refuses what is not an archive" "not an archive" "$here/upload-patch" "$T/work/resources.db.patch_4"

# build-server-db
echo junk >"$T/patches/resources.db.patch_9.tmp"
out=$(cd "$T/work" && "$here/build-server-db" --out "$T/work/sgd.db" 2>&1)
sha3=$(sha256sum <"$patch" | cut -d' ' -f1)
check "build-server-db builds" test -f "$T/work/sgd.db"
check "build-server-db ignores other names" has "$out" "ignored on the patches server: resources.db.patch_9.tmp"
check "build-server-db prints the offered list" \
    has "$out" "  patch_3 sha256 $sha3 url http://127.0.0.1:$patches_port/resources.db.patch_3"
check "the data is made of the stock db and the patch" quiet "$bin" check "$T/work/sgd.db" --db "$T/resources.db" \
    --patch "$patch=http://127.0.0.1:$patches_port/resources.db.patch_3"
rm "$T/patches/resources.db.patch_9.tmp"
expect_refused "build-server-db never overwrites" "never overwritten" "$here/build-server-db" --out "$T/work/sgd.db"
sed "s|^SKHID_PATCHES_URL=.*|SKHID_PATCHES_URL=http://127.0.0.1:$site_port|" "$T/release.env" >"$T/other.env"
expect_refused "build-server-db needs each patch at 200" "answered 404, not 200" \
    env SKHID_RELEASE_ENV="$T/other.env" "$here/build-server-db" --out "$T/work/sgd2.db"

# setup-patches-server
printf 'other.example {\n\timport stalker\n}\n' >"$T/Caddyfile"
cp "$T/Caddyfile" "$T/Caddyfile.before"
rmdir "$T/patches" 2>/dev/null || true
mv "$T/patches" "$T/patches.kept"
expect_refused "setup --check: something to do" "would do the above" "$here/setup-patches-server" --check
check "setup --check changes nothing" cmp -s "$T/Caddyfile" "$T/Caddyfile.before"
check "setup --check makes no folder" test ! -e "$T/patches"
mv "$T/patches.kept" "$T/patches"
touch "$T/caddy-fails"
out=$(yes_to "$here/setup-patches-server" || true)
check "setup: a failed caddy validate puts the Caddyfile back" cmp -s "$T/Caddyfile" "$T/Caddyfile.before"
rm "$T/caddy-fails"
out=$(yes_to "$here/setup-patches-server")
check "setup appends the block" test "$(grep -cxF "http://127.0.0.1:$patches_port {" "$T/Caddyfile")" = 1
check "setup keeps the other site as it was" cmp -s "$T/Caddyfile.before" <(head -n 3 "$T/Caddyfile")
check "setup keeps a dated backup" bash -c "ls $T/Caddyfile.bak-*-before-skhid-patches >/dev/null"
check "setup reloads caddy" grep -q "systemctl reload caddy" "$T/log"
check "setup probes the patches" has "$out" "patches there: 1, each at 200"
out=$("$here/setup-patches-server" --check 2>&1)
check "setup --check after: nothing to do" has "$out" "nothing to do"
out=$(echo n | setsid -w "$here/setup-patches-server" 2>&1)
check "setup again: nothing to do, nothing asked" bash -c "! grep -q 'Go ahead' <<<'$out' && grep -q 'nothing to do' <<<'$out'"
printf 'http://127.0.0.1:%s {\n\tfile_server\n}\n' "$patches_port" >>"$T/Caddyfile"
expect_refused "setup refuses the block twice" "2 times" "$here/setup-patches-server" --check
printf 'other.example {\n}\n:%s {\n\tfile_server\n}\n' "$patches_port" >"$T/Caddyfile"
expect_refused "setup refuses another site on the port" "another site on port" "$here/setup-patches-server" --check

# deploy
python3 - "$T/work" <<'EOF'
import sys, zipfile
at = 0x380862 - 0x11000 + 0x400
for revision in (2, 13, 14):
    exe = bytearray(at + 16)
    exe[at:at + 6] = bytes([0xC6, 0x44, 0x24, 0x27, revision, 0x90])
    with zipfile.ZipFile(f"{sys.argv[1]}/r{revision}.zip", "w") as z:
        z.writestr("game/binaries/x86/survarium_skhid.exe", bytes(exe))
        z.writestr("game/binaries/x86/updater_skhid.exe", b"updater")
EOF
cp "$T/work/r13.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
echo "old data" >"$T/live/download/server-game-data.db"
printf 'SRP_ADMIN_TOKEN=very-secret\nSRP_MIN_PATCH_REVISION=13\nOTHER=1' >"$T/live/.env"
zip14=$T/work/r14.zip
zsha=$(sha256sum <"$zip14" | cut -d' ' -f1)
printf 'skhid-client-files 1\nsha256 %s\nsize %s\n' "$zsha" "$(stat -c %s "$zip14")" >"$T/site/api/client-files"
printf '{"patch_3": {"url": "http://127.0.0.1:%s/resources.db.patch_3", "sha256": "%s"}}' "$patches_port" "$sha3" >"$T/site/patches"
echo '{"online":0,"registered":3,"matches":0,"queued":0}' >"$T/site/api/stats"
tree() { (cd "$T/live" && find . -type f -exec sha256sum {} + | sort); }
before=$(tree)
: >"$T/log"
out=$("$here/deploy" --dry-run --zip "$zip14" --server-data "$T/work/sgd.db" --revision 14 2>&1) || fail "deploy --dry-run: $out"
check "deploy --dry-run changes nothing" test "$(tree)" = "$before"
check "deploy --dry-run runs nothing there" test ! -s "$T/log"
check "deploy --dry-run shows the restart" has "$out" "docker compose up -d --no-deps --force-recreate srp26"
expect_refused "deploy refuses a ZIP of another revision" "says patch revision 14, not 15" \
    "$here/deploy" --zip "$zip14" --server-data "$T/work/sgd.db" --revision 15
expect_refused "deploy refuses data offering a later patch" "offers patch_3, above revision 2" \
    "$here/deploy" --zip "$T/work/r2.zip" --server-data "$T/work/sgd.db" --revision 2
out=$(yes_to "$here/deploy" --zip "$zip14" --server-data "$T/work/sgd.db" --revision 14 --label test) || fail "deploy: $out"
check "deploy puts the ZIP" cmp -s "$zip14" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "deploy puts the server data" cmp -s "$T/work/sgd.db" "$T/live/download/server-game-data.db"
check "deploy archives the old ZIP" bash -c "cmp -s $T/work/r13.zip $T/live/old-downloads/Survarium-0.26g-srp-files-*-test.zip"
check "deploy archives the old data" bash -c "grep -q 'old data' $T/live/old-downloads/server-game-data-*-test.db"
check "deploy sets the revision" grep -qx "SRP_MIN_PATCH_REVISION=14" "$T/live/.env"
check "deploy keeps the rest of .env" test "$(grep -v '^SRP_MIN_PATCH_REVISION=' "$T/live/.env")" = $'SRP_ADMIN_TOKEN=very-secret\nOTHER=1'
check "deploy never prints .env" bash -c "! grep -q very-secret <<<'$out'"
check "deploy restarts srp26" grep -q "docker compose up -d --no-deps --force-recreate srp26" "$T/log"
check "deploy checks /patches and /api/client-files" has "$out" "/api/client-files: the new ZIP"
check "deploy goes through the jump host" bash -c "grep -qx jump $T/log && grep -qx live $T/log"
check "deploy leaves no .tmp" bash -c "! ls $T/live/download/*.tmp 2>/dev/null"
sed -i '/^SRP_MIN_PATCH_REVISION=/d' "$T/live/.env"
echo '{}' >"$T/site/patches"
out=$(yes_to "$here/deploy" --zip "$zip14" --server-data "$T/work/sgd.db" --revision 14 --label again || true)
check "deploy adds the setting when missing, on a line of its own" grep -qx "SRP_MIN_PATCH_REVISION=14" "$T/live/.env"
check "deploy says when /patches is not the data's" has "$out" "/patches is {}, not"
out=$(echo n | setsid -w "$here/deploy" --zip "$zip14" --server-data "$T/work/sgd.db" --revision 14 2>&1 || true)
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
echo "REVISION = 14" >"$src/client-patches/patch_026g.py"
commit 14 "revision 14"
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
revision=$(git -C "$TEST_DIR/repo" show "$ref:srp-026g/client-patches/patch_026g.py" | sed -n 's/^REVISION = //p')
mkdir -p "$work"
cp "$TEST_DIR/work/r$revision.zip" "$work/Survarium-0.26g-srp-files.zip"
echo "  CHANGED  game/binaries/x86/survarium_skhid.exe"
echo "updater_skhid.exe: unchanged (the same file as live)"
STANDIN
chmod +x "$T/build-zip"
python3 - "$T/work" <<'PY'
import sys, zipfile
at = 0x380862 - 0x11000 + 0x400
for revision in (15, 16):
    exe = bytearray(at + 16)
    exe[at:at + 6] = bytes([0xC6, 0x44, 0x24, 0x27, revision, 0x90])
    with zipfile.ZipFile(f"{sys.argv[1]}/r{revision}.zip", "w") as z:
        z.writestr("game/binaries/x86/survarium_skhid.exe", bytes(exe))
        z.writestr("game/binaries/x86/updater_skhid.exe", b"updater")
PY
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
cp "$T/work/r13.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
# The first release with server data: there is none on the live host yet.
rm "$T/live/download/server-game-data.db"
printf 'SRP_ADMIN_PATH=%s\nSRP_ADMIN_TOKEN="%s"\nSRP_WEB_TRUST_PROXY=on\nSRP_MIN_PATCH_REVISION=13\nOTHER=1\n' \
    "$admin_path" "$token" >"$T/live/.env"
ln -sf "$T/live/download/Survarium-0.26g-srp-files.zip" "$T/site/download"
echo '{"online":0,"registered":3,"matches":0,"queued":0}' >"$T/site/api/stats"
touch "$T/restarts" "$T/state/admin.log"
: >"$T/outputs"
run() { # <how: yes | no | y-then-n | quiet> release.sh <args...>: its output into $out (and $T/outputs)
    local how=$1
    shift
    : >"$T/log"
    case $how in
    yes) out=$({ yes || true; } | setsid -w "$here/release.sh" "$@" 2>&1) ;;
    no) out=$(echo n | setsid -w "$here/release.sh" "$@" 2>&1) ;;
    y-then-n) out=$(printf 'y\nn\n' | setsid -w "$here/release.sh" "$@" 2>&1) ;;
    quiet) out=$(setsid -w "$here/release.sh" "$@" </dev/null 2>&1) ;;
    esac
    local status=$?
    printf '%s\n' "$out" >>"$T/outputs"
    return "$status"
}
admin_log() { tr '\n' ' ' <"$T/state/admin.log" | sed 's/ $//'; }
offers() { "$bin" info "$T/live/download/server-game-data.db" 2>/dev/null | grep -q "resources.db.$1 "; }
nothing_staged() { ! ls "$T/live/download/"*.tmp "$T/live/.release-src.tmp"* >/dev/null 2>&1; }

# A whole release, no new content, the server code as live.
run yes --minutes 2 --label a || fail "release.sh: $out"
check "release: plans the maintenance from the jump host" test "$(admin_log)" = "plan 2"
check "release: sees the countdown on the launcher page" has "$out" 'the launcher says "Maintenance in [12] min"'
check "release: finds the server code as live" has "$out" "the live tree is main's"
check "release: ... and builds no image" bash -c "! grep -q ' build ' $T/log"
check "release: the new ZIP is in place" cmp -s "$T/work/r14.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "release: the new server data is in place (patch_3)" offers patch_3
check "release: the old ZIP is archived" bash -c "cmp -s $T/work/r13.zip $T/live/old-downloads/Survarium-0.26g-srp-files-*-a.zip"
check "release: no old data to archive (there was none)" bash -c "! ls $T/live/old-downloads/server-game-data-*-a.db 2>/dev/null"
check "release: the rollback puts back no data (there was none)" bash -c "! grep -q 'old-downloads/server-game-data-.*-a.db download' <<<\"\$1\"" _ "$out"
check "release: SRP_MIN_PATCH_REVISION=14" grep -qx "SRP_MIN_PATCH_REVISION=14" "$T/live/.env"
check "release: the rest of .env as it was" test "$(grep -vc '^SRP_MIN_PATCH_REVISION=' "$T/live/.env")" = 4
check "release: restarts srp26" grep -q "docker compose up -d --no-deps --force-recreate srp26" "$T/log"
check "release: waits for no match" has "$out" "4. no match is running"
check "release: checks /patches and /api/client-files" has "$out" "/api/client-files: the new ZIP"
check "release: checks the log's game data line" has "$out" "log: INFO game data: .* 1 patch(es) offered"
check "release: prints the rollback" has "$out" "cp -p old-downloads/Survarium-0.26g-srp-files-.*-a.zip download/Survarium-0.26g-srp-files.zip"
check "release: leaves nothing staged" nothing_staged
check "release: the maintenance ended with the restart" test ! -e "$T/state/maint"

# New content and new server code.
echo "crates 2" >"$src/crates/a/src/lib.rs"
commit 15 "revision 15, server code"
mkdir -p "$T/p15/gameplay/items" "$T/r15"
echo "item 15" >"$T/p15/gameplay/items/d.options"
"$bin" pack --dir "$T/p15" --out "$T/r15/resources.db.patch_15" >/dev/null
: >"$T/state/admin.log"
run yes --resources "$T/r15/resources.db.patch_15" --minutes 2 --label b || fail "release.sh --resources: $out"
check "release --resources: publishes the patch" cmp -s "$T/r15/resources.db.patch_15" "$T/patches/resources.db.patch_15"
check "release --resources: the server data offers it" offers patch_15
check "release --resources: /patches offers it" grep -q patch_15 "$T/site/patches"
check "release: finds the changed server code" has "$out" "CHANGED   crates/a/src/lib.rs"
check "release: tags the running image" grep -q "docker tag srp-026g:latest srp-026g:before-[0-9-]*-b" "$T/log"
check "release: builds main's source, staged" grep -q "docker compose --env-file $T/live/.env -f .release-src.tmp/docker-compose.yml build srp26" "$T/log"
check "release: the source is in place after" grep -qx "crates 2" "$T/live/crates/a/src/lib.rs"
check "release: the old source is archived" bash -c "tar -xzOf $T/live/old-downloads/srp-026g-src-*-b.tar.gz crates/a/src/lib.rs | grep -qx 'crates/a/src/lib.rs 1'"
check "release: the rollback puts the image back" has "$out" "docker tag srp-026g:before-[0-9-]*-b srp-026g:latest"
check "release: leaves nothing staged (the source either)" nothing_staged
check "release: SRP_MIN_PATCH_REVISION=15" grep -qx "SRP_MIN_PATCH_REVISION=15" "$T/live/.env"

# Refused in the preflight, before anything.
: >"$T/state/admin.log"
expect_refused "release: a patch already published" "already published, these same bytes" \
    "$here/release.sh" --resources "$T/r15/resources.db.patch_15"
expect_refused "release: main's revision not above live" "is not above the live exe's 15" "$here/release.sh"
echo "crates 3" >"$src/crates/a/src/lib.rs"
commit 16 "revision 16, server code"
mkdir -p "$T/p16/gameplay/items" "$T/r16"
echo "item 16" >"$T/p16/gameplay/items/e.options"
"$bin" pack --dir "$T/p16" --out "$T/r16/resources.db.patch_16" >/dev/null
expect_refused "release: a patch of another revision" "must be called resources.db.patch_16" \
    "$here/release.sh" --resources "$T/r15/resources.db.patch_15"
expect_refused "release: --yes needs --i-know" "goes with --i-know" "$here/release.sh" --yes
touch "$T/ssh-fails-live"
expect_refused "release: says the live host cannot be reached" "ssh to the live host (live through jump) fails" "$here/release.sh"
rm "$T/ssh-fails-live"
echo "{}" >"$T/state/maint-planned"
python3 -c 'import time; print(time.time() + 600)' >"$T/state/maint"
expect_refused "release: refuses when a maintenance is planned already" "a maintenance is planned already" "$here/release.sh"
rm "$T/state/maint" "$T/state/maint-planned"
check "release: the refusals planned nothing" test ! -s "$T/state/admin.log"

# The dry run.
before=$(tree)
published_before=$(ls "$T/patches")
run quiet --dry-run --resources "$T/r16/resources.db.patch_16" --minutes 2 || fail "release.sh --dry-run: $out"
check "release --dry-run: no file changes on the live host" test "$(tree)" = "$before"
check "release --dry-run: no patch is published" test "$(ls "$T/patches")" = "$published_before"
check "release --dry-run: plans no maintenance" test ! -s "$T/state/admin.log"
check "release --dry-run: runs no docker" bash -c "! grep -q '^docker' $T/log"
check "release --dry-run: shows the POST, not its secrets" has "$out" "POST http://127.0.0.1:$site_port/<SRP_ADMIN_PATH> token=<SRP_ADMIN_TOKEN> tab=maint minutes=2"
check "release --dry-run: shows the swap and the restart" has "$out" "docker compose up -d --no-deps --force-recreate srp26"
check "release --dry-run: shows the build" has "$out" "build srp26"
check "release --dry-run: says so" has "$out" "dry run: nothing was planned, uploaded or changed"

# A no at the first question: nothing at all.
run no --minutes 1 || true
check "release: a no at the first question plans nothing" test ! -s "$T/state/admin.log"
check "release: ... and says so" has "$out" "stopped: not confirmed (nothing was done)"
check "release: ... and changes nothing" test "$(tree)" = "$before"

# A failure before the swap: the build of the image.
touch "$T/build-fails"
run yes --minutes 2 --label c || true
rm "$T/build-fails"
check "release: a failed build stops before the swap" has "$out" "STOPPED before the swap"
check "release: ... cancels the maintenance" test "$(admin_log)" = "plan 2 cancel"
check "release: ... removes the staged files" nothing_staged
check "release: ... puts the image tag back" grep -q "docker tag srp-026g:before-[0-9-]*-c srp-026g:latest" "$T/log"
check "release: ... leaves the live files as they were" test "$(tree)" = "$before"
check "release: ... restarts nothing" bash -c "! grep -q ' up ' $T/log"

# A no at the second question.
: >"$T/state/admin.log"
run y-then-n --minutes 1 --label g || true
check "release: a no at the swap cancels the maintenance" test "$(admin_log)" = "plan 1 cancel"
check "release: ... swaps nothing" has "$out" "nothing was swapped"
check "release: ... removes the staged files" nothing_staged
check "release: ... leaves the live files as they were" test "$(tree)" = "$before"

# A match that does not end.
: >"$T/state/admin.log"
echo 1000 >"$T/state/busy"
SKHID_MATCH_WAIT_MINUTES=2 run yes --minutes 1 --label e || true
check "release: a match still running stops it" has "$out" "match(es) still running 2 min after the planned time"
check "release: ... cancels the maintenance" test "$(admin_log)" = "plan 1 cancel"
check "release: ... removes the staged files" nothing_staged
check "release: ... leaves the live files as they were" test "$(tree)" = "$before"

# A match that ends a little after the planned time.
: >"$T/state/admin.log"
echo 3 >"$T/state/busy"
run yes --resources "$T/r16/resources.db.patch_16" --minutes 1 --label d || fail "release.sh with a match: $out"
rm -f "$T/state/busy"
check "release: waits while a match runs" has "$out" "1 match(es) still running; waiting"
check "release: ... then releases" cmp -s "$T/work/r16.zip" "$T/live/download/Survarium-0.26g-srp-files.zip"
check "release: ... with the new patch" offers patch_16
check "release: ... without cancelling" test "$(admin_log)" = "plan 1"

# No secret in any output or command line.
check "release: the admin token is in no output" bash -c "! grep -qF -- '$token' $T/outputs"
check "release: the admin path is in no output" bash -c "! grep -qF -- '$admin_path' $T/outputs"
check "release: neither is in a command line of ssh, curl or docker" bash -c "! grep -qF -e '$token' -e '$admin_path' $T/args $T/log"
check "release: the admin page was asked through curl's stdin" grep -q "^curl .* -K -$" "$T/args"

echo
if ((failures)); then
    echo "$failures FAILED"
    exit 1
fi
echo "all passed"
