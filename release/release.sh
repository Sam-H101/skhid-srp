#!/usr/bin/env bash
# usage: release/release.sh [--resources <resources.db.patch_<rev>>] [--test]
#                           [--server-code auto|yes|no] [--minutes <n>]
#                           [--label <word>] [--dry-run] [--yes --i-know]
#
# A whole release of origin/main in one run. Players get a maintenance
# countdown; everything is prepared while it runs, and at its end it is all
# swapped in at once and srp restarts:
#   1. preflight, nothing players see: ssh to the jump host and through it to
#      the live host, the patches server, nix, $SKHID_GAME, $SKHID_STOCK_DB
#      (its sha256), free disk here and there, srp-game-data, main's patch
#      REVISION above the live exe's, the admin page answering the jump
#      host, no maintenance planned already, no leftover staged files;
#      with --resources, the file is resources.db.patch_<main's REVISION>,
#      srp-game-data reads it, and the patches server does not have it yet;
#      which server code differs between main and the live tree;
#   2. y/N, then plans maintenance in <n> minutes on the admin page,
#      POSTed from $SKHID_ADMIN_FROM (default the jump host; `here`: this
#      machine), an address in the live SRP_ADMIN_IPS (the token and the
#      page's path go to curl on stdin), and sees the countdown on
#      $SKHID_SITE/launcher/places/en;
#   3. while it runs: release/upload-patch (with --resources),
#      release/build-server-db, release/build-zip (--ref: the commit checked
#      in step 1); checks the two files as release/deploy does; stages them
#      on the live host as download/<name>.tmp (sha256 checked there,
#      chmod 644), not in place yet; with server code to build: tags the
#      running image srp-026g:before-<stamp>-<label>, unpacks main's source
#      in .release-src.tmp and runs docker compose build srp26 there;
#   4. at the planned time, once $SKHID_SITE/api/stats says "matches":0
#      (waiting up to $SKHID_MATCH_WAIT_MINUTES more): shows what changes,
#      y/N, then archives the current ZIP and server data (and source) in
#      old-downloads/, moves the staged files into place, sets
#      SRP_MIN_PATCH_REVISION=<rev> (that line only), docker compose up -d
#      --no-deps --force-recreate srp26;
#   5. checks the container, the log's game data line, /patches (exactly the
#      server data's patches) and /api/client-files (the new ZIP), and prints
#      the rollback.
# Stopped before step 4 (a failure, a no, Ctrl-C): the staged files are
# removed, the image tag put back, the maintenance cancelled; the live
# server is as it was (a published patch stays: none is ever deleted).
# Stopped in steps 4-5: it says what was done and prints the rollback.
#
#   --resources <file>  new content: resources.db.patch_<rev>, published first
#   --test              to the test server ($SKHID_TEST, $SKHID_TEST_SITE):
#                       its ZIP (build-zip --test); its code comes from the
#                       workflow "Deploy the test server", not from here
#   --server-code auto  (default) build when main's srp-026g differs from the
#                       live tree; yes: build anyway; no: never
#   --minutes <n>       the countdown, 1-10080 (default $SKHID_MAINT_MINUTES,
#                       else 20)
#   --label <word>      names the archived files and the image tag
#                       (default before-r<rev>)
#   --dry-run           runs the read-only checks of step 1, prints every
#                       other command; plans no maintenance, writes nothing
#   --yes --i-know      answer both y/N questions yes (both flags or neither)
#
# Settings: release.env (.claude/skills/skhid-release/release.env.example).
. "$(dirname "$0")/lib.sh"
here=$(cd "$(dirname "$0")" && pwd)

# For release/test.sh only: another checkout, a stand-in for build-zip, a
# shorter minute, another stock db.
REPO=${SKHID_TEST_REPO:-$ROOT}
BUILD_ZIP=${SKHID_TEST_BUILD_ZIP:-$here/build-zip}
MINUTE=${SKHID_TEST_SECONDS_PER_MINUTE:-60}
STOCK_SHA=${SKHID_TEST_STOCK_SHA256:-$STOCK_DB_SHA256}

