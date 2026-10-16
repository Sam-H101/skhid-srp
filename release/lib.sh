# shellcheck shell=bash
# What the release scripts share. Sourced, never run. How they are used:
# .claude/skills/skhid-release/SKILL.md; their tests: release/test.sh.
set -euo pipefail
shopt -s inherit_errexit

PROG=${0##*/}
# The checkout these scripts are in.
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DRY_RUN=${DRY_RUN:-0}

# The two files of a release in <target dir>/download.
ZIP_NAME=Survarium-0.26g-srp-files.zip
DATA_NAME=server-game-data.db
# The stock 0.26g resources.db (build 2777).
# shellcheck disable=SC2034 # used by the scripts that source this
STOCK_DB_SHA256=c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c
# The players' exe in the ZIP.
ZIP_EXE=game/binaries/x86/survarium_skhid.exe

die() {
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit 1
}

say() { printf '%s\n' "$*"; }

# `--help`: the comment block at the top of the script, without the `# `.
usage() {
    sed -n '2,/^[^#]/{/^#/{s/^# \{0,1\}//;p}}' "$0"
    exit "${1:-0}"
}

need_cmds() {
    local cmd
    for cmd in "$@"; do
        command -v "$cmd" >/dev/null || die "$cmd is needed and not found"
    done
}

# Loads private/release/release.env of the main checkout (or the file
# SKHID_RELEASE_ENV names), once, and checks the settings given are set. The
# file is the owner's; nothing of it is printed.
RELEASE_ENV=
load_env() {
    if [[ -z $RELEASE_ENV ]]; then
        local env=${SKHID_RELEASE_ENV:-}
        if [[ -z $env ]]; then
            local common
            common=$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir) ||
                die "$ROOT is not in a git checkout"
            env=$(dirname "$common")/private/release/release.env
        fi
        [[ -f $env ]] || die "$env is missing: copy .claude/skills/skhid-release/release.env.example there and fill it in"
        # shellcheck disable=SC1090
        . "$env"
        RELEASE_ENV=$env
    fi
    local name
    for name in "$@"; do
        [[ -n ${!name:-} ]] || die "$name is not set in $RELEASE_ENV"
    done
}

# A word quoted for a remote shell.
q() { printf '%q' "$1"; }

sha256_of() { sha256sum <"$1" | cut -d' ' -f1; }

size_of() { stat -c %s "$1"; }

# `resources.db.patch_<n>`, n from 1 to 255 as the game writes it (no
# leading zero): the number, or nothing.
patch_number() {
    local name=$1
    [[ $name =~ ^resources\.db\.patch_([1-9][0-9]{0,2})$ ]] || return 1
    ((BASH_REMATCH[1] <= 255)) || return 1
    printf '%s\n' "${BASH_REMATCH[1]}"
}

is_revision() { [[ $1 =~ ^[1-9][0-9]{0,2}$ ]] && (($1 <= 255)); }

# GET <url> into <file>: the HTTP status (000 when there was no answer).
http_get() {
    curl -sS --max-time "${HTTP_TIMEOUT:-600}" -o "$2" -w '%{http_code}' "$1" || true
}

# Our srp-game-data: $SRP_GAME_DATA_BIN, else built from this checkout.
game_data_bin() {
    if [[ -n ${SRP_GAME_DATA_BIN:-} ]]; then
        printf '%s\n' "$SRP_GAME_DATA_BIN"
        return
    fi
    say "building srp-game-data (srp-026g, nix develop) ..." >&2
    (cd "$ROOT" && nix develop ./srp-026g -c bash -c \
        'cd srp-026g && cargo build --quiet --release -p resources-db --bin srp-game-data') >&2 ||
        die "srp-game-data did not build"
    printf '%s\n' "$ROOT/srp-026g/target/release/srp-game-data"
}

# python3, or nixpkgs' when the system has none.
python() {
    if command -v python3 >/dev/null; then
        python3 "$@"
    else
        nix shell nixpkgs#python3 -c python3 "$@"
    fi
}

# `<sha256> <name>` of every file entry of a ZIP, in name order. (Python's
# zipfile, not unzip: an unzip may be sandboxed to its working folder.)
zip_hashes() {
    python -I -c '
import hashlib, sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    for info in sorted(z.infolist(), key=lambda i: i.filename):
        if not info.is_dir():
            print(hashlib.sha256(z.read(info)).hexdigest(), info.filename)
' "$1"
}

