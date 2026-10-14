"""Builds the srp copy of the 0.26g client (build 2777) from the original
survarium.exe. The full patch includes the startup VRAM-check bypass used
by the Linux launcher.

USAGE (this script never touches the source exe):

    cd srp-026g/client-patches
    python patch_026g.py <game>/binaries/x86/survarium.exe <game>/binaries/x86/survarium_srp.exe
        full patch, upload code in a new PE section (the default)
    python patch_026g.py <survarium.exe> <survarium_srp.exe> --cave=hash-mismatch
        full patch, upload code over game_world_core::on_hash_mismatch
        (the fallback if a client with one more section misbehaves)
    python patch_026g.py <survarium.exe> <survarium_srp.exe> --cert-only
        only the certificate patch (what step 0 installed)
    python patch_026g.py <survarium.exe> <survarium_srp.exe> --no-late-join
        the full patch without the late join: patch revision 4, the exe of
        the players' download before revision 5
    python patch_026g.py <survarium.exe> <survarium_srp.exe> --server=<host>:<port>
        full patch with an embedded default login server; -server= at
        game launch still overrides it, and -replay keeps its original path
    python patch_026g.py <survarium.exe> --check [--cave=...] [--cert-only]
        every check and patch in memory, nothing written
    python selftest_026g.py
        builds every variant, re-parses and disassembles it, writes the
        results to SRP_026G_SCRATCH (never the game folder)

    <survarium.exe> must be the ORIGINAL 0.26g exe (the sha256 is checked);
    the output must be another file. Close the game before replacing an exe
    that is in use. The upload code comes from snapshot_upload_026g.asm:
        nasm -f bin -DCAVE=0xA425000 snapshot_upload_026g.asm -o snapshot_upload_026g.section.bin
        nasm -f bin -DCAVE=0x325A60  snapshot_upload_026g.asm -o snapshot_upload_026g.hash-mismatch.bin
    (both .bin files are checked in; --bin=<file> names another one).
    The late-join code is late_join_026g.section.bin, checked in too; it is
    made by `late_join_026g.py --write-bin`, which needs nasm, a C compiler
    for x86 and GNU ld (--late-join-bin=<file> names another one).

Port of ../srp-021d/client-patches/patch.py; the evidence for every address
is in docs/notes/client_patch.md.

Patches:
- the code-signing certificate check (0x12A10 -> `xor eax,eax; ret`). At
  start the client takes the survarium.com certificate from its own
  Authenticode signature, asks Windows whether its chain is valid now, and
  only then verifies the resources.db file table signature with its key.
  The certificate expired on 2015-06-06, so the chain is "not time valid"
  and every stock 0.26g client stops with "Resources archive is
  corrupted". The patch skips only the chain check; the file table
  signature is still verified, so the signature blob must stay readable
  (see --cave=section below);
- the single-instance check (0x124F9 `jz fail` -> two nops), so several
  clients run on one PC;
- the minimum VRAM check (0x106411 `jae` -> `jmp`), skipping the fatal
  warning when detected video memory is below 320 MiB. The detected size
  and the original fallback for an unknown size are kept;
- optionally, a default login server (--server=host:port at patch time).
  The missing-server error at 0x1EB25B becomes a jump into a 51-byte stub
  appended to the upload code. It initializes the parsed server key and
  repeats the original option read and connection path. Explicit server/replay options
  take their original paths. The lobby address still comes from login;
- the default of -hash_check_interval_in_ms, 100 -> 50 (the imm32 of
  `mov dword [esp+18h],64h` at 0x27CC4F, one byte at 0x27CC53). A stock
  client reports its world hash (0xDF) for every second 50 ms tick, on a
  phase of its own; two clients on opposite phases never name the same
  tick, and the match server then cannot compare their worlds (match.md
  section 14). With 50 every client reports every tick. The key on the
  command line still overrides it;
- the patch revision in the sign-in (sign_in_on_connected 0x3807D0). The
  first login message has a 12-byte version field: "0.26g0" and six zero
  bytes. `mov cl,[9DAE9Eh]` at 0x380862 (it reads the string's own zero,
  which 0x38087B writes into the field once more) becomes
  `mov byte [esp+27h],REVISION` / `nop`: byte 7 of the field says which
  of these patches the exe has. The text stays "0.26g0", so a server that
  does not read the byte is not affected, and an exe without this patch
  says 0. REVISION is raised in the change that makes a full build
  something else for the game; a server can only tell two clients apart
  by it;
- the battery rule of modes 2 and 3 (gather_victory_items_rule_core's
  container event handler 0x313A50): two instructions become nops. When a
  team's count reaches the number of batteries the handler stores the
  completion time (+0x15C) and clears the "containers can be used" byte
  (+0x160); from then on it counts no event (`jnz` at 0x313A62) and no
  container can be used (0x31CB16). Neither field is in the rule's
  serialize/deserialize, so a rollback takes the tenth battery out of the
  container again and leaves both as they were: the replay of the same
  tick cannot put it in, and nobody can use a container for the rest of
  the match (9 of 9 live matches in which a team had nine; match.md
  section 26). Without the two instructions every event is counted and
  the containers stay usable, so the replay ends as the first run did.
  The match's end is the server's (it reads the points);
- a start with a Documents folder the game may not write to (two sites,
  eight bytes, client_patch.md section 3.10). The user folder is the
  folder Survarium in Documents, mounted as `user_data`. When it does not exist
  and cannot be made, the mount fails and has no root; mounter::finish
  0x469F0 still called the "mounted" callback, resources_manager::on_mounted
  0xF9A80, with a null node (access violation at 0xF9A8F). `xor ecx,ecx` /
  `jmp short 46B12h` at 0x46B0B becomes `jmp short 46B36h` / two nops: a
  mount without a root is not announced. When the folder exists but a
  file cannot be made in it, the journal `Survarium_<user>.journal`, made
  at every start with create_always, stopped the game with a fatal
  assertion (0x40141). The default of -record_journal, the text "1" given
  to the option object 0xA9D628 by its initializer at 0x851CC7
  (`mov eax,9DB51Ch`), becomes the exe's own "0" at 0x9DBF0C: no journal
  is made unless -record_journal=1 asks for one;
- the late join (patch revision 5; the default build only, not
  --cave=hash-mismatch): a player enters a match that is being played.
  The game keeps room for twenty players from the first connection (three
  bytes of input masks, twenty histories and statistics records), says so
  in its match connection (0xD8), and on the server's 0xE4 loads the
  newcomer's profile and kits while the match goes on. Seven sites of the
  game's code are rewritten and nine calls or jumps lead into new code on
  the second page of `.srp` (late_join_026g.asm and three C files, built
  into late_join_026g.section.bin); a writable page `.srpd` holds its
  state. Such a game computes a match unlike every game without it: the
  server never puts the two in one match (client_patch.md section 3.11);
- a game that goes on without the focus (patch revision 5, with the late
  join): engine_world::logic_tick 0x1A030 calls game::tick only while the
  window is the active one; otherwise it only sends what is queued, and
  the game stands still until the player comes back (alt-tab, a click on
  another window). Such a game answers nothing the match asks of it: the
  others see it frozen, and no player can be taken into the match while
  it is away, because every game in it has to prepare for the newcomer.
  The `jz` after the active check (0x1A0EE) becomes two nops: the window
  being active no longer matters to the simulation (client_patch.md
  section 3.12);
- three faults of a match that takes players in, found with the first
  late joins of real players (patch revision 6, client_patch.md section
  3.13). The hud's lists of players (the scoreboard, a second list, the
  enemies' markers) are made once, when the match is ready: the late join
  makes them again when a newcomer has its place (one more jump into the
  new code, at 0x23572B). loose_oct_tree::insert 0x39E30 takes a box with
  a number that is no number as a box at the origin (a jump at 0x39E52):
  with such a box the tree doubled its size without end, a node each
  time, until none was left, and read through a null one, which killed a
  game that was given a newcomer. And
  scene::remove_model 0x14FF60, which went on to take a model out of its
  list when the model was not in it (and wrote through what it found
  there), lets go of the model and returns when the model is not in the
  scene (a jump at 0x14FF9B to the function's end): that killed a game
  whose join the server gave up (seen on the test server);
- the weapons' random state at a spawn (patch revision 10, with the late
  join, client_patch.md section 3.18). A weapon's random state (its shots'
  spread) was 1 from its constructor and set by nothing but the load of a
  world, which holds the current kit's player alone: a game that loaded one
  during a match had the other kits' weapons at 1, the others had them
  drawn from, and a respawn into such a kit put it out of step. The
  respawn rule's spawn (0x316DA0) calls into the new code at 0x316EA0,
  after the new kit's player is initialised and before it is put into the
  world: each weapon of its inventory gets a state from its place, kit,
  slot and the spawn's time, the same in every game;
- the option B snapshot upload: two 5-byte jumps (0x27AEA1 in
  on_match_ready, 0x27A505 in the 0xF6 case of on_match_packet_received)
  into the upload code, and the upload code itself:
  --cave=section (default): a new section `.srp` (RVA 0xA415000, VA
    0xA425000, 0x200 bytes of file) after .reloc. The section table gets an
    8th entry (40 of the 216 free header bytes), SizeOfImage grows by one
    page, and the Authenticode blob moves 0x200 bytes towards the end of
    the file, where the security data directory finds it again. Nothing of
    the original code is lost: -debug_hash_mismatch keeps its function
    (but the 0xF6 branch that called it now uploads instead);
  --cave=hash-mismatch: the code overwrites game_world_core::on_hash_mismatch
    (0x325A60, 0x47B bytes, reachable only through the branch the request
    hook replaces); the rest of it becomes int3. No header changes;
- base relocations that overlap rewritten bytes become no-op (type 0)
  entries, so a rebase could not corrupt the new code;
- the PE checksum is recomputed (full patch only; --cert-only keeps the
  file byte-identical to the copy installed in step 0).
The 0.21d mode 3 battery patch is NOT ported: 0.26g serializes the
next-battery time (match_time_vi_spawn_rule_core +0x16C), the 0.21d bug.

Each site is checked against the original bytes first, so a different build
or an already patched file is refused rather than patched twice.
"""

