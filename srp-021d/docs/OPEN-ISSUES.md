# Open issues: the 0.21d server

> **Version scope:** the 0.21d server and client (live on skhid.link), unless
> an item is marked 0.100b. As of 2026-10-12. Players are written by label,
> never by name (`docs/PUBLISHING.md` at the repository's root). The 0.26g
> server's list is `srp-026g/docs/OPEN-ISSUES.md`.

## Players and matches

1. **Clients that connect but never finish loading.** In the 2026-10-11 20:36
   match 0x125, player C and player D sent 0x40 but never 0x42 and were dropped at
   the 60 s loading limit. Both rejoined fine. Five earlier cases split into
   two kinds:
   - sent 0x40 only and never acked 0x50 (player M twice, player N);
   - acked, then went silent (player O, player C).

   Not explained. Look at the transport and at what those clients did in the
   lobby at the time.
2. **player B (Russia) goes silent right after loading**, and his TCP connections
   freeze 10 to 20 s after opening. Most likely his network. A server-side help
   would be to resend a seat that went into a dead connection sooner (it is
   lost until his client reconnects).
3. **Resyncs with no logged cause** (48 of 111 before the 20:03 fixes; 21 of
   them two players at once). See MATCH-SYNC-021D.md. Re-measure on matches after
   2026-10-11 20:47.
4. **A late first entry differs in its own animation state** (0x131 player D;
   0x124 player E wrong from his entry tick). Unproven.
5. **Spiky links** (player A, player C, player E): late 0x54/0x58 past the
   client's 500 ms horizon. Ideas: a round-trip-based `SYNC_LEAD_MS`; test
   whether players can raise `-max_ping_time_in_ms`.
6. player F's client left a match by itself 7 times (0x132). Cause
   unknown.

## Server features

7. **Squads, the queue and matches are lost on every restart** (in memory
   only). Persisting squads was offered; not decided.
8. **Nothing committed since 2026-10-10 19:12** in `srp-021d` (41 files). Commit
   only when the owner asks.
9. No off-server backups of `srp.db`. No host rate limits on 1234, 1235 and
   1236. No git or deploy script on the server.
10. **Abuse findings still open** (full list in `srp-021d/WORK.md`, "Bans,
    random session ids"):
    - results decided from snapshots (now by majority, but collusion is
      possible);
    - no connection caps or timeouts on 1234 and 1235;
    - chat writes can block every sender;
    - UDP reflection on 1236;
    - every client sees the whole world (wallhacks are possible by design).
11. Not live-verified: the closed-registration message.
12. Check that the players' files zip (`Survarium-0.21d-srp-files.zip`,
    since 2026-10-12) launches with `-minimum_resources_memory_size=768`
    (level_02 runs out of memory without it).

## Client

13. The Scaleform lobby crash (0.100b, probably 0.21d too): one-byte patch
    candidate, not applied. See CLIENT-FACTS-021D.md.
14. Perks have no effect in 0.21d (client design; nothing to fix on the
    server). Tell players if they ask.

## Docs

15. Some `srp-021d/WORK.md` headings still say "not deployed" for changes that
    are live (one lobby chat; why resyncs and rejoins fail). The deploy times
    are in MATCH-SYNC-021D.md.

