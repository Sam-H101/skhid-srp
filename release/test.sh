#!/usr/bin/env bash
# The release scripts against stand-ins, on this machine only: ssh runs the
# command here (so the jump host, the live host and the patches server are
# folders under a temporary one), docker, caddy and systemctl only log, and
# two small HTTP servers stand for the patches server and the site. Nothing
# is sent anywhere. Every confirmation is answered by a pipe (setsid: no
# terminal to ask).
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
    (cd "$here" && shellcheck -x -s bash lib.sh build-patch upload-patch build-server-db build-zip deploy setup-patches-server test.sh)
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
# ssh <host> <command>: runs the command here.
printf '%s\n' "$1" >>"$TEST_LOG"
exec bash -c "$2"
EOF
for tool in docker caddy systemctl; do
    cat >"$T/bin/$tool" <<EOF
#!/usr/bin/env bash
printf '$tool %s\n' "\$*" >>"\$TEST_LOG"
if [[ $tool == caddy && -e \$TEST_DIR/caddy-fails ]]; then exit 1; fi
EOF
done
chmod +x "$T/bin/"*
export PATH="$T/bin:$PATH" TEST_LOG=$T/log TEST_DIR=$T
: >"$T/log"

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
serve "$T/site" "$T/site.port"
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
check "deploy --dry-run shows the restart" has "$out" "docker compose up -d --no-deps srp26"
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
check "deploy restarts srp26" grep -q "docker compose up -d --no-deps srp26" "$T/log"
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

echo
if ((failures)); then
    echo "$failures FAILED"
    exit 1
fi
echo "all passed"
