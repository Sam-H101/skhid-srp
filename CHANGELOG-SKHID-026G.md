# Skhid 0.26g — what changed

Changes for players of the Skhid server (026g.skhid.link), newest first.
All times are UTC. A time is when the change reached the players' server or
their download, taken from the server's own records.

Where a rule was changed more than once, it is written down once, at the
release that made it what it is now.

---

## 14 Oct, 05:36 — Match recordings

- Matches that had 6 or more players (late arrivals count) are now recorded
  on the server, so that lag and desync problems can be looked into afterwards.
  The recording holds the match's game traffic, not chat and not addresses,
  and stays on the server. A match that never reached 6 players is deleted
  when it ends.

## 14 Oct, 04:50 and 04:52 — "Matches" window with JOIN, login hint

These two come through the launcher: close the game, start `Skhid.bat`, and it
changes the game's lobby and login screens by itself (the original screens are
kept and can be put back).

- **New "Matches" window in the lobby** (a tab after the chat tabs): the matches
  being played right now, with map, mode, how many players each side has, the
  score (batteries or tickets) and how long it has been played. A match that has
  its result is not listed.
- **JOIN button** in that window, one click, no waiting. It is on for you in a
  match only when all of this is true:
  - the match takes players now: it is being played, has more than 2 minutes
    left, and has a free place on a side (a match takes one newcomer at a time,
    so for a few seconds after somebody joins, others see no JOIN on it);
  - you would be put into it at once and it would make the teams **more even by
    value** (gear, level and your hidden rating, yours counted), not just by
    head count: a match whose teams already count alike takes nobody, even at
    3 v 4;
  - you are in the lobby, not searching, not in a match and not in a squad, you
    were never in that match, and your game is up to date.
- When you press JOIN you are put into the match at once, on the side that
  makes the teams more even. The place is held for you for 10 seconds. If the
  match stops taking players before you are in (the teams became even, somebody
  took the place, it is nearly over), the game shows "Error changing readiness
  status" and you are not put in the search.
- **Login screen** no longer shows the original game's registration link; it
  has a short hint on how to get an account here.

## 14 Oct, 03:48 — Game update 8: lag

- After a start, a late join or a resync, a game's clock is behind the match.
  The game used to catch up very slowly, and the server throws away keys that
  come more than a quarter of a second late, so a far-away player's controls
  could do nothing for 6 to 15 seconds. The game now catches up about six
  times faster (a clock 465 ms behind is made up in about 1.6 s), and the
  server measures that lag more exactly.
- Update 8 plays together with update 7, so it is optional.

## 14 Oct, 02:26 — Game update 7: grenades for late arrivals, even teams

- **Players with grenades or traps in a kit can now join a match in progress.**
  Before, such a player was never let in (it failed four times out of four).
- **Teams are made even by value**: each player counts for the gear they can
  play with, their account level and a hidden rating built from how earlier
  matches went. Nobody sees it; it is not the rating in the ranking. With an
  odd number of players the stronger players make the side with one fewer.
  A match with fewer than 4 players moves nobody's hidden rating (such a match
  says little about how teams should be made).
- **Outdated games are refused at Play** with the game's own "outdated version"
  message and a chat line saying what to do: close the game and start
  `Skhid.bat`, which updates it. From this release games older than update 7
  are refused.

## 14 Oct, 00:44 — Game update 6: three faults when someone joins a match

- A player who joins a running match now appears on the scoreboard and in the
  other lists of players (before, the lists were made once, for the players
  who were there at the start).
- Fixed two crashes of games that were given a newcomer.

## 13 Oct, 23:27 — Game update 5: join running matches, new economy

- **Matches no longer wait for a full lobby.** A match starts with 2 players,
  2 minutes after the second one pressed Play (20 players start at once). It
  takes new players while it is being played, until 2 minutes before its end:
  they go to the side that makes the teams more even. A match holds 20 players,
  10 a side; a place of a player who left is not given again. A late arrival
  is paid at the match's level, which stays what it was at the start.
- **Silver, experience and reputation**:
  - silver of a match is 150 % of the original game's at match levels 0 to 5,
    then 160, 170, 180, 190, 200, 210 and 220 % from level 12;
  - experience of a match is paid twice;
  - reputation of a match and of its quests is 120 %.
  The match's level is the mean of its players' gear levels (the first players
  put into it), so a match that began small is paid at a low level whoever
  joins later.
