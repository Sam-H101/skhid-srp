# shellcheck shell=bash
# What the release scripts share. Sourced, never run. How they are used:
# .claude/skills/skhid-release/SKILL.md; their tests: release/test.sh.
set -euo pipefail
shopt -s inherit_errexit

PROG=${0##*/}
# The checkout these scripts are in.
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DRY_RUN=${DRY_RUN:-0}

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
# SKHID_RELEASE_ENV names) and checks the settings given are set. The file
# is the owner's; nothing of it is printed.
load_env() {
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
    local name
    for name in "$@"; do
        [[ -n ${!name:-} ]] || die "$name is not set in $env"
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

# Asks before a write: shows where and the command; anything but y or yes is
# no. Reads the terminal (else stdin, as `yes | ...` would give).
confirm() {
    local where=$1 command=$2 answer=
    printf '\n%s, about to run:\n  %s\n' "$where" "$command" >&2
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

# One command on the patches server; its stdin is $3 (an upload) or
# nothing.
# shellcheck disable=SC2029 # expanded here on purpose: the command is built quoted
on_patches() { ssh "$SKHID_PATCHES_SSH" "$1" <"${2:-/dev/null}"; }

# One command on the live host, through the jump host (which holds its key).
live_command() { printf 'ssh %s %s' "$(q "$SKHID_LIVE")" "$(q "$1")"; }
# shellcheck disable=SC2029
on_live() { ssh "$SKHID_JUMP" "$(live_command "$1")" <"${2:-/dev/null}"; }

# A write: printed only with DRY_RUN=1; else asked for, then run; a no
# stops the script. <where> is `live` or `patches`; [file] is sent as the
# command's stdin (an upload).
write_on() {
    local where=$1 command=$2 file=${3:-} label shown
    case $where in
    live) label="On the live host ($SKHID_LIVE through $SKHID_JUMP)" ;;
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
    live) on_live "$command" "$file" ;;
    patches) on_patches "$command" "$file" ;;
    esac
}

# A read on the live host: printed with DRY_RUN=1 (then it prints nothing
# else), else run.
read_live() {
    if ((DRY_RUN)); then
        printf '[dry run] read on the live host:\n  %s\n' "$1" >&2
        return 0
    fi
    on_live "$1"
}
