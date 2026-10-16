#!/usr/bin/env bash
# usage: release/release.sh [--resources <resources.db.patch_<rev>>] [--test]
#                           [--server-code auto|yes|no]
#                           [--minutes <n> | --no-maintenance]
#                           [--match-wait <minutes>] [--label <word>]
#                           [--ref <branch>] [--dry-run] [--yes --i-know]
#
# A whole release of origin/main in one run. Everything is prepared and
# staged first, with Play open; then players get a maintenance countdown
# that ends 2 minutes after the last running match does (or one of
# --minutes <n>), and at its end it is all swapped in at once and srp is
# made anew. With --no-maintenance there is no countdown: the swap comes as
# soon as no match runs (the downtime is the recreate; lobby players
# reconnect):
#   1. preflight, nothing players see: ssh to the jump host and through it to
#      the live host, the patches server, nix, $SKHID_GAME, $SKHID_STOCK_DB
#      (its sha256), free disk here and there, srp-game-data, main's patch
#      REVISION above the live exe's, the admin page answering the jump
#      host (not with --no-maintenance), no maintenance planned already, no
#      other release running (the lock on the jump host), no release folder
#      left unfinished, download/ on the same file system as the folder;
#      srp26's container, its image ID the one srp-026g:latest names; which
#      of the settings srp takes since #105 the .env sets (names only); the
#      offered-patch list
#      (release/patches.tsv): every row on the patches server with its size
#      and sha256, every URL of it giving that file as the players' updater
#      fetches it (redirects followed, not a web page, the size and sha256:
#      downloaded whole once, then checked by its first bytes; the cache is
#      ~/.cache/skhid-release/patch-urls.checked), none above main's
#      REVISION, and patch_12 (the menus) in it from revision 12 on (or the
#      --resources file); with
#      --resources, the file is resources.db.patch_<main's REVISION>,
#      srp-game-data reads it, and it is neither published nor listed yet;
#      which server code differs between main and the target's tree;
#   2. takes the release lock, y/N; nothing is planned yet;
#   3. at once, players seeing nothing: release/upload-patch (with
#      --resources: publishes it and adds its row to the list),
#      release/build-server-db (of the list), release/build-zip (--ref: the
#      commit checked in step 1); checks the two files as release/deploy
#      does; makes the release folder
#      $TARGET_DIR/release-rev<rev>-<UTC YYYYMMDD-HHMM>[-<label>] and stages
#      both in it (sha256 checked there, chmod 644), with changed.txt; with
#      server code to build: tags the running image (by its ID)
#      srp-026g:before-rev<rev>-<stamp>, unpacks main's source in the
#      folder's src/ and runs docker compose build srp26 there. Once all of
#      it is staged, never before, it plans the maintenance on the admin
#      page, POSTed from the jump host (its address is in the SRP_ADMIN_IPS
#      of the target; the token and the page's path go to curl on stdin),
#      and sees the countdown on $SKHID_SITE/launcher/places/en. Its
#      minutes: ceil((last_match_ends_in + 120) / 60), at least 2, where
#      last_match_ends_in is what $SKHID_SITE/api/stats says (the seconds
#      until the last running match ends at the latest; 0: none runs). From
#      an srp that does not send it yet: 2 with "matches":0; else the most
#      "Time left" (<= <m> min) on the launcher page's matches + 2, when the
#      page gives one for every running match; else a whole match, 15 + 2 =
#      17. With --minutes <n>: <n>. Play locks from the plan on (in the
#      countdown's last 30 min), so no match starts after it. With
#      --no-maintenance: nothing is planned;
#   4. at the planned time, once $SKHID_SITE/api/stats says "matches":0
#      (waiting up to --match-wait more): shows what changes, y/N (no answer
#      in $SKHID_SWAP_ANSWER_MINUTES, default 10: no), then the swap of
#      release/deploy. With --no-maintenance: no planned time, once
#      "matches":0 (waiting up to --match-wait from the end of step 3), y/N,
#      and "matches":0 once more right before the stop (else it waits again,
#      within the same limit). The swap: srp26 stopped; srp.db, the old ZIP
#      and server data (hard links), their sha256, the image ID, the setting
#      (and the source) kept in the folder with its rollback, srp.db copied
#      to the jump host too (live only); the server data (and source) in
#      place, SRP_MIN_PATCH_REVISION=<rev>, srp26 made anew;
#   5. checks the container, the log's game data line and /patches (exactly
#      the list's patches), and only then puts the ZIP in place and checks
#      /api/client-files. A failed check prints srp's log and runs the
#      folder's rollback (srp.db, image, data, ZIP, setting, source: as
#      before). The run's output is the folder's release.log.
# Stopped before the swap (a failure, a no, Ctrl-C): the staged files are
# removed (the folder stays, state stopped), the image tag put back, the
# maintenance cancelled when it was planned (a stop before the plan asks the
# admin page nothing); the server is as it was (a published patch stays,
# with its row in the list: none is ever deleted).
#
#   --resources <file>  new content: resources.db.patch_<rev>, published first
#   --test              to the test environment ($SKHID_TEST in $SKHID_TEST_DIR,
#                       compose project srptest, $SKHID_TEST_SITE), with the
#                       test patches server ($SKHID_TEST_PATCHES_*, never the
#                       production one) and its list release/patches-test.tsv;
#                       its ZIP (build-zip --test); the same steps as live
#   --server-code auto  (default) build when main's srp-026g differs from the
#                       target's tree; yes: build anyway; no: never
#   --minutes <n>       a countdown of <n> minutes, 1-10080, instead of the
#                       one from the matches; planned once staged too
#   --no-maintenance    no countdown, the admin page never asked: prepare at
#                       once, swap as soon as $SKHID_SITE/api/stats says
#                       "matches":0
#   --match-wait <min>  how long to wait for "matches":0, then stop (nothing
#                       swapped): after the planned time (default
#                       $SKHID_MATCH_WAIT_MINUTES, else 15); with
#                       --no-maintenance, after step 3 (default 120)
#   --label <word>      ends the release folder's name
#   --ref <branch>      with --test only: release that pushed branch instead
#                       of main (a branch under review); live is always main
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