- **Ammo costs about half the original price** and **needs no reputation rank**
  (in the shop, in the kit's ammo window and in the auto-buy after a match).
  Grenades, traps and drugs keep their ranks.
- **Quest goals count what the in-match quest counter counts**: "Find and bring
  batteries" counts a battery you picked up first (or took from the enemy) and
  put into the container, if no teammate carried it in between; "Steal a battery" counts the one you took and put in; "Win a match
  without losing batteries" can be done only in the battery modes; a kills/
  deaths ratio goal is met at its number.
- **A game whose window is not in front keeps running.** Others no longer see
  you frozen when you alt-tab, and a join is not held up by a game that says
  nothing.
- **Players on bad connections**: the world of a match is now sent in smaller
  steps, aimed at the "free camera, no character" problem in which one player
  loaded the level and never entered. Not yet seen to be fixed.
- Launcher page: a second tab, **"Matches now"**, with the matches being played.

## 13 Oct, 18:38 — Economy, squads off

- **Weapons and armour no longer wear down** in matches (no new defects). All
  worn weapons and armour were mended once.
- **No play-time bonus** to the chance of a drop at the end of a match.
- **Squads are switched off**: an invite is refused with "Squads are switched
  off on this server" and everybody queues alone. Squads were in more than half
  of the matches and kept teams from being made even.
- Launcher page: the numbers (players online, queue, matches, maintenance) now
  refresh by themselves every 10 seconds.

## 13 Oct, 16:27 — Updater

- When the first installation of the game fails, the updater writes what the
  unpacker said into `updater_skhid.log`, so the reason can be found (a player
  had "exit 2" with no explanation).

## 13 Oct, 15:36 — Game update 4: Documents folder

- The game starts when Windows does not let it write to its folder in
  Documents (the cause of two start-up crashes). It then runs with settings
  that are not saved, and a message at the start says which folder and how to
  allow the game in Windows' "Controlled folder access".

## 13 Oct, 12:30 — The launcher installs the game

- A new player no longer needs the original game first: unpack our files into
  an empty folder, start `Skhid.bat`, and the launcher downloads Survarium 0.26g
  (2.9 GB, from Google Drive), checks it and installs it. The unpacker comes
  in our ZIP. Players who have the game already are not affected.
- **Game update 3 (download of 12:22):** the last battery could not be put into
  its container once a team had nine, which ended the match only on the time
  limit and stopped stealing. The containers stay usable now. (Since then a battery match
  has ended by its rule and not on the time limit: 13 Oct, 19:01.)

## 13 Oct, 10:30 — Launcher page

- The launcher's page has the game's original look again, and a link to Discord.

## 13 Oct, 09:48 (server) and 09:50 (download) — The launcher works again

- The game's own launcher starts again with our files: its page is ours (top 10,
  players online, queue, matches, accounts), it no longer asks any Vostok Games
  address, and it **updates your Skhid files by itself** from the download on
  the site each time it starts, including its own updater.
- A search for a match **restarts when your equipment level changes**.
- Quests that need grenades or artefacts are offered only once the store that
  sells them is unlocked.
- Fixed a crash after sign-in: a game that had not loaded any match was sent a
  results screen.

## 12 Oct, 22:18 — Search chat lines

- A line in the general chat when somebody starts or stops searching, and
  when a match starts, with whom. Everybody listening to the general chat sees
  it; the join line is shown once per 30 seconds per player.

## 12 Oct, 21:10 and 21:20 — New download

- The download opens the server `skhid.link` by itself (no address to type).
- Our own **crash report window** in the style of the original; "Submit bug"
  opens our Discord.
- The Linux (Wine) launcher is in the ZIP. The launchers and texts are English
  by default (21:20).

## 12 Oct, 19:16 — Leaving a match, camouflage

- **Leaving a match from the menu really leaves it.** Before, Play sent you
  straight back to the same match (a 1 v 1 reopened over and over). A game that
  crashed or lost its link still gets its place kept.
- Moving a camouflage between kits refreshes the kit.

## 12 Oct, 18:43 — New download

- The game says which version of our patches it has when it signs in, so the
  server knows who has updated.

## 12 Oct, 14:38 — Crash of every game in a match

- A match no longer crashes every game at one moment: a respawn with a kit that
  had no weapon (empty second kits and the premium kit of every premium
  account could be chosen in a match). Such kits can no longer be chosen.
- A match whose games have all stopped now ends once a player has waited about
  20 seconds for the world, instead of holding its players for 9 minutes (it
  had sent them back to the dead match each time they pressed Play).

## 12 Oct, 13:32 — Update

- The results screen you have not seen yet is kept across a server restart.
- A player who comes back into a Team Deathmatch in progress no longer gets
  wrong ticket counts.
- A game that is still connecting is waited for (3 seconds) instead of being
  told to disconnect when its first packet is lost.
- Impossible mouse turns sent by one game are no longer passed to the others
  (one bad input used to crash every game).

---

## Good to know

- The in-match quest window and the "Mission accomplished" card use the game's
  own numbers, so they do not show the 120 % reputation; the results screen,
  reputation bar and rank do.
- The kit's ammo window shows the original price for ammo you own; the shop
  price is the one the server uses.