import hashlib
import os
import re
import struct
import sys

IMAGE_BASE = 0x10000
TEXT_VA, TEXT_FILE = 0x11000, 0x400          # same in 0.26g as in 0.21d
TEXT_VSIZE = 0x85E5A3
EXE_SHA256 = "ac33d9937015c2299bf62cc6b127cb308b48a3475b30fff2ddd65479ae3bf153"

# vostok::crypt chain check of the signing certificate: sub esp,2Ch -> xor eax,eax; ret
CERT = (0x12A10, bytes.fromhex("83ec2c"), bytes.fromhex("31c0c3"))

# survarium_or_its_installer_is_already_running: cmp eax,0B7h / jz fail
MUTEX = (0x124F9, bytes.fromhex("7409"), bytes.fromhex("9090"))

# Skip the fatal low-VRAM warning after cmp ebx,14000000h (320 MiB).
# Same one-byte change as play_skhid.sh: file offset 0xF5811, 73 -> EB.
# The branch still targets 0x106422, which returns the detected size.
MEMORY_CHECK = (0x106411, bytes.fromhex("730f"), bytes.fromhex("eb0f"))

# Missing-server fatal error, reached only after both server and replay
# options were absent. Replace the push+call with jmp + nops when embedding.
DEFAULT_SERVER_HOOK = (0x1EB25B, bytes.fromhex("6890a99f00e81b83f2ff"))
DEFAULT_SERVER_OFFSET = 420  # immediately after the snapshot upload code
DEFAULT_SERVER_RETURN = 0x1EAE2D
DEFAULT_SERVER_OPTION = 0xA9C550


# network_client::try_send_all, the one-time read of the option object
# 0xAA19F8 (-hash_check_interval_in_ms) into the static 0xA2B393C:
# mov dword [esp+18h],64h (the default, kept when the key is absent) -> 32h.
# The whole instruction is checked; only the imm32's low byte changes.
HASH_INTERVAL_VA = 0x27CC4F
HASH_INTERVAL = (HASH_INTERVAL_VA, bytes.fromhex("c744241864000000"), bytes.fromhex("c744241832000000"))
HASH_INTERVAL_BYTE_VA = HASH_INTERVAL_VA + 4  # the one byte that differs
HASH_INTERVAL_MS = 50                         # = one tick