# One entry of a ZIP into a file; fails when the ZIP has none.
zip_extract() {
    python -I -c '
import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z, open(sys.argv[3], "wb") as out:
    out.write(z.read(sys.argv[2]))
' "$1" "$2" "$3" 2>/dev/null
}

# The patch revision of a players' survarium_skhid.exe: the byte of
# `mov byte [esp+27h],<revision>` that patch_026g.py writes at REVISION_VA
# 0x380862 (.text: VA 0x11000 at file 0x400), or `unknown`.
exe_revision() {
    local at=$((0x380862 - 0x11000 + 0x400)) bytes=''
    if [[ -f $1 ]]; then bytes=$(od -An -tx1 -v -j "$at" -N 6 "$1" | tr -d ' \n'); fi
    if [[ $bytes =~ ^c6442427([0-9a-f]{2})90$ ]]; then
        printf '%d\n' "0x${BASH_REMATCH[1]}"
    else
        printf 'unknown\n'
    fi
}

# The revision of the exe in a ZIP (<scratch> holds it), or `unknown`.
zip_revision() {
    if zip_extract "$1" "$ZIP_EXE" "$2"; then exe_revision "$2"; else printf 'unknown\n'; fi
}

# GET <site>/download (the players' ZIP) into <file>; dies unless 200.
fetch_site_zip() {
    local url=${1%/}/download status
    status=$(curl -sSL --max-time 600 -o "$2" -w '%{http_code}' "$url" || true)
    [[ $status == 200 ]] || die "GET $url answered $status, not 200"
}

# `REVISION = <n>` of patch_026g.py at <commit> of the checkout <repo>.
revision_at() {
    git -C "$1" show "$2:srp-026g/client-patches/patch_026g.py" 2>/dev/null |
        sed -n 's/^REVISION = \([0-9][0-9]*\)$/\1/p'
}

# Asks before a write: shows where and the command; anything but y or yes is
# no. Reads the terminal (else stdin, as `yes | ...` would give).
# SKHID_RELEASE_GO_AHEAD=1 (release/release.sh sets it for the scripts it
# runs, after its own y/N) shows the command and goes ahead.
confirm() {
    local where=$1 command=$2 answer=
    if [[ ${SKHID_RELEASE_GO_AHEAD:-} == 1 ]]; then
        printf '\n%s, running (go-ahead given to release.sh):\n  %s\n' "$where" "$command" >&2
        return 0
    fi
    printf '\n%s, about to run:\n  %s\n' "$where" "$command" >&2
    ask_yes
}

# y/N from the terminal, else stdin; true only for y or yes.
ask_yes() {
    local answer=
    if { exec 3</dev/tty; } 2>/dev/null; then
        read -r -p "Go ahead? [y/N] " answer <&3 || true
        exec 3<&-
    else
        printf 'Go ahead? [y/N] ' >&2
        read -r answer || true
        printf '%s\n' "$answer" >&2
    fi
    [[ $answer == y || $answer == yes ]]
}

# One command on the patches server; its stdin is $2 (an upload) or
# nothing.
# shellcheck disable=SC2029 # expanded here on purpose: the command is built quoted
on_patches() { ssh "$SKHID_PATCHES_SSH" "$1" <"${2:-/dev/null}"; }

# Where a release goes, set by use_target:
#   live  $SKHID_LIVE through $SKHID_JUMP (which holds its key), in
#         $SKHID_LIVE_DIR, its site $SKHID_SITE;
#   test  the test server: $SKHID_TEST (through $SKHID_TEST_VIA when that is
#         set), in $SKHID_TEST_DIR, its site $SKHID_TEST_SITE, its compose
#         command $SKHID_TEST_COMPOSE.
TARGET='' TARGET_SSH='' TARGET_VIA='' TARGET_DIR='' TARGET_SITE='' TARGET_COMPOSE='' TARGET_LABEL=''
use_target() {
    case $1 in
    live)
        load_env SKHID_JUMP SKHID_LIVE SKHID_LIVE_DIR SKHID_SITE
        TARGET=live TARGET_SSH=$SKHID_LIVE TARGET_VIA=$SKHID_JUMP TARGET_DIR=$SKHID_LIVE_DIR
        TARGET_SITE=${SKHID_SITE%/} TARGET_COMPOSE='docker compose'
        TARGET_LABEL="On the live host ($SKHID_LIVE through $SKHID_JUMP)"
        ;;
    test)
        load_env SKHID_TEST SKHID_TEST_DIR SKHID_TEST_SITE
        TARGET=test TARGET_SSH=$SKHID_TEST TARGET_VIA=${SKHID_TEST_VIA:-} TARGET_DIR=$SKHID_TEST_DIR
        TARGET_SITE=${SKHID_TEST_SITE%/}
        TARGET_COMPOSE=${SKHID_TEST_COMPOSE:-docker compose -p srptest -f docker-compose.yml --profile tls}
        TARGET_LABEL="On the test server ($SKHID_TEST${TARGET_VIA:+ through $TARGET_VIA})"
        ;;
    *) die "use_target: $1" ;;
    esac
}