resources='' test_server=0 server_code=auto minutes='' label='' yes=0 i_know=0
while (($#)); do
    case $1 in
    -h | --help) usage ;;
    --resources) (($# >= 2)) || usage 1; resources=$2; shift 2 ;;
    --test) test_server=1; shift ;;
    --server-code) (($# >= 2)) || usage 1; server_code=$2; shift 2 ;;
    --minutes) (($# >= 2)) || usage 1; minutes=$2; shift 2 ;;
    --label) (($# >= 2)) || usage 1; label=$2; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    --yes) yes=1; shift ;;
    --i-know) i_know=1; shift ;;
    *) die "unknown argument $1 (--help)" ;;
    esac
done
((yes == i_know)) || die "--yes goes with --i-know (both or neither): without a terminal to ask, this restarts the live server"
[[ $server_code =~ ^(auto|yes|no)$ ]] || die "--server-code is auto, yes or no"
[[ -z $label || $label =~ ^[A-Za-z0-9._-]+$ ]] || die "$label: a label is letters, digits, . _ -"
need_cmds ssh curl jq sha256sum od git tar awk df
load_env SKHID_JUMP SKHID_SITE SKHID_PATCHES_SSH SKHID_PATCHES_DIR SKHID_PATCHES_URL SKHID_STOCK_DB SKHID_GAME SKHID_LOCALIZATION
if ((test_server)); then
    use_target test
    load_env SKHID_TEST_GAME_SERVER
    admin_from=${SKHID_TEST_ADMIN_FROM:-$SKHID_JUMP}
    [[ $server_code != yes ]] || die "--test --server-code yes: the test server's code comes from the workflow \"Deploy the test server\""
    server_code=no
else
    use_target live
    load_env SKHID_GAME_SERVER
    admin_from=${SKHID_ADMIN_FROM:-$SKHID_JUMP}
fi
minutes=${minutes:-${SKHID_MAINT_MINUTES:-20}}
if [[ ! $minutes =~ ^[1-9][0-9]{0,4}$ ]] || ((minutes > 10080)); then die "--minutes: a whole number from 1 to 10080"; fi
match_wait=${SKHID_MATCH_WAIT_MINUTES:-15}
[[ $match_wait =~ ^[0-9]+$ ]] || die "SKHID_MATCH_WAIT_MINUTES: a whole number"
D=$(q "$TARGET_DIR")
SRC_TMP=.release-src.tmp
# What the image is built of at <commit> (the sources of its Dockerfile's
# COPY lines, but --from=), the Dockerfile and the compose file: one path of
# srp-026g a line, those it has.
build_inputs() {
    local path
    {
        printf '%s\n' Dockerfile .dockerignore docker-compose.yml
        git -C "$REPO" show "$1:srp-026g/Dockerfile" | awk 'toupper($1) == "COPY" {
            from = 0; n = 0
            for (i = 2; i <= NF; i++) if ($i ~ /^--from/) from = 1; else if ($i !~ /^--/) word[++n] = $i
            if (!from) for (i = 1; i < n; i++) print word[i]
        }'
    } | sed 's|^\./||; s|/$||' | while read -r path; do
        if [[ $path == . || $path == *[*?]* ]]; then printf '%s\n' '!unsupported'; continue; fi
        if git -C "$REPO" cat-file -e "$1:srp-026g/$path" 2>/dev/null; then printf '%s\n' "$path"; fi
    done | LC_ALL=C sort -u
}

work=$(mktemp -d "${TMPDIR:-/tmp}/skhid-release.XXXXXX")
say "working in $work"

# Where the run is: preflight, planned (maintenance planned, nothing swapped:
# a stop puts things back), swapping (a stop says what was done), finished.
phase=preflight end=0 staged=0 image_tagged=0 old_image=''
done_steps=() rb=''

clock() { date -u -d "@$1" +%H:%M:%S; }
span() { local s=$1; printf '%d:%02d' $((s / 60)) $((s % 60)); }
progress() {
    local s
    if ((end == 0)); then
        say "== $*"
        return
    fi
    s=$((end - $(date +%s)))
    if ((s >= 0)); then
        say "== [$(clock "$(date +%s)") UTC, $(span "$s") to the planned time] $*"
    else
        say "== [$(clock "$(date +%s)") UTC, $(span $((-s))) past the planned time] $*"
    fi
}
pause() { sleep "$(awk -v m="$MINUTE" -v f="$1" 'BEGIN { print m * f }')"; }

# The admin page's path and token, from the target's .env, into ADMIN_PATH
# and ADMIN_TOKEN; never printed. The command shown in `ps` is a grep.
read_admin() {
    local lines line name value
    lines=$(on_target "cd $D && grep -E '^SRP_ADMIN_(PATH|TOKEN)=' .env" || true)
    ADMIN_PATH='' ADMIN_TOKEN=''
    while IFS= read -r line; do
        [[ -n $line ]] || continue
        name=${line%%=*} value=${line#*=}
        value=${value%$'\r'}
        value=${value#"${value%%[![:space:]]*}"}
        value=${value%"${value##*[![:space:]]}"}
        if ((${#value} >= 2)) && [[ $value == \"*\" || $value == \'*\' ]]; then value=${value:1:-1}; fi
        case $name in
        SRP_ADMIN_PATH) [[ -z $ADMIN_PATH ]] || die "SRP_ADMIN_PATH is in the .env more than once"; ADMIN_PATH=$value ;;
        SRP_ADMIN_TOKEN) [[ -z $ADMIN_TOKEN ]] || die "SRP_ADMIN_TOKEN is in the .env more than once"; ADMIN_TOKEN=$value ;;
        esac
    done <<<"$lines"
    ADMIN_PATH=${ADMIN_PATH#/} ADMIN_PATH=${ADMIN_PATH%/}
    [[ -n $ADMIN_PATH && -n $ADMIN_TOKEN ]] || return 1
    [[ $ADMIN_PATH =~ ^[A-Za-z0-9._~/-]+$ ]] || die "SRP_ADMIN_PATH in the .env has characters this script does not send (not shown)"
    [[ $ADMIN_TOKEN != *[\"\\]* ]] || die "SRP_ADMIN_TOKEN in the .env has a quote or a backslash (not shown)"
}

# The admin page, from $admin_from: curl reads its URL (the secret path) and
# the form (the token, then <name=value>...) from a config on stdin, so
# neither is in a command line. No fields: a GET. Prints the answer's body,
# then its HTTP status on a line of its own. The body holds the token:
# never print it.
admin_request() {
    local field
    {
        printf 'url = "%s/%s"\n' "$TARGET_SITE" "$ADMIN_PATH"
        if (($#)); then
            printf 'data-urlencode = "token=%s"\n' "$ADMIN_TOKEN"
            for field; do printf 'data = "%s"\n' "$field"; done
        fi
    } | if [[ $admin_from == here ]]; then
        curl -sS --max-time 30 -w '\n%{http_code}' -K -
    else
        ssh "$admin_from" "curl -sS --max-time 30 -w '\n%{http_code}' -K -"
    fi
}
# The admin page's answer to a form, without the body: what it said.
admin_said() {
    local answer=$1 status body
    status=${answer##*$'\n'} body=${answer%$'\n'*}
    case $body in
    *"Maintenance starts in"*) say planned ;;
    *"Maintenance cancelled."*) say cancelled ;;
    *"No maintenance was planned."*) say none ;;
    *"Wrong token."*) say "wrong token" ;;
    *"Too many wrong tokens"*) say "locked: too many wrong tokens" ;;
    *) say "HTTP $status, no answer of the maintenance tab" ;;
    esac
}
plan_command() {
    local how="ssh $admin_from curl -K - < the form"
    [[ $admin_from != here ]] || how="curl -K - < the form, here"
    say "POST $TARGET_SITE/<SRP_ADMIN_PATH> token=<SRP_ADMIN_TOKEN> tab=maint $1 ($how)"
}

cancel_maintenance() {
    local answer said
    answer=$(admin_request tab=maint cancel=1 lang=en) || true
    said=$(admin_said "$answer")
    case $said in
    cancelled) say "maintenance: cancelled" ;;
    none) say "maintenance: none was planned any more" ;;
    *) say "maintenance: NOT cancelled ($said): cancel it on the admin page (tab Maintenance)" ;;
    esac
}

# The public launcher page: `planned <minutes>`, `down`, `none` or `unknown`.
maintenance_state() {
    local page
    page=$(curl -sS --max-time 20 "$TARGET_SITE/launcher/places/en" 2>/dev/null || true)
    if [[ $page =~ Maintenance\ in\ ([0-9]+)\ min ]]; then
        say "planned ${BASH_REMATCH[1]}"
    elif [[ $page == *"Maintenance: the server is down"* ]]; then
        say down
    elif [[ $page == *"On the server now"* ]]; then
        say none
    else
        say unknown
    fi
}

matches_now() {
    curl -sS --max-time 20 "$TARGET_SITE/api/stats" 2>/dev/null | jq -r '.matches | numbers' 2>/dev/null || true
}

# What a stop does, by phase.
on_exit() {
    local status=$?
    set +e
    trap - INT TERM
    case $phase in
    planned)
        say
        say "STOPPED before the swap; putting the live server back as it was:"
        if ((staged)); then
            if on_target "cd $D && rm -f download/$ZIP_NAME.tmp download/$DATA_NAME.tmp && rm -rf $SRC_TMP $SRC_TMP.tar"; then
                say "staged files: removed"
            else
                say "staged files: NOT removed: remove download/*.tmp and $SRC_TMP* in $TARGET_DIR by hand"
            fi
        fi
        if ((image_tagged)); then
            if on_target "docker tag $(q "$old_image") srp-026g:latest && docker rmi $(q "$old_image") >/dev/null"; then
                say "image: srp-026g:latest is the running one again"
            else
                say "image: NOT put back: docker tag $old_image srp-026g:latest"
            fi
        fi
        cancel_maintenance
        if [[ -n $resources && -n ${published:-} ]]; then
            say "${resources##*/} stays published on the patches server (patches are never deleted); the server does not offer it. Next time: no --resources."
        fi
        ;;
    swapping)
        say
        say "STOPPED IN THE SWAP. Done:"
        if ((${#done_steps[@]})); then printf '  %s\n' "${done_steps[@]}"; else say "  nothing"; fi
        say_rollback
        if [[ ${done_steps[*]} != *"srp26 restarted"* ]]; then
            say "Until srp restarts, the maintenance stays planned (at its time the games close)."
        fi
        ;;
    esac
    if [[ -d $work/zip/src ]]; then git -C "$REPO" worktree remove --force "$work/zip/src" 2>/dev/null; fi
    exit "$status"
}
trap on_exit EXIT
trap 'die "interrupted"' INT TERM

# 1. Preflight.
problems=()
problem() {
    problems+=("$*")
    say "  MISSING  $*"
}
okay() { say "  ok       $*"; }
ssh_check() { ssh -o BatchMode=yes -o ConnectTimeout=20 "$@" </dev/null >/dev/null 2>&1; }
free_mb() { df -Pk "$1" | awk 'NR == 2 { print int($4 / 1024) }'; }

progress "1. preflight (nothing players see)"
via_ok=1 target_ok=0 patches_ok=0
if [[ -n $TARGET_VIA ]]; then
    if ssh_check "$TARGET_VIA" true; then okay "ssh to the jump host ($TARGET_VIA)"; else via_ok=0; problem "ssh to the jump host ($TARGET_VIA) fails (ssh -o BatchMode=yes $TARGET_VIA true)"; fi
fi
if ((via_ok)); then
    if [[ -n $TARGET_VIA ]]; then
        ssh_check "$TARGET_VIA" "ssh -o BatchMode=yes -o ConnectTimeout=20 $(q "$TARGET_SSH") true" && target_ok=1
    else
        ssh_check "$TARGET_SSH" true && target_ok=1
    fi
    if ((target_ok)); then okay "ssh to the $TARGET host ($TARGET_SSH${TARGET_VIA:+ through $TARGET_VIA})"; else problem "ssh to the $TARGET host ($TARGET_SSH${TARGET_VIA:+ through $TARGET_VIA}) fails"; fi
fi
if [[ $admin_from != "$TARGET_VIA" && $admin_from != here ]]; then
    if ssh_check "$admin_from" true; then
        okay "ssh to $admin_from (the admin page is asked from there)"
    else
        problem "ssh to $admin_from (where the admin page is asked from) fails"
    fi
fi
if ssh_check "$SKHID_PATCHES_SSH" true; then patches_ok=1; okay "ssh to the patches server ($SKHID_PATCHES_SSH)"; else problem "ssh to the patches server ($SKHID_PATCHES_SSH) fails"; fi
if command -v nix >/dev/null; then okay "nix"; else problem "nix is not installed here (the ZIP and srp-game-data are built with it)"; fi
command -v python3 >/dev/null || command -v nix >/dev/null || problem "python3 (or nix to get it)"
if [[ -d $SKHID_GAME ]]; then okay "SKHID_GAME: $SKHID_GAME"; else problem "SKHID_GAME: $SKHID_GAME is not a folder (the original 0.26g install)"; fi
if [[ -f $SKHID_STOCK_DB ]]; then
    cache=${XDG_CACHE_HOME:-$HOME/.cache}/skhid-release/stock-db.sha256
    key="$(stat -c '%s %Y' "$SKHID_STOCK_DB") $(readlink -f "$SKHID_STOCK_DB")"
    stock=$(grep -F -- "$key " "$cache" 2>/dev/null | tail -n 1 || true)
    stock=${stock##* }
    if [[ -z $stock ]]; then
        say "  ...      sha256 of $SKHID_STOCK_DB ($(size_of "$SKHID_STOCK_DB") bytes; once, then cached by size and time)"
        stock=$(sha256_of "$SKHID_STOCK_DB")
        { mkdir -p "${cache%/*}" && printf '%s %s\n' "$key" "$stock" >>"$cache"; } 2>/dev/null || true
    fi
    if [[ $stock == "$STOCK_SHA" ]]; then okay "SKHID_STOCK_DB: the stock resources.db (sha256 ${STOCK_SHA:0:12}…)"; else problem "SKHID_STOCK_DB: $SKHID_STOCK_DB has sha256 $stock, not the stock 0.26g one ($STOCK_SHA)"; fi
else
    problem "SKHID_STOCK_DB: $SKHID_STOCK_DB: no such file (the stock 0.26g resources.db)"
fi
need=${SKHID_MIN_FREE_MB:-4096}
have=$(free_mb "$work")
if ((have >= need)); then okay "free here: $have MB in ${work%/*}"; else problem "free here: $have MB in ${work%/*}, $need wanted (SKHID_MIN_FREE_MB)"; fi
if bin=$(game_data_bin 2>"$work/game-data-build.log"); then okay "srp-game-data: $bin"; else bin=''; problem "srp-game-data does not build (log: $work/game-data-build.log)"; fi

# The revision: main's, the live exe's.
main_rev='' main_commit='' live_rev=unknown
if git -C "$REPO" fetch --quiet origin main:refs/remotes/origin/main 2>"$work/fetch.log"; then
    main_commit=$(git -C "$REPO" rev-parse origin/main)
    main_rev=$(revision_at "$REPO" "$main_commit")
    if is_revision "${main_rev:-x}"; then okay "origin/main: $(git -C "$REPO" log -1 --format='%h %s' "$main_commit"), REVISION $main_rev"; else problem "no REVISION in patch_026g.py of origin/main"; main_rev=''; fi
else
    problem "git fetch origin main failed ($(tr '\n' ' ' <"$work/fetch.log"))"
fi
if (fetch_site_zip "$TARGET_SITE" "$work/live.zip") 2>"$work/live-zip.log"; then
    live_rev=$(zip_revision "$work/live.zip" "$work/live.exe")
    if [[ $live_rev == unknown ]]; then
        problem "the exe in $TARGET_SITE/download says no revision this script can read"
    elif [[ -n $main_rev ]] && ((main_rev <= live_rev)); then
        problem "main's REVISION $main_rev is not above the $TARGET exe's $live_rev: merge the bump PR first"
    else
        okay "the $TARGET exe is revision $live_rev, main has $main_rev"
    fi
else
    problem "$(cat "$work/live-zip.log")"
fi
label=${label:-before-r${main_rev:-x}}

# The new patch.
published=''
if [[ -n $resources ]]; then
    name=${resources##*/}
    if [[ -n $main_rev && $name != "resources.db.patch_$main_rev" ]]; then
        problem "--resources: $name must be called resources.db.patch_$main_rev (main's REVISION; the game mounts patch_<n> of revision n)"
    elif [[ ! -f $resources ]]; then
        problem "--resources: $resources: no such file"
    elif [[ $(head -c 13 "$resources") != little-endian ]]; then
        problem "--resources: $resources is not an archive of the game (no \"little-endian\" header)"
    elif [[ -n $bin && -f $SKHID_STOCK_DB ]]; then
        if "$bin" build --db "$SKHID_STOCK_DB" --patch "$resources" --patch-url "$SKHID_PATCHES_URL" \
            --out "$work/patch-check.db" >"$work/patch-check.log" 2>&1; then
            okay "--resources: $name, $(size_of "$resources") bytes, sha256 $(sha256_of "$resources"); srp-game-data reads it"
        else
            problem "--resources: srp-game-data cannot read $resources over the stock db: $(tail -n 3 "$work/patch-check.log" | tr '\n' ' ')"
        fi
        rm -f "$work/patch-check.db"
    fi
    if ((patches_ok)) && [[ -f $resources ]]; then
        there=$(on_patches "cd $(q "$SKHID_PATCHES_DIR") && if test -e $(q "$name"); then sha256sum < $(q "$name"); else echo none; fi" || echo "unknown")
        if [[ $there == none ]]; then
            okay "--resources: not on the patches server yet"
        elif [[ ${there%% *} == "$(sha256_of "$resources")" ]]; then
            problem "--resources: $name is already published, these same bytes: run without --resources (build-server-db takes every published patch)"
        else
            problem "--resources: $name is already published with OTHER bytes (${there%% *}); a published patch is never replaced: make the next revision"
        fi
    fi
fi

# The target: disk, leftovers, settings, the admin page.
old_min=none have_zip=0 have_data=0
if ((target_ok)); then
    have=$(on_target "df -Pk $D | awk 'NR == 2 { print int(\$4 / 1024) }'" || echo 0)
    need=${SKHID_LIVE_MIN_FREE_MB:-2048}
    if ((have >= need)); then okay "free on the $TARGET host: $have MB"; else problem "free on the $TARGET host: $have MB in $TARGET_DIR, $need wanted (SKHID_LIVE_MIN_FREE_MB)"; fi
    left=$(on_target "cd $D && ls -d download/*.tmp $SRC_TMP $SRC_TMP.tar 2>/dev/null || true")
    if [[ -z $left ]]; then okay "no staged files left from before"; else problem "left from an earlier run in $TARGET_DIR: $(tr '\n' ' ' <<<"$left")— remove them by hand (or finish that release)"; fi
    present=$(on_target "cd $D && for f in download/$ZIP_NAME download/$DATA_NAME; do if test -f \$f; then echo \$f; fi; done")
    [[ $present == *"$ZIP_NAME"* ]] && have_zip=1
    [[ $present == *"$DATA_NAME"* ]] && have_data=1
    if old_min=$(current_min_revision 2>"$work/min.log"); then
        okay "SRP_MIN_PATCH_REVISION now: $old_min"
    else
        problem "$(cat "$work/min.log")"
    fi
    proxy=$(on_target "cd $D && grep -E '^SRP_WEB_TRUST_PROXY=' .env | tail -n 1" || true)
    if [[ ${proxy#*=} != on ]]; then
        problem "SRP_WEB_TRUST_PROXY is not on in the $TARGET .env: the admin page would see Caddy's address, not $admin_from's"
    fi
    if read_admin; then
        answer=$(admin_request) || answer=000
        if [[ ${answer##*$'\n'} == 200 ]]; then
            okay "the admin page answers $admin_from"
        else
            problem "the admin page does not answer $admin_from (HTTP ${answer##*$'\n'}): its address must be in SRP_ADMIN_IPS"
        fi
    else
        problem "SRP_ADMIN_PATH and SRP_ADMIN_TOKEN are not both set in the $TARGET .env (the admin page is off)"
    fi
fi

# The site.
state=$(maintenance_state)
case $state in
none) okay "no maintenance planned on $TARGET_SITE" ;;
planned*) problem "a maintenance is planned already on $TARGET_SITE (${state#planned } min): another release? cancel it first" ;;
*) problem "$TARGET_SITE/launcher/places/en: maintenance state $state" ;;
esac
matches=$(matches_now)
if [[ -n $matches ]]; then okay "$TARGET_SITE/api/stats: $matches match(es) now"; else problem "$TARGET_SITE/api/stats gives no match count"; fi

# The server code: main's srp-026g build inputs against the live tree.
build_code=0 code_report=''
src_tar=$work/srp-026g-src.tar
if [[ $TARGET == live && -n $main_commit ]]; then
    mapfile -t build_paths < <(build_inputs "$main_commit")
    if [[ " ${build_paths[*]} " == *" !unsupported "* ]]; then
        problem "main's Dockerfile copies the whole folder or a pattern: say --server-code yes or no"
    elif git -C "$REPO" archive --format=tar -o "$src_tar" "$main_commit:srp-026g" "${build_paths[@]}" 2>"$work/archive.log"; then
        mkdir -p "$work/src"
        tar -xf "$src_tar" -C "$work/src"
        inputs=$(printf '%q ' "${build_paths[@]}")
        list="find $inputs -type f ! -path '*/__pycache__/*' ! -path '*/target/*' -print0 2>/dev/null | LC_ALL=C sort -z | xargs -0 -r sha256sum"
        (cd "$work/src" && bash -c "$list") >"$work/src-main.sha256"
        if ((target_ok)); then
            on_target "cd $D && $list" >"$work/src-live.sha256" || true
            code_report=$(awk 'NR == FNR { main[$2] = $1; next } { live[$2] = $1 }
                END {
                    for (p in main) if (!(p in live)) print "ADDED     " p; else if (main[p] != live[p]) print "CHANGED   " p
                    for (p in live) if (!(p in main)) print "only live " p
                }' "$work/src-main.sha256" "$work/src-live.sha256" | LC_ALL=C sort -k2)
            code_changes=$(grep -cE '^(ADDED|CHANGED)' <<<"$code_report" || true)
            if ((code_changes)); then
                okay "server code: $code_changes file(s) of main differ from the live tree:"
                grep -E '^(ADDED|CHANGED)' <<<"$code_report" | head -n 30 | sed 's/^/             /'
                ((code_changes <= 30)) || say "             ... ($code_changes in all)"
            else
                okay "server code: the live tree is main's"
            fi
            if grep -q '^only live' <<<"$code_report"; then
                say "             (and $(grep -c '^only live' <<<"$code_report") file(s) only in the live tree, left as they are)"
            fi
            case $server_code in
            auto) ((code_changes == 0)) || build_code=1 ;;
            yes) build_code=1 ;;
            no) ((code_changes == 0)) || say "  WARNING  --server-code no: the image is NOT built although the code differs" ;;
            esac
        fi
    else
        problem "git archive of main's srp-026g failed: $(cat "$work/archive.log")"
    fi
elif [[ $TARGET == test ]]; then
    say "  note     the test server's code is main as of the last \"Deploy the test server\" run; run that first if it changed"
fi

if ((${#problems[@]})); then
    say
    say "preflight: ${#problems[@]} problem(s), nothing was done:"
    printf '  - %s\n' "${problems[@]}"
    # A dry run goes on, to show the rest, when it knows what to release.
    ((DRY_RUN)) && [[ -n $main_rev && -n $main_commit ]] || exit 1
    say "dry run: a real run stops here; what it would do after:"
fi
rev=$main_rev
say
say "the release: revision $live_rev -> $rev to the $TARGET server ($TARGET_SITE), label $label"
say "  new content: ${resources:-none}"
say "  server code: $(if ((build_code)); then say "built during the countdown (docker compose build srp26)"; else say "not built"; fi)"
scale=""
((MINUTE == 60)) || scale=" (a minute here is $MINUTE s)"
say "  maintenance: in $minutes min$scale; players see the countdown, Play is locked in its last 30 min"

# 2. Maintenance.
if ((DRY_RUN)); then
    say
    say "[dry run] 2. plan the maintenance, from $admin_from:"
    say "  $(plan_command "minutes=$minutes")"
    say "[dry run] then: $TARGET_SITE/launcher/places/en must say \"Maintenance in $minutes min\""
else
    say
    say "Next: maintenance in $minutes min on $TARGET_SITE. Players see the countdown at once; from here a stop cancels it."
    if ((yes)); then
        say "(--yes --i-know: going ahead)"
    else
        ask_yes || die "stopped: not confirmed (nothing was done)"
    fi
    phase=planned
    start=$(date +%s)
    end=$((start + minutes * MINUTE))
    answer=$(admin_request tab=maint "minutes=$minutes" lang=en) || answer=000
    said=$(admin_said "$answer")
    [[ $said == planned ]] || die "the admin page did not plan the maintenance: $said"
    seen=''
    for _ in 1 2 3 4 5; do
        state=$(maintenance_state)
        if [[ $state == planned* ]]; then seen=${state#planned }; break; fi
        sleep 2
    done
    [[ -n $seen ]] || die "$TARGET_SITE/launcher/places/en does not show the maintenance ($state)"
    progress "2. maintenance planned: the launcher says \"Maintenance in $seen min\"; the swap at $(clock "$end") UTC"
    # The scripts run from here are covered by the y/N above.
    export SKHID_RELEASE_GO_AHEAD=1
fi

# 3. During the countdown.
stamp=$(date -u +%Y%m%d-%H%M)
dry=()
((DRY_RUN)) && dry=(--dry-run)
if [[ -n $resources ]]; then
    progress "3. publishing ${resources##*/}"
    "$here/upload-patch" "${dry[@]}" "$resources" </dev/null || die "upload-patch failed"
    ((DRY_RUN)) || published=1
fi
data=$work/$DATA_NAME
zip=$work/zip/$ZIP_NAME
build_zip=("$BUILD_ZIP" --ref "$main_commit" --work "$work/zip")
((test_server)) && build_zip+=(--test)
if ((DRY_RUN)); then
    say "[dry run] 3. build here: release/build-server-db --out $data"
    say "[dry run]    and: ${build_zip[*]}"
    say "[dry run]    then the checks of release/deploy on both, and:"
    archive_names "$stamp" "$label"
    stage_upload "$data" "$DATA_NAME" "<its sha256>"
    stage_upload "$zip" "$ZIP_NAME" "<its sha256>"
else
    progress "3. building the server data (release/build-server-db)"
    "$here/build-server-db" --out "$data" </dev/null || die "build-server-db failed"
    progress "3. building the ZIP of $(git -C "$REPO" log -1 --format=%h "$main_commit") (release/build-zip)"
    "${build_zip[@]}" </dev/null 2>&1 | tee "$work/build-zip.log" || die "build-zip failed"
    if [[ -d $work/zip/src ]]; then git -C "$REPO" worktree remove --force "$work/zip/src"; fi
    check_release_files "$zip" "$data" "$rev" "$work"
    if [[ -n $resources ]]; then
        grep -qx "patch_$rev $(sha256_of "$resources") .*" <<<"$OFFERED" ||
            die "$data does not offer patch_$rev with the sha256 of $resources"
    fi
    say "ZIP: $ZIP_SIZE bytes, sha256 $ZIP_SHA, revision $rev; server data: sha256 $DATA_SHA; /patches will offer:"
    say_offered
    progress "3. staging on the $TARGET host (as .tmp, not in place)"
    staged=1
    stage_upload "$data" "$DATA_NAME" "$DATA_SHA"
    stage_upload "$zip" "$ZIP_NAME" "$ZIP_SHA"
fi
if ((build_code)); then
    old_image=srp-026g:before-$stamp-$label
    progress "3. the server code: building the image from main's source"
    staged=1
    write_on live "cat > $D/$SRC_TMP.tar" "$src_tar"
    if ((!DRY_RUN)); then
        there=$(on_target "sha256sum < $D/$SRC_TMP.tar")
        [[ ${there%% *} == "$(sha256_of "$src_tar")" ]] || die "$SRC_TMP.tar has sha256 ${there%% *} on the host, not the one here"
    fi
    write_on live "cd $D && mkdir $SRC_TMP && tar -xf $SRC_TMP.tar -C $SRC_TMP && docker tag srp-026g:latest $old_image"
    ((DRY_RUN)) || image_tagged=1
    write_on live "cd $D && docker compose --env-file $D/.env -f $SRC_TMP/docker-compose.yml build srp26"
fi
if ((!DRY_RUN)); then progress "3. staged"; fi

# 4. At the planned time, with no match running: the swap.
if ((DRY_RUN)); then
    say "[dry run] 4. wait for the planned time, the launcher page saying the server is down, and \"matches\":0 on $TARGET_SITE/api/stats (up to $match_wait min more), then y/N and:"
else
    while (($(date +%s) < end)); do
        progress "4. waiting for the planned time ($(clock "$end") UTC)"
        left=$((end - $(date +%s)))
        if ((left > MINUTE)); then pause 1; elif ((left > 0)); then sleep "$left"; fi
    done
    limit=$(($(date +%s) + 5 * MINUTE))
    while :; do
        state=$(maintenance_state)
        [[ $state == down ]] && break
        [[ $state != none ]] || die "the maintenance is no longer planned on $TARGET_SITE (cancelled on the admin page, or srp restarted)"
        (($(date +%s) < limit)) || die "the launcher page still says \"$state\" 5 min after the planned time (the clocks?)"
        pause 0.25
    done
    limit=$(($(date +%s) + match_wait * MINUTE))
    while :; do
        matches=$(matches_now)
        [[ $matches == 0 ]] && break
        (($(date +%s) < limit)) || die "${matches:-an unknown number of} match(es) still running $match_wait min after the planned time (SKHID_MATCH_WAIT_MINUTES): nothing was swapped"
        progress "4. ${matches:-? (no answer from /api/stats)} match(es) still running; waiting (until $(clock "$limit") UTC at most)"
        pause 0.25
    done
    progress "4. no match is running"

    # What changes.
    say
    say "On the $TARGET host now:"
    on_target "cd $D && ls -l download/$ZIP_NAME download/$DATA_NAME 2>&1; sha256sum download/$ZIP_NAME download/$DATA_NAME 2>&1; true" | sed 's/^/  /'
    say "It becomes:"
    say "  download/$ZIP_NAME: $ZIP_SIZE bytes, sha256 $ZIP_SHA (revision $live_rev -> $rev)"
    say "  download/$DATA_NAME: sha256 $DATA_SHA"
    say "  .env: SRP_MIN_PATCH_REVISION $old_min -> $rev"
    say "  /patches: $(curl -sS --max-time 20 "$TARGET_SITE/patches" 2>/dev/null | jq -cS . 2>/dev/null || say '(no list)') -> $EXPECTED"
    if ((build_code)); then say "  the image: main's source, built (the running one is tagged $old_image)"; fi
    say "  the ZIP's entries, against the live ZIP:"
    grep -E '^  (CHANGED|ADDED|REMOVED) |^updater_skhid.exe' "$work/build-zip.log" | sed 's/^ */    /' || say "    (no entry changed)"
    say "Next: archive, move into place, set the revision, restart srp26 (every lobby link drops)."
    if ((yes)); then
        say "(--yes --i-know: going ahead)"
    else
        ask_yes || die "stopped: not confirmed (nothing was swapped)"
    fi
fi

archive_names "$stamp" "$label"
old_src=old-downloads/srp-026g-src-$stamp-$label.tar.gz
rb="cd $D"
((have_zip)) && rb+=" && cp -p $OLD_ZIP download/$ZIP_NAME"
((have_data)) && rb+=" && cp -p $OLD_DATA download/$DATA_NAME"
if [[ $old_min == none ]]; then
    rb+=" && sed -i '/^SRP_MIN_PATCH_REVISION=/d' .env"
else
    rb+=" && sed -i 's/^SRP_MIN_PATCH_REVISION=.*/SRP_MIN_PATCH_REVISION=$old_min/' .env"
fi
((build_code)) && rb+=" && tar -xzf $old_src && docker tag $old_image srp-026g:latest"
rb+=" && $TARGET_COMPOSE up -d --no-deps --force-recreate srp26"
say_rollback() {
    say "Rollback (puts back what was archived and the setting, and restarts), on the $TARGET host ($TARGET_SSH${TARGET_VIA:+, through $TARGET_VIA}):"
    say "  $rb"
}

((DRY_RUN)) || phase=swapping
progress "4. the swap"
archive=$(archive_command)
if ((build_code)); then
    archive+=" && tar -czf $old_src \$(for p in $inputs; do if test -e \$p; then echo \$p; fi; done)"
fi
write_on live "$archive"
archived=()
((have_zip)) && archived+=("$OLD_ZIP")
((have_data)) && archived+=("$OLD_DATA")
((build_code)) && archived+=("$old_src")
done_steps+=("archived in $TARGET_DIR: ${archived[*]:-nothing (no ZIP or server data before)}")
write_on live "cd $D/download && mv $DATA_NAME.tmp $DATA_NAME && mv $ZIP_NAME.tmp $ZIP_NAME"
done_steps+=("moved into place: download/$DATA_NAME, download/$ZIP_NAME")
if ((build_code)); then
    write_on live "cd $D && tar -xf $SRC_TMP.tar && rm -rf $SRC_TMP $SRC_TMP.tar"
    done_steps+=("main's source unpacked into $TARGET_DIR")
fi
write_on live "$(min_revision_command "$rev")"
done_steps+=("SRP_MIN_PATCH_REVISION=$rev in .env (was $old_min)")
write_on live "$(restart_command)"
done_steps+=("srp26 restarted")

# 5. The checks.
if ((DRY_RUN)); then
    say "[dry run] 5. then: srp26 running, its log's \"game data:\" line, GET $TARGET_SITE/patches is exactly the server data's list, GET $TARGET_SITE/api/client-files names the new ZIP"
    say "[dry run] after it:"
    say_rollback
    say
    say "dry run: nothing was planned, uploaded or changed"
    if ((${#problems[@]})); then
        say "dry run: preflight found ${#problems[@]} problem(s) (above): a real run would stop before planning the maintenance"
        exit 1
    fi
    exit 0
fi
progress "5. checks"
container=''
for _ in $(seq 45); do
    container=$(on_target "cd $D && id=\$($TARGET_COMPOSE ps -q srp26) && test -n \"\$id\" && docker inspect -f '{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}} {{.State.StartedAt}}' \$id" || true)
    [[ $container == "running healthy "* || $container == "running none "* ]] && break
    sleep 2
done
[[ $container == "running healthy "* || $container == "running none "* ]] || die "srp26 is not up: ${container:-no container}"
say "srp26: $container"
line=$(on_target "cd $D && $TARGET_COMPOSE logs --no-log-prefix --tail=500 srp26 2>&1 | grep 'game data:' | tail -n 1" || true)
[[ -n $line ]] || die "no \"game data:\" line in srp26's log"
say "log: $line"
count=$(grep -c . <<<"$OFFERED" || true)
[[ $line == *"; $count patch(es) offered at /patches"* ]] || die "the log's game data line is not of the new server data ($count patch(es) expected)"
check_served "$work"
phase=finished
say
((build_code)) && archived+=("image $old_image")
say "released revision $rev to the $TARGET server. Archived in $TARGET_DIR: ${archived[*]:-nothing}"
say_rollback
say "Kept here: $zip, $data (their hashes go in the changelog PR)."
say "Now: a game updates, downloads the patches, signs in; an old game is refused."