# sign_in_on_connected 0x3807D0 builds the 12-byte version field of the first
# login message on the stack at [esp+20h]: eight zero bytes, "0.26" at +0,
# "g0" at +4, the string's own zero at +6 (through cl, read here) and a zero
# at +6 again (`mov [esp+26h],al`, 0x38087B); four more zero bytes follow in
# the message. mov cl,[9DAE9Eh] -> mov byte [esp+27h],REVISION / nop.
# The base relocation of the address (0x380864) is disabled with the rest.
# 0: an exe without this patch (the download of 2026-10-12). 1: the patches
# above and the revision field. 2: adds the optional embedded-server patch.
# 3: adds the battery rule patch below, the first that changes what a match
# computes: a game of revision 3 and an older one part at a team's last
# battery (match.md section 26). 4: adds the two start-up patches below
# (a Documents folder the game may not write to); a match is not affected.
# 5: adds the late join below. A game with it and one without never play
# together: the world they keep and the packets they send differ. With it
# goes the game that goes on without the focus (RUN_UNFOCUSED).
# 6: adds the three faults of a match that takes players in (the hud's
# lists made again, BOX below, REMOVE_MODEL below). A game of 5 in a match
# can still be killed by a newcomer, so a server that takes players into
# matches asks for 6. 5 and 6 compute a match alike and play together.
# 7: a slot's grenades and traps go in front of the list the world is saved
# by, whenever the slot is bound (late_join_026g.asm, bind_slot_reserved),
# and into the states a game rolls back to (late_join_install_026g.c). In 5
# and 6 a player with a grenade or a trap in a kit who was taken into a
# match being played was never let in. The order of the world's game
# objects is another one than 5 and 6 keep: a game of 7 and an older one
# part as soon as two players' grenades are not as they began, so a server
# asks for 7.
# 8: the game's clock is brought to the match clock at 30 % instead of 5 %
# (CLOCK_FACTOR below). Nothing a match computes: 7 and 8 play together,
# and the match server times its corrections by the revision a game says.
# 9: the statistics handler keeps the scoreboard's score of all twenty
# places (LATE_JOIN_PATCHES, start_match). In 5 to 8 a newcomer's score was
# added again at every rollback over its events, and its row showed a
# score of an earlier match until its first. And the hud draws the team
# points while a match counts down (PREGAME_PROGRESS); the original kept
# the last match's until play began. Neither is saved in the world or
# hashed: 7, 8 and 9 play together.
# 10: every spawn gives each weapon of the new kit a random state every game
# works out alike (WEAPON_RANDOM below). In 9 and before a game that loaded
# a world during a match kept the other kits' weapons at the constructor's
# 1, and a respawn into such a kit put it out of step at once. What a match
# computes changes from a match's first spawn: a game of 10 and an older one
# part there, so a server asks for 10.
REVISION = 10
# What a full build without the late join says (--no-late-join, and every
# --cave=hash-mismatch build): the players' download before revision 5.
REVISION_WITHOUT_LATE_JOIN = 4
REVISION_VA, REVISION_ORIGINAL = 0x380862, bytes.fromhex("8a0d9eae9d00")


def revision_site(revision):
    """(address, the original's bytes, `mov byte [esp+27h],revision` / `nop`)"""
    return REVISION_VA, REVISION_ORIGINAL, bytes([0xC6, 0x44, 0x24, 0x27, revision, 0x90])


REVISION_SITE = revision_site(REVISION)

# gather_victory_items_rule_core's container event handler 0x313A50:
# `cmp dword [ecx+15Ch],-1` / `jnz 313B3Bh` (an event after the completion
# is not counted) -> the jnz becomes nops;
GATHER_EVENTS = (0x313A62, bytes.fromhex("0f85d3000000"), b"\x90" * 6)
# `mov [ecx+15Ch],edx` / `mov byte [ecx+160h],0` / `ret 0Ch` (at the
# completion no container can be used any more) -> the mov byte becomes nops.
GATHER_LOCK = (0x313B34, bytes.fromhex("c6816001000000"), b"\x90" * 7)
GATHER = (GATHER_EVENTS, GATHER_LOCK)

# vfs::mounter::finish 0x469F0, the last step of every mount: `mov
# eax,[eax+34h]` (the mount's root) / `test eax,eax` / `jnz 46B0Fh`, then
# for a mount without a root `xor ecx,ecx` / `jmp short 46B12h`, which goes
# on to call file_system.on_mounted(NULL) -> `jmp short 46B36h` (the
# function's end) / two nops. A mount has no root when it failed, as the
# user folder's does when <Documents>\Survarium is not there and cannot be
# made; resources_manager::on_mounted 0xF9A80 reads the node's flags at
# +30h without looking (client_patch.md section 3.10).
MOUNT_NO_ROOT = (0x46B0B, bytes.fromhex("33c9eb03"), bytes.fromhex("eb299090"))
MOUNT_NO_ROOT_END = 0x46B36

# The initializer of the option object -record_journal (0xA9D628):
# `push 0A9D628h` / `mov eax,9DB51Ch` ("1") / `call 0DDFF0h`. The imm32
# becomes the address of the exe's own "0" literal, the one the initializers
# of other options give in the same three instructions (0x84CF92, 0x84E4F2,
# 0x853B02). The journal is made by journaling 0xEB310 when
# the option reads as set (game start, 0x1F69B2); with "0" it does not, and
# -record_journal=1 still makes one. Only the imm32 is checked and
# changed, and its base relocation stays as it is: the new value is an
# address of the image too.
JOURNAL_DEFAULT_VA = 0x851CC7                 # the `mov eax,imm32`
JOURNAL_DEFAULT_ON, JOURNAL_DEFAULT_OFF = 0x9DB51C, 0x9DBF0C
JOURNAL_DEFAULT = (JOURNAL_DEFAULT_VA, b"\xB8" + struct.pack("<I", JOURNAL_DEFAULT_ON),
                   b"\xB8" + struct.pack("<I", JOURNAL_DEFAULT_OFF))
JOURNAL_OPTION = 0xA9D628

# game_world_core::on_hash_mismatch, 0x47B bytes of code + 5 int3 of padding
HASH_MISMATCH_VA, HASH_MISMATCH_SIZE, HASH_MISMATCH_REGION = 0x325A60, 0x47B, 0x480
HASH_MISMATCH_SHA256 = "f757a41458ada034433e7476bb47447a433c7c0a33f5b403cd4168e0688a040c"

# The new section of --cave=section, right after .reloc.
SECTION_NAME = b".srp\0\0\0\0"
SECTION_RVA = 0xA415000                       # = the original SizeOfImage
SECTION_VA = IMAGE_BASE + SECTION_RVA         # 0xA425000, the CAVE of the .section.bin
SECTION_CHARACTERISTICS = 0x60000020          # code, execute, read (as .text)

CAVES = {"section": SECTION_VA, "hash-mismatch": HASH_MISMATCH_VA}

# The late join (client_patch.md section 3.11). Its code is the second page
# of `.srp` and what follows it: 4096 bytes of late_join_026g.asm, whose
# entries are at fixed offsets, then the C code. The first page stays what
# it is without the late join (the upload code, the default-server stub and
# the address), so no address in the code depends on the server given at
# patch time. Its state is one zeroed, writable page of its own, `.srpd`,
# on the page after the code.
LATE_JOIN_OFFSET = 0x1000
LATE_JOIN_VA = SECTION_VA + LATE_JOIN_OFFSET
LATE_JOIN_ASM_SIZE = 0x1000
LATE_JOIN_STATE_NAME = b".srpd\0\0\0"
LATE_JOIN_STATE_SIZE = 0x1000
LATE_JOIN_STATE_CHARACTERISTICS = 0xC0000080   # uninitialized data, read, write

