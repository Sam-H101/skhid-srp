# A kit chosen before the match starts (patch 31)

The rules the code and the tests follow. Code: `client-patches/kit_change_026g.asm`,
README patch 31, `client_patch.md` section 3.29; the server: `TimelimitRule::before_play`,
`PlayerResult::kit_changes`, `autobuy::used`, the bots' `pick_kit`.

- **When:** from the player's spawn on entering the match until the match starts (the time
  rule leaves waiting/countdown). The tick whose time rule begins play still counts: its switch
  comes before that change. Never after.
- **Who:** a player in the match from its start, alive, who has done nothing yet: no shot, hit,
  kill or other event, no damage from another player. Several switches allowed until then.
- **Effect:** respawn in the same place, same facing (the look up or down is level, as at any
  spawn), what is left of the spawn invulnerability kept, with the chosen kit; no death, no
  timer, no ticket; supplies from the account as at any spawn.
- **State:** after a switch the player stays in the pre-start state (warmup camera, movement
  lock) taken from the match's current state, the one the network client set the camera for
  (+1F1C60h); the start then releases them like everyone.
- **Ordering:** switches key off the server-ordered kit choice (0xF3), by the tick it is for,
  taken by the places' tick and applied by the respawn rule in tick list 0, before the time
  rule (list 1) changes the state, the same order on every client and in every replay of a
  tick; a switch is applied before the start transition or not at all.
- **HUD:** the kit selector shows at spawn before the start and stays open after a pick, the
  cursor where it was (another pick switches again); it hides at the start. It is shown exactly
  while the camera is the pre-start one.
- **Late joiners:** join only after the start; no selector, no switch, their spawn untouched;
  they start with their first kit (the lowest kit the match loaded, as the slot's bind takes
  it; the same on `main`).
- **Bots:** pick a kit only after the start.
- **Results/supplies:** each kit choice before play counts as a life for supplies used;
  per-kit counters unaffected (a switch needs counters all zero).
- **Release:** changes the simulation: the `next` row of
  `crates/session/src/v026g/simulation_changes.tsv` says so; the release that ships it makes
  that row its revision, its server's minimum patch revision, and from it on replays recorded by
  older games are refused (`session::v026g::replays`).
