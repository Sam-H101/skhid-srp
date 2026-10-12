//! Reading a world snapshot's statistics and game rules
//! (`../srp-021d/docs/notes/snapshot_format.md`), for the results and for
//! ending a match when a team has every battery.
//!
//! The body is `u32 mask`, a block per player in the mask, then four
//! u16-prefixed sections: bullets, statistics, game objects, game rules. A
//! player block ends in a bitstream with no byte length, so the sections are
//! found from the end instead: the last three prefixes chain to the end of
//! the body, and the statistics must parse to exactly their length.

use std::collections::HashMap;

/// Events of `match_stats_events_dict_enum`.
pub const EVENTS: usize = 24;

pub mod event {
    pub const KILL: usize = 0;
    pub const ASSIST: usize = 1;
    pub const DEATH: usize = 2;
    pub const ARTEFACT: usize = 5;
    pub const TEAMMATE_CURE: usize = 7;
    pub const TEAMMATE_RESCUE: usize = 8;
    pub const PARTICIPATING: usize = 11;
    pub const WON: usize = 12;
    pub const MELEE_KILL: usize = 13;
    pub const SILENT_KILL: usize = 14;
    pub const HEADSHOT_KILL: usize = 15;
    pub const WINNING_TEAM_VI_BONUS: usize = 16;
    pub const STEAL_VICTORY_ITEM: usize = 17;
    pub const RECOVER_VICTORY_ITEM: usize = 18;
    pub const VICTORY_ITEM_FOUND: usize = 19;
    pub const PUT_VICTORY_ITEM_IN_CONTAINER: usize = 20;
    pub const BULLET_FIRE: usize = 22;
    pub const BULLET_HIT: usize = 23;
}

/// Events whose counters are hundredths (credit split by carry share).
pub const HUNDREDTHS: [usize; 3] = [6, 17, 18];

/// One roster place's statistics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerStatistics {
    pub events: [u16; EVENTS],
    /// The current streak of kills.
    pub killstreak: u16,
    /// Kills of players wearing each faction's gear (0..=6).
    pub faction_kills: [u8; 7],
}

/// `timelimit_rule_core`'s state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timelimit {
    /// The phase's start, relative to the snapshot's time (negative: past).
    pub start: Option<i64>,
    /// 0 inactive, 1 waiting for players, 2 countdown, 3 in process.
    pub state: u8,
    pub players_mask: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// `None` when the client's statistics were detached (empty section).
    pub statistics: Option<Vec<PlayerStatistics>>,
    /// Batteries in each team's container (the gather rule's own count,
    /// which decides the winner).
    pub team_points: [u8; 2],
    /// Kills and deaths per roster place (`kd_stats_rule`).
    pub kills_deaths: Vec<(u16, u16)>,
    pub timelimit: Timelimit,
}

/// The game rules hold at least the gather rule's seed and team points, the
/// timelimit (13 bytes) and the kd stats (5 a player).
const GATHER_HEAD: usize = 6;
const TIMELIMIT_LEN: usize = 13;
const KD_LEN: usize = 5;
/// Time fields are written as `t + 1e9 − T`.
const TIME_OFFSET: i64 = 1_000_000_000;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Decode a snapshot body (0x53 without its time) of a match with
/// `players` roster places and `batteries` batteries.
pub fn decode(body: &[u8], players: usize, batteries: usize) -> Option<Decoded> {
    // Where each possible u16 prefix's section would end.
    let mut ending_at: HashMap<usize, Vec<usize>> = HashMap::new();
    for at in 4..body.len().saturating_sub(1) {
        let end = at + 2 + usize::from(u16_at(body, at)?);
        if end <= body.len() {
            ending_at.entry(end).or_default().push(at);
        }
    }
    let mut empty_statistics = None;
    for &rules_at in ending_at.get(&body.len())? {
        for &objects_at in ending_at.get(&rules_at).into_iter().flatten() {
            for &statistics_at in ending_at.get(&objects_at).into_iter().flatten() {
                let statistics = &body[statistics_at + 2..objects_at];
                let rules = &body[rules_at + 2..];
                let Some(rules) = decode_rules(rules, players) else {
                    continue;
                };
                if statistics.is_empty() {
                    empty_statistics.get_or_insert(rules);
                    continue;
                }
                if let Some(statistics) = decode_statistics(statistics, players, batteries) {
                    return Some(Decoded {
                        statistics: Some(statistics),
                        ..rules
                    });
                }
            }
        }
    }
    empty_statistics
}

/// The gather rule's head, the timelimit and the kd stats.
fn decode_rules(rules: &[u8], players: usize) -> Option<Decoded> {
    let kd_at = rules.len().checked_sub(KD_LEN * players)?;
    let timelimit_at = kd_at.checked_sub(TIMELIMIT_LEN)?;
    if timelimit_at < GATHER_HEAD {
        return None;
    }
    let team_points = [rules[4], rules[5]];
    let kills_deaths = (0..players)
        .map(|player| {
            let at = kd_at + KD_LEN * player;
            Some((u16_at(rules, at)?, u16_at(rules, at + 2)?))
        })
        .collect::<Option<Vec<_>>>()?;
    let start = u32_at(rules, timelimit_at)?;
    let current = u32_at(rules, timelimit_at + 4)?;
    // The rule's last tick is the snapshot's own time.
    if i64::from(current) != TIME_OFFSET {
        return None;
    }
    let timelimit = Timelimit {
        start: (start != u32::MAX).then(|| i64::from(start) - TIME_OFFSET),
        players_mask: u32_at(rules, timelimit_at + 8)?,
        state: rules[timelimit_at + 12],
    };
    Some(Decoded {
        statistics: None,
        team_points,
        kills_deaths,
        timelimit,
    })
}