# base_game_statistics_handler (the network client's, +1EE8D8h; vtable
# 0xA0755C) keeps the scoreboard's score of each place, `m_scores[20]` at
# +31F0h, which the world does not save. on_event (0x26DB80, slot +44h, on
# every statistics event) adds the event's points to any place's score;
# recalculate_scores (0x26DC30), which deserialize (0x26DB60, slot +34h)
# runs after every state a game loads, works them out again from the
# statistics, and start_match (0x26D8D0, slot +3Ch) tells the hud all of
# them, both for as many places as the handler's count, +3240h. start_match
# copies that count from the roster of the start (options+577B5h); the late
# join raises the roster and the shared statistics' live count (+31E8h) but
# not this one. So a newcomer's score was never worked out again: every
# rollback over its events added them once more (2.7 to 4 times its score
# in a match of ten, seen live), and its hud cell kept the score a player
# of an earlier match had had in that place until its first. With twenty, every
# place is worked out again at every state loaded, and start_match tells
# the hud a 0 for every place at a match's start. Places with no player
# have zero statistics: a score of 0, told once. The live count beside it
# stays the roster's. `mov al,[edx+577B5h]` -> `mov al,14h`.
STATISTICS_START_MATCH_VA = 0x26D8D0
STATISTICS_COUNT = (0x26D8F3, bytes.fromhex("8a82b5770500"), b"\xb0\x14" + b"\x90" * 4)

# A weapon's random state (weapon_core+5BCh: boost's minstd_rand, x * 48271
# mod 2^31-1, drawn from by its shots' spread alone, 0x2E1D50 and 0x2E2120)
# is 1 from the constructor (0x2DF429), comes out of a world in deserialize
# (0x2E34C1), and is set by nothing else: not at a respawn, which keeps the
# kit's weapon objects. A place has a player object for each of its three
# kits, and a world holds only the current one's (0x3218E0). So a game that
# loaded a world during a match (a newcomer, a resynced game) had every other
# kit's weapons at 1 where the games that played them had drawn from them,
# and a respawn into such a kit put that game out of step at once (seen live:
# match 0x1ce45cbe, six desyncs of its newcomers on the kit timeline); so
# did a rollback over a respawn into another kit, which loads the current
# kit's player alone. The respawn rule's spawn (0x316DA0, slot +5Ch of every
# respawn rule; its tick 0x316820 spawns every player, a match's first spawn
# too, in every game and in every replay of a tick) makes the new kit
# current (0x316DE3), initialises its player (vtable+1Ch: the inventory from
# the profile, a weapon in hand) and puts it into the world (vtable+20h).
# Between the two, at 0x316EA0 (`mov edx,[esi]` / `mov eax,[edx+20h]`, the
# load of that call; esi = the player, ebp = the spawn's time), a call into
# weapon_random_hook (late_join_026g.asm) gives each weapon of the player's
# inventory a state from its place, kit, slot and the spawn's time, in
# 1..2^31-2, then does the two instructions. A weapon is an item whose
# vtable+70h is weapon_core's bind 0x2E30F0, which only the vtables of
# weapon_core and the client's weapon hold.
WEAPON_RANDOM_VA, WEAPON_RANDOM_OFFSET = 0x316EA0, 3712
WEAPON_RANDOM_HOOK = (WEAPON_RANDOM_VA, bytes.fromhex("8b168b4220"), WEAPON_RANDOM_OFFSET, 0xE8)
WEAPON_SPAWN_VA = 0x316DA0                    # the respawn rule's spawn
WEAPON_BIND_VA = 0x2E30F0                     # weapon_core's bind, vtable+70h of its item
WEAPON_RANDOM_FIELD = 0x5BC                   # weapon_core's random state
WEAPON_RANDOM_MODULUS = 0x7FFFFFFF


def weapon_random_state(place, kit, slot, time):
    """The state weapon_random_hook gives a weapon at a spawn."""
    h = ((((place & 0xFF) << 16 | (kit & 0xFF) << 8 | slot) * 0x85EBCA6B)
         ^ (time * 0x9E3779B1)) & 0xFFFFFFFF
    return (h ^ h >> 15) % (WEAPON_RANDOM_MODULUS - 1) + 1

# Rewritten in place: (address, the original's bytes, the new ones). The
# actual roster count stays what resources and the countdown read; only
# what must survive a growing roster takes the fixed capacity.
LATE_JOIN_PATCHES = (
    # base_match_client::set_players_count, both implementations (the
    # journaling vtable 0xA083D8+34h and the live match_client's
    # 0xA08864+34h): the ED/EE input masks have three bytes from the first
    # connection, whatever the count. `mov eax,3` and nops.
    (0x279610, bytes.fromhex("0fb6442404489983e20703c2c1f803fec0"), bytes.fromhex("b803000000") + b"\x90" * 12),
    (0x27A2E0, bytes.fromhex("0fb6442404489983e20703c2c1f803fec0"), bytes.fromhex("b803000000") + b"\x90" * 12),
    # kd_stats_rule: all twenty entries of its array, which the constructor
    # zeroes. `mov dl,[ecx+577B5h]` -> `mov dl,14h`.
    (0x3237B4, bytes.fromhex("8a91b5770500"), b"\xb2\x14" + b"\x90" * 4),
    # base_game_statistics_handler::start_match: all twenty scores
    # (STATISTICS_COUNT above).
    STATISTICS_COUNT,
    # shared_statistics::serialize and deserialize: twenty records, not the
    # live count (which rewards and event handlers go on reading).
    # `cmp [esi+31E0h],bl` / `cmp bl,[esi+31E0h]` -> `cmp bl,14h`.
    (0x2F5ACF, bytes.fromhex("389ee0310000"), b"\x80\xfb\x14" + b"\x90" * 3),
    (0x2F5AF4, bytes.fromhex("3a9ee0310000"), b"\x80\xfb\x14" + b"\x90" * 3),
    (0x2F5BE6, bytes.fromhex("389ee0310000"), b"\x80\xfb\x14" + b"\x90" * 3),
    (0x2F5C03, bytes.fromhex("3a9ee0310000"), b"\x80\xfb\x14" + b"\x90" * 3),
)
# Calls and jumps into the new code: (address, the original's bytes, the
# entry's offset in the code, 0xE8 call or 0xE9 jmp); the rest of the
# original's bytes become nops.
LATE_JOIN_HOOKS = (
    (0x2CBF9B, bytes.fromhex("e860c10000"), 3072, 0xE8),         # a player's animations: register_animations
    (0x2D3880, bytes.fromhex("e89bdf0400"), 0, 0xE8),            # a slot's ids: bind_slot
    (0x2D3906, bytes.fromhex("03f28d8de4e30000"), 96, 0xE9),     # twenty histories: history_capacity
    (0x27A23D, bytes.fromhex("881c388b44241c"), 128, 0xE9),      # 0xD8's capability byte: connect_capability
    (0x27A378, bytes.fromhex("83be641c1f0001"), 1856, 0xE9),     # the 0xE4 case: prepare_hook
    (0x1F2013, bytes.fromhex("e8e8000000"), 2048, 0xE8),         # player's destructor: player_destroyed_hook
    (0x27B517, bytes.fromhex("8b8f401c1f00"), 2336, 0xE9),       # on_match_disconnected: disconnect_hook
    (0x219E60, bytes.fromhex("558bec83e4f8"), 2432, 0xE9),       # network_client's destructor: client_destroyed_hook
    (0x23572B, bytes.fromhex("8b8c2498000000"), 3456, 0xE9),     # the hud's lists of players, made again: lists_hook
    (0x39E52, bytes.fromhex("f30f7e00660fd64704"), 3520, 0xE9),  # a box that is no box, in the tree: box_hook
    WEAPON_RANDOM_HOOK,                                          # a spawn's weapons' random state: weapon_random_hook
)
# on_match_ready's jump (HOOKS[0], which goes to the upload code without
# the late join) goes to ready_hook, which goes on to the upload code.
LATE_JOIN_READY_VA = 0x27AEA1
LATE_JOIN_READY_OFFSET = 1664