ref=main resources='' test_server=0 server_code=auto minutes='' label='' yes=0 i_know=0
no_maint=0 match_wait=''
while (($#)); do
    case $1 in
    -h | --help) usage ;;
    --resources) (($# >= 2)) || usage 1; resources=$2; shift 2 ;;
    --test) test_server=1; shift ;;
    --server-code) (($# >= 2)) || usage 1; server_code=$2; shift 2 ;;
    --minutes) (($# >= 2)) || usage 1; minutes=$2; shift 2 ;;
    --no-maintenance) no_maint=1; shift ;;
    --match-wait) (($# >= 2)) || usage 1; match_wait=$2; shift 2 ;;
    --label) (($# >= 2)) || usage 1; label=$2; shift 2 ;;
    --ref) (($# >= 2)) || usage 1; ref=$2; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    --yes) yes=1; shift ;;
    --i-know) i_know=1; shift ;;
    *) die "unknown argument $1 (--help)" ;;
    esac
done
((yes == i_know)) || die "--yes goes with --i-know (both or neither): without a terminal to ask, this restarts the live server"
[[ $server_code =~ ^(auto|yes|no)$ ]] || die "--server-code is auto, yes or no"
[[ -z $minutes || $no_maint == 0 ]] || die "--minutes is the countdown, and --no-maintenance has none"
[[ $ref == main || $test_server == 1 ]] || die "--ref goes with --test only: a live release is of main"
[[ $ref =~ ^[A-Za-z0-9._/-]+$ && $ref != -* ]] || die "$ref: not a branch name"
[[ -z $label || $label =~ ^[A-Za-z0-9._-]+$ ]] || die "$label: a label is letters, digits, . _ -"
need_cmds ssh curl jq sha256sum od git tar awk df
load_env SKHID_JUMP SKHID_SITE SKHID_STOCK_DB SKHID_GAME SKHID_LOCALIZATION
step_flags=()
if ((test_server)); then
    use_target test
    use_patches test
    load_env SKHID_TEST_GAME_SERVER
    step_flags=(--test)
else
    use_target live
    use_patches live
    load_env SKHID_GAME_SERVER
fi
read_patch_list
if [[ -n $minutes ]]; then
    if [[ ! $minutes =~ ^[1-9][0-9]{0,4}$ ]] || ((minutes > 10080)); then die "--minutes: a whole number from 1 to 10080"; fi
fi
if [[ -n ${SKHID_MAINT_MINUTES:-} ]]; then
    say "note: SKHID_MAINT_MINUTES is not read any more: the countdown ends 2 min after the last match (or --minutes <n>)"
fi
# SKHID_MATCH_WAIT_MINUTES is the wait after a countdown, when most matches
# have ended; without one a match may have just begun (they last up to 15).
if [[ -n $match_wait ]]; then
    [[ $match_wait =~ ^[0-9]{1,5}$ ]] || die "--match-wait: a whole number of minutes"
elif ((no_maint)); then
    match_wait=120
else
    match_wait=${SKHID_MATCH_WAIT_MINUTES:-15}
    [[ $match_wait =~ ^[0-9]{1,5}$ ]] || die "SKHID_MATCH_WAIT_MINUTES: a whole number"
fi
answer_wait=${SKHID_SWAP_ANSWER_MINUTES:-10}
[[ $answer_wait =~ ^[1-9][0-9]*$ ]] || die "SKHID_SWAP_ANSWER_MINUTES: a whole number from 1"
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
SWAP_WORK=$work
log_to "$work/release.log"
say "working in $work"

# Where the run is: preflight, planned (going ahead, nothing moved: a stop
# puts things back), swapping (a stop rolls back), finished. staged: step 3
# is done (the maintenance is planned only then). maint_planned: the
# maintenance was asked for (a stop cancels it).
phase=preflight start=0 end=0 image_tagged=0 maint_planned=0 staged=0

clock() { date -u -d "@$1" +%H:%M:%S; }
span() { local s=$1; printf '%d:%02d' $((s / 60)) $((s % 60)); }
# The clock: to (or past) the planned time; with --no-maintenance the time
# since the y/N.
progress() {
    local s
    if ((end == 0 && start != 0)); then
        say "== [$(clock "$(date +%s)") UTC, $(span $(($(date +%s) - start))) since the go-ahead] $*"
        return
    fi
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
# A sleep a stop ends at once: bash runs a trap only after a foreground
# command, but `wait` returns to it as the signal comes (the sleep is killed
# on the way out).
NAP_PID=''
nap() {
    sleep "$1" >/dev/null 2>&1 &
    NAP_PID=$!
    wait "$NAP_PID"
    NAP_PID=''
}
pause() { nap "$(awk -v m="$MINUTE" -v f="$1" 'BEGIN { print m * f }')"; }

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

# The admin page, asked from the jump host (the address in SRP_ADMIN_IPS):
# curl there reads its URL (the secret path) and the form (the token, then
# <name=value>...) from a config on stdin, so neither is in a command line.
# No fields: a GET. Prints the answer's body, then its HTTP status on a line
# of its own. The body holds the token: never print it.
admin_request() {
    local field
    {
        printf 'url = "%s/%s"\n' "$TARGET_SITE" "$ADMIN_PATH"
        if (($#)); then
            printf 'data-urlencode = "token=%s"\n' "$ADMIN_TOKEN"
            for field; do printf 'data = "%s"\n' "$field"; done
        fi
    } | ssh "$SKHID_JUMP" "curl -sS --max-time 30 -w '\n%{http_code}' -K -"
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
    say "POST $TARGET_SITE/<SRP_ADMIN_PATH> token=<SRP_ADMIN_TOKEN> tab=maint $1 (ssh $SKHID_JUMP curl -K - < the form)"
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

# The maintenance's minutes into MAINT_MINUTES, and where they come from
# into MAINT_WHY: --minutes <n>; else 2 min after the last running match
# ends at the latest, as /api/stats says (last_match_ends_in, seconds), at
# least 2. From an srp that does not send it: 2 with "matches":0; else the
# launcher page's most "Time left" + 2, when the page gives a time left for
# every running match (it rounds up to whole minutes); else a whole match.
LAST_MATCH_MARGIN=120 WHOLE_MATCH_MINUTES=15
maintenance_minutes() {
    local stats ends matches page cell count=0 most=0 older why
    local re='^<td class=e>&le; ([0-9]+) min</td>$'
    if [[ -n $minutes ]]; then
        MAINT_MINUTES=$minutes MAINT_WHY="--minutes $minutes"
        return 0
    fi
    stats=$(curl -sS --max-time 20 "$TARGET_SITE/api/stats" 2>/dev/null || true)
    ends=$(jq -r '.last_match_ends_in | numbers | ceil' <<<"$stats" 2>/dev/null || true)
    if [[ $ends =~ ^[0-9]{1,9}$ ]]; then
        MAINT_MINUTES=$(((ends + LAST_MATCH_MARGIN + 59) / 60))
        ((MAINT_MINUTES >= 2)) || MAINT_MINUTES=2
        if ((ends)); then
            MAINT_WHY="/api/stats: the last match ends in $(span "$ends") at the latest (last_match_ends_in $ends), then 2 min"
        else
            MAINT_WHY="/api/stats: no match runs (last_match_ends_in 0), then 2 min"
        fi
        return 0
    fi
    matches=$(jq -r '.matches | numbers' <<<"$stats" 2>/dev/null || true)
    older="/api/stats has no last_match_ends_in (an srp from before it)"
    if [[ $matches == 0 ]]; then
        MAINT_MINUTES=2 MAINT_WHY="$older; \"matches\":0: no match runs, then 2 min"
        return 0
    fi
    page=$(curl -sS --max-time 20 "$TARGET_SITE/launcher/places/en" 2>/dev/null || true)
    why=''
    while read -r cell; do
        count=$((count + 1))
        if [[ $cell =~ $re ]]; then
            ((BASH_REMATCH[1] <= most)) || most=${BASH_REMATCH[1]}
        else
            why="a match with no time left yet (still loading)"
        fi
    done < <(grep -o '<td class=e>[^<]*</td>' <<<"$page" || true)
    if [[ -z $why && ! $matches =~ ^[1-9][0-9]*$ ]]; then
        why="no match count"
    elif [[ -z $why ]] && ((count != matches)); then
        why="$count time(s) left for $matches match(es)"
    fi
    if [[ -z $why ]]; then
        MAINT_MINUTES=$((most + 2))
        MAINT_WHY="$older; the launcher page's \"Time left\": $count match(es), the last ending in <= $most min, then 2 min"
    else
        MAINT_MINUTES=$((WHOLE_MATCH_MINUTES + 2))
        MAINT_WHY="$older, and the launcher page's \"Time left\" does not say ($why): a whole match, $WHOLE_MATCH_MINUTES min, then 2 min"
    fi
}

# Once everything is staged: the maintenance planned on the admin page, seen
# on the launcher page; the planned time into end.
plan_maintenance() {
    local answer said state seen='' there _
    # Everything staged, and so on the host: the folder staging, both files
    # in it with their sha256 (new.sha); an image to build was built before
    # (its build is the last of step 3, and a failed one stops the run).
    ((staged)) || die "the maintenance is planned only once everything is staged, and it is not (release.sh is wrong)"
    there=$(on_target "cd $REL_DIR && cat state new.sha" || true)
    if [[ $(head -n 1 <<<"$there") != staging ]] || ! grep -qx "$DATA_SHA  $DATA_NAME" <<<"$there" ||
        ! grep -qx "$ZIP_SHA  $ZIP_NAME" <<<"$there"; then
        die "$TARGET_DIR/$REL is not staged as it must be before the maintenance is planned: $(tr '\n' ' ' <<<"$there")"
    fi
    state=$(maintenance_state)
    case $state in planned* | down) die "the launcher page says \"$state\" before this run planned anything: a maintenance planned meanwhile (the admin page)? Nothing was swapped" ;; esac
    maintenance_minutes
    progress "3. planning the maintenance in $MAINT_MINUTES min ($MAINT_WHY)"
    end=$(($(date +%s) + MAINT_MINUTES * MINUTE))
    maint_planned=1
    answer=$(admin_request tab=maint "minutes=$MAINT_MINUTES" lang=en) || answer=000
    said=$(admin_said "$answer")
    [[ $said == planned ]] || die "the admin page did not plan the maintenance: $said"
    for _ in 1 2 3 4 5; do
        state=$(maintenance_state)
        if [[ $state == planned* ]]; then seen=${state#planned }; break; fi
        nap 2
    done
    [[ -n $seen ]] || die "$TARGET_SITE/launcher/places/en does not show the maintenance ($state)"
    progress "3. maintenance planned: the launcher says \"Maintenance in $seen min\"$( ((MAINT_MINUTES > 30)) || printf '; Play is locked'); the swap at $(clock "$end") UTC"
}

# Back once $TARGET_SITE/api/stats says "matches":0; at $limit (a time) a
# stop, "... match(es) still running <when>: nothing was swapped".
wait_no_match() {
    local matches
    while :; do
        matches=$(matches_now)
        [[ $matches == 0 ]] && return 0
        (($(date +%s) < limit)) || die "${matches:-an unknown number of} match(es) still running $1: nothing was swapped"
        progress "4. ${matches:-? (no answer from /api/stats)} match(es) still running; waiting (until $(clock "$limit") UTC at most)"
        pause 0.25
    done
}
# --no-maintenance: swap_prepare runs it right before it stops srp26, so the
# stop follows a "matches":0 as closely as it can (a match may have begun
# during the y/N).
match_check_before_stop() {
    if ((DRY_RUN)); then
        say "[dry run] GET $TARGET_SITE/api/stats: \"matches\":0 once more right before the stop (else wait for it, within the same limit)"
        return 0
    fi
    wait_no_match "$match_wait min after the wait began (--match-wait), at the check before the stop"
    say "\"matches\":0 once more: stopping srp26 now"
}

# What a stop does, by phase.
on_exit() {
    local status=$?
    set +e
    trap - INT TERM
    if [[ -n $NAP_PID ]]; then kill "$NAP_PID" 2>/dev/null; fi
    case $phase in
    planned)
        say
        say "STOPPED before the swap; putting the $TARGET server back as it was:"
        swap_failed
        if [[ -z $SWAP_STATE ]]; then stage_abandon; fi
        if ((image_tagged)); then
            if on_target "docker tag $(q "$RUN_IMAGE") $IMAGE && docker rmi $BEFORE_TAG >/dev/null"; then
                say "image: $IMAGE is the running one again ($RUN_IMAGE)"
            else
                say "image: NOT put back: docker tag $RUN_IMAGE $IMAGE"
            fi
        fi
        if ((maint_planned)); then cancel_maintenance; fi
        if [[ -n $resources && -n ${published:-} ]]; then
            say "${resources##*/} stays published on $PATCHES_LABEL, with its row in $PATCH_LIST (patches are never deleted); the server does not offer it yet. Next time: no --resources."
        fi
        ;;
    swapping) swap_failed ;;
    esac
    case $phase in
    planned | swapping | finished) save_release_log "$work/release.log" ;;
    esac
    lock_drop
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
jump_ok=0 target_ok=0 patches_ok=0
if ssh_check "$SKHID_JUMP" true; then jump_ok=1; okay "ssh to the jump host ($SKHID_JUMP)"; else problem "ssh to the jump host ($SKHID_JUMP) fails (ssh -o BatchMode=yes $SKHID_JUMP true)"; fi
if [[ -n $TARGET_VIA && $TARGET_VIA != "$SKHID_JUMP" ]]; then
    if ssh_check "$TARGET_VIA" true; then okay "ssh to $TARGET_VIA"; else problem "ssh to $TARGET_VIA (the way to the $TARGET host) fails"; fi
fi
if [[ -n $TARGET_VIA ]]; then
    ssh_check "$TARGET_VIA" "ssh -o BatchMode=yes -o ConnectTimeout=20 $(q "$TARGET_SSH") true" && target_ok=1
else
    ssh_check "$TARGET_SSH" true && target_ok=1
fi
if ((target_ok)); then okay "ssh to the $TARGET host ($TARGET_SSH${TARGET_VIA:+ through $TARGET_VIA})"; else problem "ssh to the $TARGET host ($TARGET_SSH${TARGET_VIA:+ through $TARGET_VIA}) fails"; fi
if ssh_check "$PATCHES_SSH" true; then patches_ok=1; okay "ssh to $PATCHES_LABEL"; else problem "ssh to $PATCHES_LABEL fails"; fi
if ((jump_ok)); then
    owner=$(lock_owner || echo "unknown (the lock could not be read)")
    if [[ -z $owner ]]; then okay "no other release to the $TARGET server is running (the lock on the jump host is free)"; else problem "a release to the $TARGET server holds the lock on the jump host: $owner. Gone? ssh $SKHID_JUMP rm -r $(lock_path)"; fi
fi
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
if git -C "$REPO" fetch --quiet origin "$ref:refs/remotes/origin/$ref" 2>"$work/fetch.log"; then
    main_commit=$(git -C "$REPO" rev-parse "origin/$ref")
    main_rev=$(revision_at "$REPO" "$main_commit")
    if is_revision "${main_rev:-x}"; then okay "origin/$ref: $(git -C "$REPO" log -1 --format='%h %s' "$main_commit"), REVISION $main_rev"; else problem "no REVISION in patch_026g.py of origin/$ref"; main_rev=''; fi
else
    problem "git fetch origin $ref failed ($(tr '\n' ' ' <"$work/fetch.log"))"
fi
if (fetch_site_zip "$TARGET_SITE" "$work/live.zip") 2>"$work/live-zip.log"; then
    live_rev=$(zip_revision "$work/live.zip" "$work/live.exe")
    if [[ $live_rev == unknown ]]; then
        problem "the exe in $TARGET_SITE/download says no revision this script can read"
    elif [[ -n $main_rev ]] && ((main_rev <= live_rev && test_server)); then
        okay "the test exe is revision $live_rev, $ref has $main_rev (not above it: fine for the test environment)"
    elif [[ -n $main_rev ]] && ((main_rev <= live_rev)); then
        problem "main's REVISION $main_rev is not above the $TARGET exe's $live_rev: merge the bump PR first"
    else
        okay "the $TARGET exe is revision $live_rev, main has $main_rev"
    fi
else
    problem "$(cat "$work/live-zip.log")"
fi

# The offered patches: the list, each on the patches server and at every
# URL of its row.
published=''
listed=$(grep -c . <<<"$LIST" || true)
if ((patches_ok && listed)); then
    names=$(cut -d' ' -f1 <<<"$LIST" | tr '\n' ' ')
    there=$(on_patches "cd $(q "$PATCHES_DIR") && for f in $names; do if test -f \"\$f\"; then echo \"\$f \$(stat -c %s \"\$f\") \$(sha256sum < \"\$f\" | cut -d' ' -f1)\"; else echo \"\$f missing\"; fi; done" || echo "unreadable")
    bad=0
    urls_checked=0
    while read -r name sha size urls; do
        [[ -n $name ]] || continue
        if ! grep -qx "$name $size $sha" <<<"$there"; then
            bad=1
            problem "$PATCH_LIST: $name ($size bytes, sha256 ${sha:0:12}…) is not so on $PATCHES_LABEL: $(grep "^$name " <<<"$there" || echo "$there")"
            continue
        fi
        read -r -a each <<<"$urls"
        for url in "${each[@]}"; do
            say "  ...      $name at $url"
            if check_patch_url "$url" "$sha" "$size" "$work"; then
                urls_checked=$((urls_checked + 1))
            else
                bad=1
                problem "$PATCH_LIST: $name: $URL_PROBLEM"
            fi
        done
    done <<<"$LIST"
    ((bad)) || okay "the offered-patch list: $listed patch(es), each on $PATCHES_LABEL with its sha256, and its $urls_checked URL(s) giving it"
elif ((patches_ok)); then
    okay "the offered-patch list ($PATCH_LIST) is empty"
fi
if [[ -n $main_rev ]] && ((LIST_TOP > main_rev)); then
    problem "$PATCH_LIST offers resources.db.patch_$LIST_TOP, above main's REVISION $main_rev"
fi
rel=$(realpath -m --relative-to="$REPO" "$PATCH_LIST")
if [[ -n $main_commit && $rel != ../* ]] && ! git -C "$REPO" show "$main_commit:$rel" 2>/dev/null | cmp -s - "$PATCH_LIST"; then
    say "  note     $rel here is not main's: commit its new rows with this release's record"
fi

# The new patch.
if [[ -n $resources ]]; then
    name=${resources##*/}
    if [[ -n $main_rev && $name != "resources.db.patch_$main_rev" ]]; then
        problem "--resources: $name must be called resources.db.patch_$main_rev (main's REVISION; the game mounts patch_<n> of revision n)"
    elif [[ ! -f $resources ]]; then
        problem "--resources: $resources: no such file"
    elif [[ $(head -c 13 "$resources") != little-endian ]]; then
        problem "--resources: $resources is not an archive of the game (no \"little-endian\" header)"
    elif [[ -n $(list_row "$name") ]]; then
        problem "--resources: $name is in $PATCH_LIST already: run without --resources (the list is what is offered)"
    elif [[ -n $bin && -f $SKHID_STOCK_DB ]]; then
        if "$bin" build --db "$SKHID_STOCK_DB" --patch "$resources=$PATCHES_URL/$name" \
            --out "$work/patch-check.db" >"$work/patch-check.log" 2>&1; then
            okay "--resources: $name, $(size_of "$resources") bytes, sha256 $(sha256_of "$resources"); srp-game-data reads it"
        else
            problem "--resources: srp-game-data cannot read $resources over the stock db: $(tail -n 3 "$work/patch-check.log" | tr '\n' ' ')"
        fi
        rm -f "$work/patch-check.db"
    fi
    if ((patches_ok)) && [[ -f $resources ]]; then
        there=$(on_patches "cd $(q "$PATCHES_DIR") && if test -e $(q "$name"); then sha256sum < $(q "$name"); else echo none; fi" || echo "unknown")
        if [[ $there == none ]]; then
            okay "--resources: not on $PATCHES_LABEL yet"
        elif [[ ${there%% *} == "$(sha256_of "$resources")" ]]; then
            problem "--resources: $name is already published, these same bytes, but not in $PATCH_LIST: add its row with release/upload-patch${step_flags[*]:+ ${step_flags[*]}} $resources, then run without --resources"
        else
            problem "--resources: $name is already published with OTHER bytes (${there%% *}); a published patch is never replaced: make the next revision"
        fi
    fi
fi
# The menus: from revision 12 on, patch_12 must be offered.
if [[ -n $main_rev ]] && ((main_rev >= MENUS_PATCH)) && [[ -z $(list_row "resources.db.patch_$MENUS_PATCH") &&
    ${resources##*/} != "resources.db.patch_$MENUS_PATCH" ]]; then
    problem "revision $main_rev needs resources.db.patch_$MENUS_PATCH (the menus) offered, and $PATCH_LIST has no row of it: release it (--resources) first"
fi

# The target: disk, leftovers, settings, the container, the admin page.
if ((target_ok)); then
    have=$(on_target "df -Pk $D | awk 'NR == 2 { print int(\$4 / 1024) }'" || echo 0)
    need=${SKHID_LIVE_MIN_FREE_MB:-2048}
    if ((have >= need)); then okay "free on the $TARGET host: $have MB"; else problem "free on the $TARGET host: $have MB in $TARGET_DIR, $need wanted (SKHID_LIVE_MIN_FREE_MB)"; fi
    left=$(on_target "cd $D && for f in release-rev*/state; do if test -f \"\$f\"; then case \$(cat \"\$f\") in staging | swapping) echo \"\${f%/state} (\$(cat \"\$f\"))\" ;; esac; fi; done; ls -d download/.*.new 2>/dev/null; true")
    if [[ -z $left ]]; then okay "no release folder left unfinished"; else problem "left unfinished in $TARGET_DIR: $(tr '\n' ' ' <<<"$left")— look at it (its release.log), finish or roll it back (release/rollback${step_flags[*]:+ ${step_flags[*]}} <folder>), then set its state to stopped"; fi
    fs=$(on_target "cd $D && stat -L -c %d . download" || true)
    if [[ $(sort -u <<<"$fs" | grep -c .) == 1 ]]; then okay "download/ is on the file system of $TARGET_DIR (a release folder's files go in by rename)"; else problem "download/ is not on the file system of $TARGET_DIR (or not there): the swap hard-links the release folder's files into it"; fi
    blobs=$(missing_original_blobs)
    if [[ -z $blobs ]]; then okay "download/resources/ keeps the original menus' blobs (the updater's restore to stock)"; else problem "download/resources/ on the $TARGET host lacks the original blobs the updater's restore fetches: $(tr '\n' ' ' <<<"$blobs")"; fi
    present=$(on_target "cd $D && for f in download/$ZIP_NAME download/$DATA_NAME; do if test -f \$f; then echo \$f; fi; done")
    [[ $present == *"$ZIP_NAME"* ]] && HAVE_ZIP=1
    [[ $present == *"$DATA_NAME"* ]] && HAVE_DATA=1
    if OLD_MIN=$(current_min_revision 2>"$work/min.log"); then
        okay "SRP_MIN_PATCH_REVISION now: $OLD_MIN"
    else
        OLD_MIN=none
        problem "$(cat "$work/min.log")"
    fi
    if inspect_target; then
        if [[ $LATEST_IMAGE == "$RUN_IMAGE" ]]; then
            okay "srp26: image $RUN_IMAGE (what $IMAGE names), data volume $RUN_VOLUME"
        else
            problem "$IMAGE is $LATEST_IMAGE, but srp26 runs $RUN_IMAGE: a restart would run an image no release made (built by hand?); tag the running one back (docker tag $RUN_IMAGE $IMAGE) or release that one first"
        fi
    else
        problem "srp26's container on the $TARGET host cannot be read (docker compose ps / docker inspect)"
    fi
    compose_text=$(if [[ -n $main_commit ]]; then git -C "$REPO" show "$main_commit:srp-026g/docker-compose.yml" 2>/dev/null; fi || true)
    set_now=$(env_settings_set "$compose_text" || echo "(not readable)")
    if [[ -n $set_now ]]; then say "  note     main's compose file now passes on $set_now, which the $TARGET .env sets (names only): those values take effect at the recreate"; fi
    # The admin page plans the maintenance: --no-maintenance never asks it.
    if ((no_maint)); then
        okay "the admin page: not needed (--no-maintenance)"
    else
        proxy=$(on_target "cd $D && grep -E '^SRP_WEB_TRUST_PROXY=' .env | tail -n 1" || true)
        if [[ ${proxy#*=} != on ]]; then
            problem "SRP_WEB_TRUST_PROXY is not on in the $TARGET .env: the admin page would see Caddy's address, not the jump host's"
        fi
        if read_admin; then
            answer=$(admin_request) || answer=000
            if [[ ${answer##*$'\n'} == 200 ]]; then
                okay "the admin page answers the jump host"
            else
                problem "the admin page does not answer the jump host (HTTP ${answer##*$'\n'}): its address must be in SRP_ADMIN_IPS"
            fi
        else
            problem "SRP_ADMIN_PATH and SRP_ADMIN_TOKEN are not both set in the $TARGET .env (the admin page is off)"
        fi
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
OLD_PATCHES=$(served_patches)
say "  now      /patches: ${OLD_PATCHES:-(no list: a server from before patches)}"

# The server code: main's srp-026g build inputs against the live tree.
build_code=0 code_report=''
src_tar=$work/srp-026g-src.tar
if [[ -n $main_commit ]]; then
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
                grep -E '^(ADDED|CHANGED)' <<<"$code_report" | sed -n '1,30s/^/             /p'
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
release_folder "$rev" "$(date -u +%Y%m%d-%H%M)" "$label"
SWAP_SRC=$build_code SWAP_SRC_INPUTS=${inputs:-}
new_row=''
if [[ -n $resources && -f $resources ]]; then
    new_row="${resources##*/} $(sha256_of "$resources") $(size_of "$resources") $PATCHES_URL/${resources##*/}"
fi
OFFERED=$(offered_of_list "$new_row")
say
say "the release: revision $live_rev -> $rev to the $TARGET server ($TARGET_SITE), its folder $TARGET_DIR/$REL"
say "  new content: ${resources:-none}"
say "  /patches will offer:"
say_offered
say "  server code: $(if ((build_code)); then say "built before the swap (docker compose build srp26)"; else say "not built"; fi)"
scale=""
((MINUTE == 60)) || scale=" (a minute here is $MINUTE s)"
if ((no_maint)); then
    say "  maintenance: none (--no-maintenance)$scale; the swap as soon as $TARGET_SITE/api/stats says \"matches\":0 (up to $match_wait min, --match-wait); lobby players get a ~30 s \"server sync\" and reconnect"
elif [[ -n $minutes ]]; then
    say "  maintenance: in $minutes min (--minutes)$scale, planned once all is staged; players see the countdown, Play is locked in its last 30 min"
else
    say "  maintenance: planned once all is staged, to start 2 min after the last running match ends$scale; players see the countdown, Play is locked from then on"
fi

# 2. The lock and the y/N: nothing is planned yet.
if ((DRY_RUN)); then
    say
    say "[dry run] 2. take the release lock on the jump host ($(lock_path)); nothing planned yet$( ((!no_maint)) || printf ' (--no-maintenance: the admin page is never asked)')"
else
    lock_take
    say
    if ((no_maint)); then
        say "Next, with no maintenance: the patch, the server data, the ZIP and the image prepared now (players see nothing),"
        say "then the swap as soon as $TARGET_SITE/api/stats says \"matches\":0 (up to $match_wait min), after one more y/N."
    else
        say "Next: the patch, the server data, the ZIP and the image prepared and staged now (players see nothing, Play stays open);"
        say "then the maintenance planned on $TARGET_SITE, $(if [[ -n $minutes ]]; then printf 'in %s min (--minutes)' "$minutes"; else printf 'to start 2 min after the last running match ends'; fi),"
        say "and at its time the swap, after one more y/N. From the plan on, a stop cancels it."
    fi
    if ((yes)); then
        say "(--yes --i-know: going ahead)"
    else
        ask_yes || die "stopped: not confirmed (nothing was done)"
    fi
    phase=planned
    start=$(date +%s)
    progress "2. going ahead: preparing now$( ((!no_maint)) || printf ' (--no-maintenance)'), nothing planned yet"
    # The scripts run from here are covered by the y/N above.
    export SKHID_RELEASE_GO_AHEAD=1
fi

# 3. Prepared and staged at once; then the maintenance planned.
dry=()
((DRY_RUN)) && dry=(--dry-run)
if [[ -n $resources ]]; then
    progress "3. publishing ${resources##*/} on $PATCHES_LABEL"
    "$here/upload-patch" "${step_flags[@]}" "${dry[@]}" "$resources" </dev/null || die "upload-patch failed"
    ((DRY_RUN)) || published=1
fi
data=$work/$DATA_NAME
zip=$work/zip/$ZIP_NAME
build_zip=("$BUILD_ZIP" --ref "$main_commit" --work "$work/zip" "${step_flags[@]}")
if ((DRY_RUN)); then
    say "[dry run] 3. build here: release/build-server-db${step_flags[*]:+ ${step_flags[*]}} --out $data"
    say "[dry run]    and: ${build_zip[*]}"
    say "[dry run]    then the checks of release/deploy on both, and the release folder:"
    stage_begin
    write_on live "cat > $REL_DIR/changed.txt" "$work/changed.txt"
    stage_upload "$data" "$DATA_NAME" "<its sha256>"
    stage_upload "$zip" "$ZIP_NAME" "<its sha256>"
else
    progress "3. building the server data (release/build-server-db)"
    "$here/build-server-db" "${step_flags[@]}" --out "$data" </dev/null || die "build-server-db failed"
    progress "3. building the ZIP of $(git -C "$REPO" log -1 --format=%h "$main_commit") (release/build-zip)"
    "${build_zip[@]}" </dev/null 2>&1 | tee "$work/build-zip.log" || die "build-zip failed"
    if [[ -d $work/zip/src ]]; then git -C "$REPO" worktree remove --force "$work/zip/src"; fi
    planned_offer=$OFFERED
    check_release_files "$zip" "$data" "$rev" "$work"
    [[ $OFFERED == "$planned_offer" ]] ||
        die "$data does not offer what the preflight said (the list and the new patch): $(tr '\n' ' ' <<<"$OFFERED")"
    say "ZIP: $ZIP_SIZE bytes, sha256 $ZIP_SHA, revision $rev; server data: sha256 $DATA_SHA; /patches will offer:"
    say_offered
    progress "3. staging in the release folder $TARGET_DIR/$REL (not in place yet)"
    {
        say "release $REL: origin/$ref $(git -C "$REPO" log -1 --format='%h %s' "$main_commit"), revision $live_rev -> $rev"
        say
        say "server source, main's srp-026g against the $TARGET tree$( ((build_code)) || printf ' (not built)'):"
        if [[ -n $code_report ]]; then say "$code_report"; else say "  (the same)"; fi
        say
        say "the players' ZIP, against the one the site hands out now:"
        grep -E '^  (CHANGED|ADDED|REMOVED) |^updater_skhid.exe' "$work/build-zip.log" || say "  (no entry changed)"
    } >"$work/changed.txt"
    stage_begin
    write_on live "cat > $REL_DIR/changed.txt" "$work/changed.txt"
    stage_upload "$data" "$DATA_NAME" "$DATA_SHA"
    stage_upload "$zip" "$ZIP_NAME" "$ZIP_SHA"
fi
if ((build_code)); then
    progress "3. the server code: building the image from main's source"
    write_on live "cat > $REL_DIR/src.tar" "$src_tar"
    if ((!DRY_RUN)); then
        there=$(on_target "sha256sum < $REL_DIR/src.tar")
        [[ ${there%% *} == "$(sha256_of "$src_tar")" ]] || die "$REL/src.tar has sha256 ${there%% *} on the host, not the one here"
    fi
    write_on live "cd $REL_DIR && mkdir src && tar -xf src.tar -C src && docker tag $(q "$RUN_IMAGE") $BEFORE_TAG"
    ((DRY_RUN)) || image_tagged=1
    write_on live "cd $D && docker compose --env-file $D/.env -f $REL/src/docker-compose.yml build srp26"
fi
staged=1
if ((DRY_RUN && !no_maint)); then
    maintenance_minutes
    say "[dry run] 3. once all of it is staged: plan the maintenance in $MAINT_MINUTES min (now: $MAINT_WHY), from the jump host:"
    say "  $(plan_command "minutes=$MAINT_MINUTES")"
    say "[dry run] then: $TARGET_SITE/launcher/places/en must say \"Maintenance in $MAINT_MINUTES min\""
elif ((!DRY_RUN)); then
    progress "3. staged"
    if ((!no_maint)); then plan_maintenance; fi
fi

# 4. At the planned time (--no-maintenance: at once), with no match running:
# the swap.
if ((DRY_RUN && no_maint)); then
    say "[dry run] 4. wait for \"matches\":0 on $TARGET_SITE/api/stats (up to $match_wait min), then y/N (no answer in $answer_wait min: no) and:"
    SWAP_BEFORE_STOP=match_check_before_stop
elif ((DRY_RUN)); then
    say "[dry run] 4. wait for the planned time, the launcher page saying the server is down, and \"matches\":0 on $TARGET_SITE/api/stats (up to $match_wait min more), then y/N (no answer in $answer_wait min: no) and:"
elif ((no_maint)); then
    limit=$(($(date +%s) + match_wait * MINUTE))
    wait_no_match "after $match_wait min of waiting (--match-wait)"
else
    while (($(date +%s) < end)); do
        progress "4. waiting for the planned time ($(clock "$end") UTC)"
        left=$((end - $(date +%s)))
        if ((left > MINUTE)); then pause 1; elif ((left > 0)); then nap "$left"; fi
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
    wait_no_match "$match_wait min after the planned time (--match-wait)"
fi
if ((!DRY_RUN)); then
    progress "4. no match is running"

    # What changes.
    say
    say "On the $TARGET host now:"
    on_target "cd $D && ls -l download/$ZIP_NAME download/$DATA_NAME 2>&1; sha256sum download/$ZIP_NAME download/$DATA_NAME 2>&1; true" | sed 's/^/  /'
    say "It becomes:"
    say "  download/$ZIP_NAME: $ZIP_SIZE bytes, sha256 $ZIP_SHA (revision $live_rev -> $rev)"
    say "  download/$DATA_NAME: sha256 $DATA_SHA"
    say "  .env: SRP_MIN_PATCH_REVISION $OLD_MIN -> $rev"
    say "  /patches: ${OLD_PATCHES:-(no list)} -> $EXPECTED"
    if ((build_code)); then say "  the image: main's source, built (the running one, $RUN_IMAGE, is tagged $BEFORE_TAG)"; fi
    say "  the ZIP's entries, against the live ZIP:"
    grep -E '^  (CHANGED|ADDED|REMOVED) |^updater_skhid.exe' "$work/build-zip.log" | sed 's/^ */    /' || say "    (no entry changed)"
    say "Next: srp26 stopped, the old files kept in the release folder, the server data in place, the revision set,"
    say "srp26 made anew (every lobby link drops; its start copies srp.db to backups/), checked; srp.db's copy kept;"
    say "then the ZIP. A failed check rolls it all back."
    if ((no_maint)); then say "(--no-maintenance: \"matches\":0 once more right before the stop; a match begun meanwhile is waited for, within the limit)"; fi
    if ((yes)); then
        say "(--yes --i-know: going ahead)"
    else
        say "(no answer in $answer_wait min is a no)"
        ask_yes "$((answer_wait * MINUTE))" || die "stopped: not confirmed (nothing was swapped)"
    fi
    if ((no_maint)); then SWAP_BEFORE_STOP=match_check_before_stop; fi
fi

progress "4. the swap"
swap_prepare "$rev"
((DRY_RUN)) || phase=swapping
swap_server "$rev"
progress "5. the server is up on the new data; the ZIP"
swap_zip

# 5. After.
if ((DRY_RUN)); then
    say "[dry run] 5. after the restart: srp26 running, its log's \"game data:\" line, GET $TARGET_SITE/patches is exactly the list's; srp's new backup of srp.db kept in the folder$( [[ $TARGET == live ]] && printf ' and copied to the jump host'); then the ZIP, GET $TARGET_SITE/api/client-files names it"
    say "[dry run] a failed check: srp's log, then sh $TARGET_DIR/$REL/rollback.sh $TARGET_DIR/$REL on the $TARGET host (srp.db, image, files, setting back)"
    say
    say "dry run: nothing was planned, uploaded or changed"
    if ((${#problems[@]})); then
        say "dry run: preflight found ${#problems[@]} problem(s) (above): a real run would stop before the lock"
        exit 1
    fi
    exit 0
fi
phase=finished
say
say "released revision $rev to the $TARGET server. Its folder: $TARGET_DIR/$REL (the files before it, srp.db before it, the image IDs, changed.txt, release.log); the old image is $BEFORE_TAG ($RUN_IMAGE)"
say "Rollback: release/rollback${step_flags[*]:+ ${step_flags[*]}} $REL (on the $TARGET host: sh $TARGET_DIR/$REL/rollback.sh $TARGET_DIR/$REL)"
say "  (srp.db back as it was before the release; --keep-db keeps today's)"
say "Kept here: $zip, $data (their hashes go in the changelog PR)."
if [[ -n ${published:-} ]]; then say "Commit $PATCH_LIST (its new row: ${resources##*/}) with the record."; fi
if ((DB_COPY_FAILED)); then say "WARNING: srp.db's copy is NOT on the jump host (above): copy it by hand."; fi
say "Now: a game updates, downloads the patches, signs in; an old game is refused."
