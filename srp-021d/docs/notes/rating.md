# Rating and the ranking page (0.21d)

Evidence: **[V]** read in the exe or the lobby SWF; **[S]** srp's choice.

## What the client shows

- **`36 0A` base_info:** its u32 is the number of ranked players
  (`root.ranking_set_players_total_count`, `PlayerRanking.totalPlayers`). The
  "top" tab shows at most 100 of them. [V]
- **`36 0B` own row** (`read_player_elo_rating` 0x19E88D →
  `set_player_elo_rating` 0x177BBF): `u32 place`, `u16 rating`,
  `u16 rating before`. Flash gets {place, nickname (the account's),
  ranking = rating, ranking_diff = rating − before} via
  `root.ranking_set_player`. [V]
- **`21 0C` request** (`PlayerRanking.request_players` →
  `request_players_range`): `u32 start` (0-based index), `u8 count`. The page
  asks (0, 100) when it opens; the "my league" tab asks around
  `place − 50`. It files the rows it gets, in order, under the indices it
  asked for (`fillPlayers`), so the reply must start at `start`. [V]
- **`36 0C` rows** (`read_players_elo_list` 0x19EA34, the list builder at
  0x177A9D, stride 0x48): `u8 n`, then n × {`string name`, `u32 place`,
  `u16 rating`, `i16 change`}. Flash: {place, nickname, ranking,
  ranking_diff}; the change is shown as "(+N)" with an up or down arrow. [V]
- **Results screen** (`player_result_full` +0x44 / +0x46): rating before
  and after; the box shows "after (+after − before)". [V]

## srp's rule [S]

The original formula is lost. srp uses Elo between team averages:
everyone starts at 1000; each player moves by
`round(32 × (score − expected))`, expected = 1 / (1 + 10^((other − own) /
400)) of the team averages, score 1 a win, 0.5 a draw, 0 a loss. A player
who left the match scores 0. A match without two teams (solo) or without a
known winner moves nothing. The ranking is every account srp knows, by
rating, then name; places start at 1. Ratings live in memory like the rest
of the account.