# engine_world::logic_tick 0x1A030, after the window was shown
# (engine_world+313h): `mov edx,[ebx+8]` / `mov eax,[edx+30h]` /
# `lea ecx,[ebx+8]` / `call eax` (is_application_active) / `test al,al` /
# `jz 1A113h`. Not active: the branch at 0x1A113 calls the game's
# on_waiting_for_render (it sends what is queued and ticks the lobby link)
# and game::tick is not called (transport.md section 6.1). The `jz`
# becomes two nops; the call stays, so whatever it does is still done.
# In the builds of patch revision 5 only (the late join needs every game
# of a match to answer). (address, the original's bytes, the new ones.)
# vostok::render::scene::remove_model 0x14FF60. It looks the model up in the
# scene's list (0x14FF88); when it is not there (`cmp edi,esi` / `jnz`,
# 0x14FF93) the original writes "remove_model: model is not inserted now"
# and goes on all the same: it takes the model's object out of the tree and
# erases the list's end (0x1501B4), which moves what lies after the list
# and adds one to a reference count through it (access violation at
# 0x15EE8D, live 2026-10-13: a game that had loaded a match and never
# entered it, when the server closed its link). The tree's erase before
# that did not fault, so the model's object was in a tree: LIKELY the model
# is another scene's, which holds it and lets go of it in its own time. The
# first instruction of that branch, `cmp [1F1D60Ch],ebx`, becomes a jump to
# the function's own end (0x1501F2): there it lets go of the model it was
# given (the pointer is passed by value, and the function that takes it
# counts it down: `lock xadd [model+0D0h]`, and 0xE0210 at zero), restores
# the three registers it saved and returns (`ret 8`). A jump past that
# (0x150215) would keep the model for ever. The branch's address has a base
# relocation, which is disabled.
REMOVE_MODEL_VA, REMOVE_MODEL_END = 0x14FF9B, 0x1501F2
REMOVE_MODEL = (REMOVE_MODEL_VA, bytes.fromhex("391d0cd6f101"),
                b"\xE9" + struct.pack("<i", REMOVE_MODEL_END - REMOVE_MODEL_VA - 5) + b"\x90")

# The game's own setting `time_floating_factor`: a console variable (the
# object at 0xA9D0D0, registered by 0x851330: its value is the float at
# 0xA799C8 in .data, from 0.001 to 0.5, the two floats the registration
# reads at 0x91A8F0 and 0x927A44). The `game` constructor copies the value
# into the game clock's timer once (0x1E9848: `movss xmm0,[0A799C8h]`, then
# timer+28h at 0x1E986D); nothing else reads it, and it is not among the
# settings the game keeps in user.cfg. When the server tells a game how far
# its clock is off (0xE7 -> floating_timer::set_offset 0x1EE9E0), the clock
# runs at 1 + the factor, or 1 - it, until the whole is made up: `offset /
# factor` long. With the original's 0.05 a game that starts from a world
# 465 ms behind (a round trip of 200 ms) needs 9.3 s, and the server drops
# every input that is more than 250 ms late: for 6 to 9 s after every
# start, late join and resync the player's keys do nothing (live,
# 2026-10-14; 13 to 15 s at a round trip of 300 ms). The default becomes
# 0.3, which the game's own range allows: 1.6 s. Four bytes of data, no
# code and no address (no base relocation is under them). Nothing that a
# match computes reads the factor, so games of either play one match.
CLOCK_FACTOR_VA = 0xA799C8
CLOCK_FACTOR_ORIGINAL, CLOCK_FACTOR = 0.05, 0.3
CLOCK_FACTOR_READ_VA = 0x1E9848               # the constructor's `movss`
CLOCK_FACTOR_REGISTER_VA = 0x851369           # `mov dword [0A9D110h],0A799C8h`
CLOCK_FACTOR_OPTION = 0xA9D0D0                # the console variable; its name at +10h
CLOCK_FACTOR_MIN_VA, CLOCK_FACTOR_MAX_VA = 0x91A8F0, 0x927A44
# .data: the only section but .text that a patch writes into.
DATA_VA, DATA_FILE, DATA_FILE_SIZE = 0xA5E000, 0xA4C400, 0x92200
CLOCK_FACTOR_PATCH = (CLOCK_FACTOR_VA, struct.pack("<f", CLOCK_FACTOR_ORIGINAL), struct.pack("<f", CLOCK_FACTOR))

# game_world_ui::update_ui (0x23B200), when the hud's state has changed:
# while the match waits or counts down (state+138h, the timelimit rule's)
# it shows the countdown (0x236AD0, 0x236B50) and jumps past the rest to the
# players' rows (0x23B328 -> 0x23B421); otherwise it shows the match time
# (0x236A00) and goes on at 0x23B383: the team points (0x2364C0,
# `root.set_game_progress`), in Team Deathmatch the last players
# (0x236610, `root.set_last_players`), and the respawn time (0x236C30,
# `root.set_respawn_time`). The rules fill the state's points every tick,
# the countdown too (tdm 0x316430: the tickets; gather 0x3140C0: the
# points), so they are the match's own then, but the hud kept the bars of
# the match before until play began (seen: 63 and 53, the last match's end,
# under "Awaiting match start"). The jump goes on to 0x23B383 instead: the
# same three, with the match time and the start's tip left out. During the
# countdown the respawn time is 0 (no player waits for one: the saved
# worlds of a real match), which shows an empty text, and the hud movie
# hides the last players while a team has tickets (HUD.as,
# set_game_progress): nothing but the bars changes, and they are drawn as
# play would draw them a few seconds later. Both paths reach 0x23B383 with
# the stack and the registers the code after it reads alike. The jump's
# rel32 holds no address. `jmp 0x23b421` -> `jmp 0x23b383`.
PREGAME_PROGRESS_VA, PREGAME_PROGRESS_TARGET = 0x23B328, 0x23B383
PREGAME_PROGRESS = (PREGAME_PROGRESS_VA, bytes.fromhex("e9f4000000"),
                    b"\xE9" + struct.pack("<i", PREGAME_PROGRESS_TARGET - PREGAME_PROGRESS_VA - 5))