# The command the first ssh runs: through the jump host, one more ssh.
target_command() {
    if [[ -n $TARGET_VIA ]]; then printf 'ssh %s %s' "$(q "$TARGET_SSH")" "$(q "$1")"; else printf '%s' "$1"; fi
}
# One command on the target; its stdin is $2 (an upload) or nothing.
# shellcheck disable=SC2029
on_target() { ssh "${TARGET_VIA:-$TARGET_SSH}" "$(target_command "$1")" <"${2:-/dev/null}"; }
on_live() { on_target "$@"; }

# A write: printed only with DRY_RUN=1; else asked for, then run; a no
# stops the script. <where> is `live` (the target) or `patches`; [file] is
# sent as the command's stdin (an upload).
write_on() {
    local where=$1 command=$2 file=${3:-} label shown
    case $where in
    live) label=$TARGET_LABEL ;;
    patches) label="On the patches server ($SKHID_PATCHES_SSH)" ;;
    *) die "write_on: $where" ;;
    esac
    shown=$command${file:+ < $file}
    if ((DRY_RUN)); then
        printf '[dry run] %s:\n  %s\n' "$label" "$shown"
        return 0
    fi
    confirm "$label" "$shown" || die "stopped: not confirmed (nothing more was done)"
    case $where in
    live) on_target "$command" "$file" || die "failed on the $TARGET host: $command" ;;
    patches) on_patches "$command" "$file" || die "failed on the patches server: $command" ;;
    esac
}

# A read on the target: printed with DRY_RUN=1 (then it prints nothing
# else), else run.
read_live() {
    if ((DRY_RUN)); then
        printf '[dry run] read on the %s host:\n  %s\n' "$TARGET" "$1" >&2
        return 0
    fi
    on_target "$1"
}

