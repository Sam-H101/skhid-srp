# Match rewards

`servers/lobby-server/src/lobby_server/rewards.rs` gives what a match earned
and takes what it used up. No 2013 figures survive: the 2013 sources (the
alpha diary of 13 May 2013, the 0.13a–0.18a patch notes) describe ratings and
a statistics window but give no numbers, and two wipes in early 2014 erased
the 2013 progress. Searched sources and saved pages: session scratchpad
`audit2/web/rewards/`.

| What | Value | Source |
|---|---|---|
| Points | kill 10, battery taken from the enemy base 35 | wiki `Survarium`, revision 2015-02-04T20:36:03Z |
| Assists | none | this build has no assists (no client string, scoreboard has kills and deaths only) |
| Experience | the match's points | as above ("the score ... used to calculate your Ranking"); srp's choice |
| Experience per level | level 1: 500, each next 250 more (listed to 15 = 4,000) | same revision, "Required Experience"; srp continues past 15 |
| Level reward | 100 gold at level 1 and every fifth, 250 at 100; otherwise 500 money at 2–4, 250 more every five levels, to 5,000 | same revision, "Leveling" |
| Skill point | 1 a level | same revision; wiki `Hero`, 2013-06-12 |
| Money | 100 for staying to the match's end, 10 a point (100 a kill) | not from a source: chosen by the project owner |
| Starting money | 20,000 | not from a source: chosen by the project owner |

Supplies: every round fired (of the ammo the shot used), drug used and trap
placed in the match (from its start) comes off the account's stacks, the
selected profile's first. In this build nothing else wears: items keep their
condition, and the repair button's handler is a stub
(`InventoryItemRenderer.onRepairClick`).

When: the match server keeps a result per player when the match finishes, or
when the player leaves or drops before (then without the 100). The lobby
gives it at that account's next lobby message, whatever it is, and keeps it on
the account. The notice (`server-notices.md`) waits for the account's chat.