RUN_UNFOCUSED_VA = 0x1A0E1
RUN_UNFOCUSED_BRANCH_VA = 0x1A0EE
RUN_UNFOCUSED = (RUN_UNFOCUSED_VA, bytes.fromhex("8b53088b42308d4b08ffd084c07423"),
                 bytes.fromhex("8b53088b42308d4b08ffd084c09090"))


def late_join_state_va(code):
    """Where the state page of this late-join code is: the page after it."""
    end = LATE_JOIN_VA + len(code)
    return (end + 0xFFF) // 0x1000 * 0x1000


def late_join_hook(va, expected, offset, opcode):
    """The bytes a hook site gets: the call or jump and nops."""
    target = LATE_JOIN_VA + offset
    return bytes([opcode]) + struct.pack("<i", target - va - 5) + b"\x90" * (len(expected) - 5)

HOOKS = [
    # network_client::on_match_ready, ebx = network_client: mov ebx,[ebx+1F1C48h]
    (0x27AEA1, bytes.fromhex("8b9b481c1f00")),
    # on_match_packet_received, case 0xF6: cmp byte [g_debug_hash_mismatches],0
    (0x27A505, bytes.fromhex("803d2cd3f10100")),
]


def parse_server(text):
    """The login client has a 64-byte host buffer; reserve its NUL byte."""
    match = re.fullmatch(r"([A-Za-z0-9](?:[A-Za-z0-9.-]{0,61}[A-Za-z0-9])?):([0-9]{1,5})", text)
    if not match or not 1 <= int(match.group(2)) <= 65535:
        sys.exit("server must be host:port (ASCII host up to 63 bytes, port 1..65535)")
    return f"{match.group(1)}:{int(match.group(2))}"


def file_offset(va):
    assert TEXT_VA <= va < TEXT_VA + TEXT_VSIZE
    return va - TEXT_VA + TEXT_FILE


def data_file_offset(va):
    assert DATA_VA <= va < DATA_VA + DATA_FILE_SIZE
    return va - DATA_VA + DATA_FILE


def patch_value(data, va, expected, new):
    """As `patch`, for a value in .data."""
    at = data_file_offset(va)
    found = bytes(data[at:at + len(expected)])
    if found != expected or len(new) != len(expected):
        sys.exit(f"{va:#x}: expected {expected.hex()}, found {found.hex()}")
    data[at:at + len(new)] = new


def patch(data, va, expected, new):
    at = file_offset(va)
    found = bytes(data[at:at + len(expected)])
    if found != expected:
        sys.exit(f"{va:#x}: expected {expected.hex()}, found {found.hex()}")
    data[at:at + len(new)] = new


class Headers:
    """The few PE header fields the patch reads or changes (PE32 only)."""

    def __init__(self, data):
        self.pe = struct.unpack_from("<I", data, 0x3C)[0]
        if data[self.pe:self.pe + 4] != b"PE\0\0":
            sys.exit("not a PE file")
        self.count_at = self.pe + 6
        self.optional = self.pe + 24
        if struct.unpack_from("<H", data, self.optional)[0] != 0x10B:
            sys.exit("not a PE32 image")
        self.size_of_code_at = self.optional + 4
        self.size_of_image_at = self.optional + 56
        self.checksum_at = self.optional + 64
        self.security_at = self.optional + 96 + 8 * 4
        self.reloc_at = self.optional + 96 + 8 * 5
        self.debug_at = self.optional + 96 + 8 * 6
        self.table = self.optional + struct.unpack_from("<H", data, self.pe + 20)[0]
        self.section_alignment, self.file_alignment = struct.unpack_from("<II", data, self.optional + 32)
        self.size_of_headers = struct.unpack_from("<I", data, self.optional + 60)[0]

    def sections(self, data):
        """[(name, rva, virtual size, raw offset, raw size, characteristics)]"""
        out = []
        for i in range(struct.unpack_from("<H", data, self.count_at)[0]):
            o = self.table + 40 * i
            vsize, rva, rsize, raw = struct.unpack_from("<IIII", data, o + 8)
            out.append((bytes(data[o:o + 8]), rva, vsize, raw, rsize, struct.unpack_from("<I", data, o + 36)[0]))
        return out


def pe_checksum(data, checksum_at):
    """The checksum of IMAGEHLP CheckSumMappedFile: a 16-bit one's-complement
    sum of the file with the checksum field as zero, plus the file size."""
    total = 0
    padded = bytes(data) + b"\0" * (len(data) % 2)
    for (word,) in struct.iter_unpack("<H", padded[:checksum_at] + b"\0\0\0\0" + padded[checksum_at + 4:]):
        total += word
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return total + len(data)