# Checks a release's two files here, before anything goes anywhere: the
# ZIP's exe says patch revision <n>, the server data reads (srp-game-data
# info) and offers no patch above <n>. Sets ZIP_SHA ZIP_SIZE DATA_SHA,
# OFFERED (`patch_<n> <sha256> <url>` lines) and EXPECTED (/patches as JSON).
# <scratch> is a folder for the exe.
check_release_files() {
    local zip=$1 data=$2 revision=$3 scratch=$4 got bin info key
    got=$(zip_revision "$zip" "$scratch/exe")
    [[ $got == "$revision" ]] || die "$zip: its exe says patch revision $got, not $revision"
    bin=$(game_data_bin)
    info=$("$bin" info "$data") || die "$data: srp-game-data info refused it"
    OFFERED=$(sed -n 's/^  resources\.db\.\(patch_[0-9]*\) [0-9]* bytes sha256 \([0-9a-f]*\) url \(.*\)$/\1 \2 \3/p' <<<"$info")
    while read -r key _; do
        [[ -n $key ]] || continue
        ((${key#patch_} <= revision)) || die "$data offers $key, above revision $revision"
    done <<<"$OFFERED"
    # shellcheck disable=SC2034 # the callers use them
    ZIP_SHA=$(sha256_of "$zip") ZIP_SIZE=$(size_of "$zip") DATA_SHA=$(sha256_of "$data")
    EXPECTED=$(jq -cnS --arg list "$OFFERED" '$list | split("\n") | map(select(length > 0) | split(" ")
        | {key: .[0], value: {sha256: .[1], url: .[2]}}) | from_entries')
}

# What /patches will offer, one line each (or `nothing ({})`).
say_offered() {
    local line
    while read -r line; do
        if [[ -n $line ]]; then say "  $line"; fi
    done <<<"$OFFERED"
    if [[ -z $OFFERED ]]; then say "  nothing ({})"; fi
}

# Uploads <file> to download/<name>.tmp on the target, checks its sha256
# there, chmod 644. Not in place yet: `mv` is the caller's.
stage_upload() {
    local file=$1 name=$2 sha=$3 there D
    D=$(q "$TARGET_DIR")
    write_on live "cat > $D/download/$name.tmp" "$file"
    if ((!DRY_RUN)); then
        there=$(on_target "sha256sum < $D/download/$name.tmp")
        [[ ${there%% *} == "$sha" ]] || die "download/$name.tmp has sha256 ${there%% *} on the host, not $sha; left as .tmp"
        say "download/$name.tmp: sha256 $sha, as here"
    else
        say "[dry run] then: sha256sum < $D/download/$name.tmp must be $sha"
    fi
    write_on live "chmod 644 $D/download/$name.tmp"
}

# The names the current ZIP and server data are archived under, in
# old-downloads/ (<stamp> is UTC YYYYMMDD-HHMM).
archive_names() {
    OLD_ZIP=old-downloads/${ZIP_NAME%.zip}-$1-$2.zip
    OLD_DATA=old-downloads/${DATA_NAME%.db}-$1-$2.db
}
# The command that copies them there (archive_names first).
archive_command() {
    printf '%s' "cd $(q "$TARGET_DIR") && mkdir -p old-downloads && if test -e download/$ZIP_NAME; then cp -p download/$ZIP_NAME $OLD_ZIP; fi && if test -e download/$DATA_NAME; then cp -p download/$DATA_NAME $OLD_DATA; fi && ls -l old-downloads | tail -n 4"
}

# The command that sets SRP_MIN_PATCH_REVISION=<n> in .env: that line only
# (added on a line of its own when missing); prints the line.
min_revision_command() {
    printf '%s' "cd $(q "$TARGET_DIR") && if grep -q '^SRP_MIN_PATCH_REVISION=' .env; then sed -i 's/^SRP_MIN_PATCH_REVISION=.*/SRP_MIN_PATCH_REVISION=$1/' .env; else { test -z \"\$(tail -c 1 .env)\" || echo; echo SRP_MIN_PATCH_REVISION=$1; } >> .env; fi && grep -E '^SRP_MIN_PATCH_REVISION=' .env"
}

# The value of SRP_MIN_PATCH_REVISION in the target's .env, `none` when the
# line is missing; dies when it is there more than once.
current_min_revision() {
    local lines
    lines=$(on_target "cd $(q "$TARGET_DIR") && grep -E '^SRP_MIN_PATCH_REVISION=' .env || true")
    [[ $(grep -c . <<<"$lines" || true) -le 1 ]] || die ".env has SRP_MIN_PATCH_REVISION more than once; fix it by hand first"
    if [[ -z $lines ]]; then say none; else say "${lines#*=}"; fi
}

# The restart: srp26 made anew on what is in place now (--force-recreate:
# even when neither its image nor its settings changed, it reads the new
# server data).
restart_command() {
    printf '%s' "cd $(q "$TARGET_DIR") && $TARGET_COMPOSE up -d --no-deps --force-recreate srp26"
}

# After the restart: $TARGET_SITE/patches must be EXPECTED (it is retried
# while srp starts) and /api/client-files must name the ZIP (sha256, size).
# <scratch> is a folder for the answers.
check_served() {
    local scratch=$1 patches='' status=000 _
    for _ in $(seq "${CHECK_TRIES:-30}"); do
        status=$(http_get "$TARGET_SITE/patches" "$scratch/patches")
        if [[ $status == 200 ]]; then
            patches=$(jq -cS . "$scratch/patches" 2>/dev/null || true)
            [[ -n $patches ]] && break
        fi
        sleep 2
    done
    [[ -n $patches ]] || die "GET $TARGET_SITE/patches gave no list (status $status)"
    [[ $patches == "$EXPECTED" ]] || die "/patches is $patches, not $EXPECTED"
    say "/patches: $patches, as the server data says"
    status=$(http_get "$TARGET_SITE/api/client-files" "$scratch/client-files")
    [[ $status == 200 ]] || die "GET /api/client-files answered $status"
    if ! grep -qx "sha256 $ZIP_SHA" "$scratch/client-files" || ! grep -qx "size $ZIP_SIZE" "$scratch/client-files"; then
        die "/api/client-files is not the new ZIP: $(tr '\n' ' ' <"$scratch/client-files")"
    fi
    say "/api/client-files: the new ZIP (sha256 $ZIP_SHA, $ZIP_SIZE bytes)"
}