/// `shared_statistics::serialize`: each roster place, the curing records,
/// the batteries, the team points. `None` unless it fills `bytes` exactly.
fn decode_statistics(
    bytes: &[u8],
    players: usize,
    batteries: usize,
) -> Option<Vec<PlayerStatistics>> {
    let mut at = 0;
    let mut statistics = Vec::with_capacity(players);
    for _ in 0..players {
        let mut events = [0; EVENTS];
        for event in &mut events {
            *event = u16_at(bytes, at)?;
            at += 2;
        }
        // Damage from others, then hits on others: {u8, 4 bytes} each.
        for _ in 0..2 {
            let count = usize::from(*bytes.get(at)?);
            at += 1 + 5 * count;
        }
        // Last kill time and last hit enemy.
        at += 5;
        let killstreak = u16_at(bytes, at)?;
        // The streak, the action coefficient, then 7 faction counts.
        let faction_kills = bytes.get(at + 6..at + 13)?.try_into().ok()?;
        at += 2 + 4 + 7;
        statistics.push(PlayerStatistics {
            events,
            killstreak,
            faction_kills,
        });
    }
    // Curing records, 7 bytes a player.
    at += 7 * players;
    for _ in 0..batteries {
        let carriers = u32_at(bytes, at)?.count_ones() as usize;
        // Mask, distances, the two nearest positions, owner, found.
        at += 4 + 4 * carriers + 8 + 2;
    }
    // The statistics' own team points.
    at += 2;
    (at == bytes.len()).then_some(statistics)
}

/// The scoreboard's score: the events weighted as the client weighs them
/// (`recalculate_scores`).
pub fn score(events: &[u16; EVENTS], weights: &[f32; EVENTS]) -> u16 {
    let total: f64 = (0..EVENTS)
        .map(|event| {
            let factor = if HUNDREDTHS.contains(&event) {
                0.01
            } else {
                1.0
            };
            f64::from(events[event]) * f64::from(weights[event]) * factor
        })
        .sum();
    (total + 0.5).floor().clamp(0.0, f64::from(u16::MAX)) as u16
}

/// The winner by batteries in the containers: the gather rule decides every
/// network match (`winner_team`). 2 is a draw.
pub fn winner(team_points: [u8; 2]) -> u8 {
    match team_points[0].cmp(&team_points[1]) {
        std::cmp::Ordering::Greater => 0,
        std::cmp::Ordering::Less => 1,
        std::cmp::Ordering::Equal => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two players, five batteries, level_03 in mode 2, 539.65 s in: player
    /// 1 has 3 kills, 2 deaths, 3 headshots and has brought one battery
    /// home.
    const LATE: &[u8] = include_bytes!("../tests/data/level_03_mode2_539650.bin");
    /// The same match's initial world.
    const INITIAL: &[u8] = include_bytes!("../tests/data/level_03_mode2_50.bin");

    #[test]
    fn a_running_match_is_read_from_its_snapshot() {
        let decoded = decode(LATE, 2, 5).unwrap();
        assert_eq!(decoded.team_points, [0, 1]);
        assert_eq!(decoded.kills_deaths, [(0, 3), (3, 2)]);
        assert_eq!(decoded.timelimit.state, 3);
        assert_eq!(decoded.timelimit.players_mask, 0b11);
        let statistics = decoded.statistics.unwrap();
        let bob = &statistics[1].events;
        assert_eq!(bob[event::KILL], 3);
        assert_eq!(bob[event::DEATH], 2);
        assert_eq!(bob[event::HEADSHOT_KILL], 3);
        assert_eq!(bob[6], 100, "one whole battery brought");
        assert_eq!(statistics[0].events[event::DEATH], 3);
        assert_eq!(statistics[0].events[event::VICTORY_ITEM_FOUND], 1);
        assert_eq!(winner(decoded.team_points), 1);
    }

    #[test]
    fn the_initial_world_waits_for_players() {
        let decoded = decode(INITIAL, 2, 5).unwrap();
        assert_eq!(decoded.team_points, [0, 0]);
        assert_eq!(decoded.timelimit.state, 1);
        assert!(
            decoded
                .statistics
                .unwrap()
                .iter()
                .all(|player| player.events == [0; EVENTS])
        );
    }

    #[test]
    fn the_score_weighs_the_events() {
        let mut events = [0; EVENTS];
        events[event::KILL] = 3;
        events[6] = 150;
        let mut weights = [0.0; EVENTS];
        weights[event::KILL] = 100.0;
        weights[6] = 100.0;
        assert_eq!(score(&events, &weights), 450);
    }

    #[test]
    fn garbage_is_not_a_snapshot() {
        assert_eq!(decode(&[0; 40], 2, 5), None);
        assert_eq!(decode(&LATE[..LATE.len() - 1], 2, 5), None);
    }
}