def neutralize_relocs(data, ranges):
    """Turn HIGHLOW base relocations that overlap a [lo, hi) VA range into
    type 0 (skip). Returns the VAs of the changed entries."""
    headers = Headers(data)
    rva, size = struct.unpack_from("<II", data, headers.reloc_at)
    base_off = None
    for _, srva, vsize, raw, rsize, _ in headers.sections(data):
        if srva <= rva < srva + max(vsize, rsize):
            base_off = rva - srva + raw
    if base_off is None:
        sys.exit("no .reloc section")
    o, end, changed = base_off, base_off + size, []
    while o < end:
        page, block = struct.unpack_from("<II", data, o)
        if block < 8:
            break
        for i in range((block - 8) // 2):
            at = o + 8 + 2 * i
            e = struct.unpack_from("<H", data, at)[0]
            va = IMAGE_BASE + page + (e & 0xFFF)
            if e >> 12 == 3 and any(va + 4 > lo and va < hi for lo, hi in ranges):
                struct.pack_into("<H", data, at, e & 0xFFF)
                changed.append(va)
        o += block
    return changed


def split_code(code):
    sizes = [len(original) for _, original in HOOKS]
    return code[:-sum(sizes)], code[-sum(sizes):], sizes


def add_state_section(data, state_va):
    """Adds `.srpd`, the late join's zeroed, writable page at `state_va`,
    after `.srp`. It has no bytes in the file."""
    headers = Headers(data)
    sections = headers.sections(data)
    name, rva, vsize, _, _, _ = sections[-1]
    align = lambda value, to: (value + to - 1) // to * to
    slot = headers.table + 40 * len(sections)
    if slot + 40 > headers.size_of_headers or any(data[slot:slot + 40]):
        sys.exit("no free section table entry for the late join's state")
    state_rva = state_va - IMAGE_BASE
    if name != SECTION_NAME or state_rva != align(rva + vsize, headers.section_alignment):
        sys.exit("the late join's state does not follow the code")
    data[slot:slot + 40] = LATE_JOIN_STATE_NAME + struct.pack(
        "<IIIIIIHHI", LATE_JOIN_STATE_SIZE, state_rva, 0, 0, 0, 0, 0, 0, LATE_JOIN_STATE_CHARACTERISTICS)
    struct.pack_into("<H", data, headers.count_at, len(sections) + 1)
    struct.pack_into("<I", data, headers.size_of_image_at, state_rva + LATE_JOIN_STATE_SIZE)
    uninitialized_at = headers.optional + 12
    struct.pack_into("<I", data, uninitialized_at,
                     struct.unpack_from("<I", data, uninitialized_at)[0] + LATE_JOIN_STATE_SIZE)


def add_section(data, cave):
    """Appends the .srp section holding `cave` and moves the Authenticode
    blob behind it. Returns the new file contents."""
    headers = Headers(data)
    sections = headers.sections(data)
    _, last_rva, last_vsize, last_raw, last_rsize, _ = sections[-1]
    align = lambda value, to: (value + to - 1) // to * to
    slot = headers.table + 40 * len(sections)
    if slot + 40 > headers.size_of_headers or any(data[slot:slot + 40]):
        sys.exit("no free section table entry")
    if align(last_rva + last_vsize, headers.section_alignment) != SECTION_RVA \
            or struct.unpack_from("<I", data, headers.size_of_image_at)[0] != SECTION_RVA:
        sys.exit("the image does not end where the new section starts")
    raw = last_raw + last_rsize
    security_offset, security_size = struct.unpack_from("<II", data, headers.security_at)
    # The attribute certificate table is the only thing after the last
    # section, and its data directory holds a FILE offset: it must move with
    # the bytes, and stays the last thing in the file (8-byte aligned), as
    # the PE format asks. vostok::crypt::verify_sign reads it at every start.
    if security_offset != raw or security_offset + security_size != len(data) or raw % headers.file_alignment:
        sys.exit("unexpected layout after the last section")
    if struct.unpack_from("<I", data, security_offset)[0] > security_size:
        sys.exit("the certificate table entry is larger than the table")
    debug_rva, debug_size = struct.unpack_from("<II", data, headers.debug_at)
    # IMAGE_DEBUG_DIRECTORY.PointerToRawData is the other file offset in the
    # headers; it must point before the bytes that move.
    debug_at = next(debug_rva - rva + offset for _, rva, vsize, offset, rsize, _ in sections
                    if rva <= debug_rva < rva + rsize)
    for entry in range(debug_size // 28):
        if struct.unpack_from("<I", data, debug_at + 28 * entry + 24)[0] >= raw:
            sys.exit("debug data after the last section would move")
    raw_size = align(len(cave), headers.file_alignment)
    if (raw + raw_size) % 8:
        sys.exit("the certificate table would lose its 8-byte alignment")

    out = bytearray(data[:raw]) + cave + b"\xCC" * (raw_size - len(cave)) + data[raw:]
    out[slot:slot + 40] = SECTION_NAME + struct.pack(
        "<IIIIIIHHI", len(cave), SECTION_RVA, raw_size, raw, 0, 0, 0, 0, SECTION_CHARACTERISTICS)
    struct.pack_into("<H", out, headers.count_at, len(sections) + 1)
    struct.pack_into("<I", out, headers.size_of_image_at,
                     align(SECTION_RVA + len(cave), headers.section_alignment))
    struct.pack_into("<I", out, headers.size_of_code_at,
                     struct.unpack_from("<I", data, headers.size_of_code_at)[0] + raw_size)
    struct.pack_into("<I", out, headers.security_at, security_offset + raw_size)
    return out


def build(original, mode, code=None, server=None, late_join=None):
    """Returns (patched file, description). mode: "cert-only", "section" or
    "hash-mismatch"; code = the assembled .bin for that cave. An optional
    server embeds a default host:port in full builds only. late_join = the
    late-join code (late_join_026g.section.bin), in "section" builds only:
    with it the build is patch revision REVISION, without it
    REVISION_WITHOUT_LATE_JOIN."""
    if hashlib.sha256(original).hexdigest() != EXE_SHA256:
        sys.exit("the source is not the original 0.26g survarium.exe (sha256 mismatch)")
    if server is not None:
        server = parse_server(server)
        if mode == "cert-only":
            sys.exit("embedding a server requires a full patch, not --cert-only")
    if late_join is not None:
        if mode != "section":
            sys.exit("the late join needs the default build: its code and its state are sections of their own")
        if len(late_join) <= LATE_JOIN_ASM_SIZE:
            sys.exit("the late-join code is the 4096 bytes of late_join_026g.asm and the C code after them")
    revision = REVISION if late_join is not None else REVISION_WITHOUT_LATE_JOIN
    data = bytearray(original)
    patch(data, *CERT)
    if mode == "cert-only":
        return data, "certificate check only"

    cave_va = CAVES[mode]
    cave, hooks, sizes = split_code(code)
    patch(data, *MUTEX)
    patch(data, *MEMORY_CHECK)
    patch(data, *HASH_INTERVAL)
    rewritten = [(HASH_INTERVAL_BYTE_VA, HASH_INTERVAL_BYTE_VA + 1),
                 (MEMORY_CHECK[0], MEMORY_CHECK[0] + 1)]
    patch(data, *revision_site(revision))
    rewritten.append((REVISION_VA, REVISION_VA + len(REVISION_ORIGINAL)))
    upload_size = len(cave)
    if server is not None:
        if upload_size != DEFAULT_SERVER_OFFSET:
            sys.exit("embedding a server requires the standard 420-byte snapshot cave")
        stub = open(default_server_bin(mode), "rb").read()
        # The checked-in stub ends with the relative jump to the original
        # server-option read; reject a stub assembled for the other placement.
        if len(stub) < 5 or stub[-5] != 0xE9 or \
                cave_va + upload_size + len(stub) + struct.unpack_from("<i", stub, len(stub) - 4)[0] != DEFAULT_SERVER_RETURN:
            sys.exit("the default-server stub has the wrong return target")
        target = cave_va + upload_size
        va, expected = DEFAULT_SERVER_HOOK
        hook = b"\xE9" + struct.pack("<i", target - va - 5) + b"\x90" * (len(expected) - 5)
        patch(data, va, expected, hook)
        rewritten.append((va, va + len(expected)))
        cave += stub + server.encode("ascii") + b"\0"
    for site in GATHER:
        patch(data, *site)
        rewritten.append((site[0], site[0] + len(site[1])))
    patch(data, *MOUNT_NO_ROOT)
    rewritten.append((MOUNT_NO_ROOT[0], MOUNT_NO_ROOT[0] + len(MOUNT_NO_ROOT[1])))
    # Not in `rewritten`: the relocation of this address must stay active.
    patch(data, *JOURNAL_DEFAULT)
    if mode == "hash-mismatch":
        if len(cave) > HASH_MISMATCH_SIZE:
            sys.exit(f"the cave code is {len(cave)} bytes; {HASH_MISMATCH_SIZE} fit")
        at = file_offset(HASH_MISMATCH_VA)
        if hashlib.sha256(data[at:at + HASH_MISMATCH_REGION]).hexdigest() != HASH_MISMATCH_SHA256:
            sys.exit(f"{HASH_MISMATCH_VA:#x}: on_hash_mismatch is not the original")
        data[at:at + HASH_MISMATCH_REGION] = cave + b"\xCC" * (HASH_MISMATCH_REGION - len(cave))
        rewritten.append((HASH_MISMATCH_VA, HASH_MISMATCH_VA + HASH_MISMATCH_REGION))
        where = "over on_hash_mismatch"
    else:
        if len(cave) > LATE_JOIN_OFFSET:
            sys.exit(f"the cave code is {len(cave)} bytes; one page fits")
        where = f"in the new section .srp at {SECTION_VA:#x}"

    pos = 0
    for (va, original_bytes), size in zip(HOOKS, sizes):
        hook = hooks[pos:pos + size]
        pos += size
        if hook[0] != 0xE9 or set(hook[5:]) != {0x90}:
            sys.exit(f"{va:#x}: the hook is not jmp + nops: {hook.hex()}")
        target_va = va + 5 + struct.unpack_from("<i", hook, 1)[0]
        if not cave_va <= target_va < cave_va + len(cave):
            sys.exit(f"{va:#x}: the hook jumps to {target_va:#x}, outside the cave "
                     f"(was the .bin assembled with -DCAVE={cave_va:#x}?)")
        if late_join is not None and va == LATE_JOIN_READY_VA:
            # The late join's ready_hook first; it jumps on to `target_va`.
            if target_va != SECTION_VA:
                sys.exit(f"{va:#x}: the late join expects the upload's ready hook at the cave's start")
            hook = late_join_hook(va, original_bytes, LATE_JOIN_READY_OFFSET, 0xE9)
        patch(data, va, original_bytes, hook)
        rewritten.append((va, va + size))
    rewritten += [(CERT[0], CERT[0] + len(CERT[1])), (MUTEX[0], MUTEX[0] + len(MUTEX[1]))]
    relocs = neutralize_relocs(data, rewritten)
    if late_join is not None:
        sites = []
        for va, expected, new in LATE_JOIN_PATCHES:
            patch(data, va, expected, new)
            sites.append((va, va + len(expected)))
        for va, expected, offset, opcode in LATE_JOIN_HOOKS:
            patch(data, va, expected, late_join_hook(va, expected, offset, opcode))
            sites.append((va, va + len(expected)))
        patch(data, *RUN_UNFOCUSED)
        sites.append((RUN_UNFOCUSED_VA, RUN_UNFOCUSED_VA + len(RUN_UNFOCUSED[1])))
        patch(data, *PREGAME_PROGRESS)
        sites.append((PREGAME_PROGRESS_VA, PREGAME_PROGRESS_VA + len(PREGAME_PROGRESS[1])))
        patch(data, *REMOVE_MODEL)
        # The one address in what is replaced: the loader must not add to it.
        if neutralize_relocs(data, [(REMOVE_MODEL_VA, REMOVE_MODEL_VA + len(REMOVE_MODEL[1]))]) != [REMOVE_MODEL_VA + 2]:
            sys.exit("the base relocation of scene::remove_model's branch is not where it was read")
        # None of these instructions holds an address: a relocation under
        # one would mean another exe than the one they were read in.
        if neutralize_relocs(data, sites):
            sys.exit("a base relocation under a late-join site")
        patch_value(data, *CLOCK_FACTOR_PATCH)
        # A number, not an address.
        if neutralize_relocs(data, [(CLOCK_FACTOR_VA, CLOCK_FACTOR_VA + 4)]):
            sys.exit("a base relocation under time_floating_factor's value")

    if mode == "section":
        if late_join is not None:
            cave = cave + b"\xCC" * (LATE_JOIN_OFFSET - len(cave)) + late_join
        data = add_section(data, cave)
        if late_join is not None:
            add_state_section(data, late_join_state_va(late_join))
    headers = Headers(data)
    struct.pack_into("<I", data, headers.checksum_at, pe_checksum(data, headers.checksum_at))
    return data, (f"certificate check, mutex, minimum VRAM check, hash interval {HASH_INTERVAL_MS} ms, "
                  f"patch revision {revision}, battery rule, mount without a root, no journal by default, "
                  f"{upload_size} bytes of upload code {where} "
                  f"({len(relocs)} relocations disabled), {len(HOOKS)} snapshot hooks"
                  + (f", late join ({len(late_join)} bytes of code at {LATE_JOIN_VA:#x}, "
                     f"{len(LATE_JOIN_PATCHES)} sites rewritten, {len(LATE_JOIN_HOOKS) + 1} hooks), "
                     "goes on without the focus, a model not in the scene stays out of it, "
                     f"the clock is brought up at {CLOCK_FACTOR:g} (time_floating_factor), "
                     "the countdown shows the match's team points, "
                     "a spawn gives its weapons a random state of the match's own"
                     if late_join is not None else "")
                  + (f", default login server {server}" if server is not None else ""))


def default_bin(mode):
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), f"snapshot_upload_026g.{mode}.bin")


def default_server_bin(mode):
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), f"default_server_026g.{mode}.bin")


def late_join_bin():
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "late_join_026g.section.bin")


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--") or a == "--check"]
    options = [a for a in sys.argv[1:] if a.startswith("--") and a != "--check"]
    mode, bin_path, server = "section", None, None
    late_join, late_join_path = True, None
    for option in options:
        if option == "--cert-only":
            mode = "cert-only"
        elif option == "--no-late-join":
            late_join = False
        elif option.startswith("--late-join-bin="):
            late_join_path = option[16:]
        elif option.startswith("--cave=") and option[7:] in CAVES:
            mode = option[7:]
        elif option.startswith("--bin="):
            bin_path = option[6:]
        elif option.startswith("--server="):
            server = parse_server(option[9:])
        else:
            sys.exit(f"unknown option {option}\n\n{__doc__}")
    if len(args) == 3:                                     # the draft's form: <exe> <out> <bin>
        bin_path = args.pop()
    if len(args) != 2:
        sys.exit(__doc__)
    source, target = args
    if target != "--check" and os.path.exists(target) and os.path.samefile(source, target):
        sys.exit("the output must not be the original exe")
    code = None
    if mode != "cert-only":
        code = open(bin_path or default_bin(mode), "rb").read()
    late_join_code = None
    if mode == "section" and late_join:
        late_join_code = open(late_join_path or late_join_bin(), "rb").read()
    elif late_join_path:
        sys.exit("--late-join-bin goes with the default build only")
    data, description = build(open(source, "rb").read(), mode, code, server, late_join_code)
    if target != "--check":
        with open(target, "wb") as out:
            out.write(data)
    print(f"{target}: {description}")


if __name__ == "__main__":
    main()
