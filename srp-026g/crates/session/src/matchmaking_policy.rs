//! Placement for experimental growing rosters. Departed players remain counted.
//! Maximize admitted population, then minimize the resulting team-average gap.
//! Across running matches prefer `(resulting population)^p * minutes remaining`.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use crate::MAX_PLAYERS_PER_MATCH;

pub const GATHER_WINDOW: Duration = Duration::from_secs(120);
pub const JOIN_CUTOFF: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Team {
    pub players: usize,
    pub level_sum: u32,
}

impl Team {
    pub fn average(self) -> f64 {
        if self.players == 0 {
            0.0
        } else {
            self.level_sum as f64 / self.players as f64
        }
    }

    pub fn add(&mut self, group: Self) {
        self.players += group.players;
        self.level_sum += group.level_sum;
    }
}

#[derive(Clone, Debug)]
pub struct Placement {
    /// Group index and team, in queue order. A group is never split.
    pub assignments: Vec<(usize, u8)>,
    pub teams: [Team; 2],
}

impl Placement {
    pub fn population(&self) -> usize {
        self.teams.iter().map(|team| team.players).sum()
    }

    pub fn level_gap(&self) -> f64 {
        (self.teams[0].average() - self.teams[1].average()).abs()
    }

    pub fn score(&self, remaining: Duration, exponent: f64) -> Option<f64> {
        (remaining > JOIN_CUTOFF && exponent.is_finite() && exponent > 0.0)
            .then(|| (self.population() as f64).powf(exponent) * remaining.as_secs_f64() / 60.0)
    }
}

struct Choice {
    previous: Option<Rc<Choice>>,
    group: usize,
    team: u8,
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn reconstruct(teams: [Team; 2], groups: &[Team], path: Option<&Rc<Choice>>) -> Placement {
    let mut assignments = Vec::new();
    let mut cursor = path;
    let mut result = teams;
    while let Some(choice) = cursor {
        assignments.push((choice.group, choice.team));
        result[usize::from(choice.team)].add(groups[choice.group]);
        cursor = choice.previous.as_ref();
    }
    assignments.sort_unstable();
    Placement {
        assignments,
        teams: result,
    }
}

/// Exact bounded-capacity dynamic programming. First find the largest balanced
/// population attainable, then compare level sums for those final team sizes.
/// For fixed sizes A/B, minimizing |sum_A * B - sum_B * A| minimizes the average
/// gap. Equal count/difference states are equivalent for every later group.
pub fn place(teams: [Team; 2], groups: &[Team], capacity: usize) -> Option<Placement> {
    let capacity = capacity.min(MAX_PLAYERS_PER_MATCH / 2);
    if capacity == 0 || teams.iter().any(|team| team.players > capacity) {
        return None;
    }
    let free = teams.map(|team| capacity - team.players);
    let mut reachable = vec![vec![false; free[1] + 1]; free[0] + 1];
    reachable[0][0] = true;
    for group in groups.iter().filter(|g| g.players > 0) {
        let n = group.players;
        for a in (0..=free[0]).rev() {
            for b in (0..=free[1]).rev() {
                if (a >= n && reachable[a - n][b]) || (b >= n && reachable[a][b - n]) {
                    reachable[a][b] = true;
                }
            }
        }
    }
    let targets: Vec<_> = reachable
        .iter()
        .enumerate()
        .flat_map(|(a, row)| {
            row.iter().enumerate().filter_map(move |(b, &yes)| {
                let final_a = teams[0].players + a;
                let final_b = teams[1].players + b;
                (yes && a + b > 0 && final_a > 0 && final_b > 0 && final_a.abs_diff(final_b) <= 1)
                    .then_some((a, b))
            })
        })
        .collect();
    let most = targets.iter().map(|(a, b)| a + b).max()?;
    let mut best: Option<Placement> = None;
    for (a, b) in targets.into_iter().filter(|(a, b)| a + b == most) {
        let target = [teams[0].players + a, teams[1].players + b];
        // Reduce weights for equal teams; keeps the number of distinct sums small.
        let divisor = if target[0] == target[1] { target[0] } else { 1 };
        let weights = [
            (target[1] / divisor) as i64,
            -((target[0] / divisor) as i64),
        ];
        let initial =
            i64::from(teams[0].level_sum) * weights[0] + i64::from(teams[1].level_sum) * weights[1];
        // All attainable differences equal initial + a multiple of this gcd.
        // The nearest such value to zero is a proven lower bound even when
        // perfect equality is impossible (e.g. an odd initial sum, even joins).
        let step = groups
            .iter()
            .filter(|g| g.players > 0)
            .fold(0, |step, group| {
                weights.iter().fold(step, |step, weight| {
                    gcd(step, (i64::from(group.level_sum) * weight).abs())
                })
            });
        let lower_bound = if step == 0 {
            initial.abs()
        } else {
            let remainder = initial.rem_euclid(step);
            remainder.min(step - remainder)
        };
        let mut states = vec![vec![BTreeMap::<i64, Option<Rc<Choice>>>::new(); b + 1]; a + 1];
        states[0][0].insert(initial, None);
        for (index, group) in groups.iter().enumerate().filter(|(_, g)| g.players > 0) {
            let n = group.players;
            for x in (0..=a).rev() {
                for y in (0..=b).rev() {
                    for (team, &weight) in weights.iter().enumerate() {
                        let source = match team {
                            0 if x >= n => Some((x - n, y)),
                            1 if y >= n => Some((x, y - n)),
                            _ => None,
                        };
                        let Some((sx, sy)) = source else { continue };
                        let additions: Vec<_> = states[sx][sy]
                            .iter()
                            .map(|(&delta, path)| {
                                (delta + i64::from(group.level_sum) * weight, path.clone())
                            })
                            .collect();
                        for (delta, previous) in additions {
                            states[x][y].entry(delta).or_insert_with(|| {
                                Some(Rc::new(Choice {
                                    previous,
                                    group: index,
                                    team: team as u8,
                                }))
                            });
                        }
                    }
                }
            }
            if states[a][b].contains_key(&-lower_bound) || states[a][b].contains_key(&lower_bound) {
                // No later group can improve this target's level gap.
                break;
            }
        }
        let (_, path) = states[a][b].iter().min_by_key(|(delta, _)| delta.abs())?;
        let candidate = reconstruct(teams, groups, path.as_ref());
        if candidate.level_gap() == 0.0 {
            // No alternate final team-size target can improve equality.
            return Some(candidate);
        }
        if best
            .as_ref()
            .is_none_or(|old| candidate.level_gap() < old.level_gap())
        {
            best = Some(candidate);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team(players: usize, average: u32) -> Team {
        Team {
            players,
            level_sum: players as u32 * average,
        }
    }

    #[test]
    fn squads_can_balance_each_other_in_one_batch() {
        let result = place([team(2, 10), team(3, 10)], &[team(2, 10), team(3, 10)], 10).unwrap();
        assert_eq!(result.assignments, [(0, 1), (1, 0)]);
        assert_eq!(result.teams, [team(5, 10); 2]);
        assert!(place([team(3, 10); 2], &[team(3, 10)], 10).is_none());
        let result = place(
            [team(3, 10); 2],
            &[team(3, 10), team(1, 10), team(1, 10)],
            10,
        )
        .unwrap();
        assert_eq!(result.population(), 11);
        assert_eq!(result.teams[0].players.abs_diff(result.teams[1].players), 1);
    }

    #[test]
    fn balanced_counts_are_required_even_when_gathering_has_expired() {
        assert!(place([Team::default(); 2], &[team(3, 10), team(1, 10)], 10).is_none());
        assert!(place([Team::default(); 2], &[team(3, 10)], 10).is_none());
        assert_eq!(
            place([Team::default(); 2], &[team(1, 10); 2], 10)
                .unwrap()
                .population(),
            2
        );
        assert!(place([team(2, 10); 2], &[team(2, 10)], 2).is_none());
    }

    #[test]
    fn a_high_level_joiner_brings_team_averages_closer() {
        let result = place([team(3, 10), team(3, 20)], &[team(1, 25)], 10).unwrap();
        assert_eq!(result.assignments, [(0, 0)]);
        assert_eq!(result.level_gap(), 6.25);
        // Size takes precedence even when the only placement worsens levels.
        let result = place([team(2, 40), team(3, 10)], &[team(1, 50)], 10).unwrap();
        assert_eq!(result.assignments, [(0, 0)]);
    }

    #[test]
    fn scoring_uses_resulting_population_and_excludes_the_last_two_minutes() {
        let a = Placement {
            assignments: vec![],
            teams: [team(4, 10); 2],
        };
        let b = Placement {
            assignments: vec![],
            teams: [team(1, 10); 2],
        };
        assert_eq!(a.score(Duration::from_secs(240), 1.0), Some(32.0));
        assert!(a.score(Duration::from_secs(240), 1.0) > b.score(Duration::from_secs(720), 1.0));
        assert!(a.score(Duration::from_secs(240), 0.5) < b.score(Duration::from_secs(720), 0.5));
        assert!(a.score(JOIN_CUTOFF, 1.0).is_none());
        assert!(
            a.score(JOIN_CUTOFF - Duration::from_millis(1), 1.0)
                .is_none()
        );
        assert!(
            a.score(JOIN_CUTOFF + Duration::from_millis(1), 1.0)
                .is_some()
        );
        assert!(a.score(Duration::from_secs(600), f64::NAN).is_none());
    }

    #[test]
    fn a_large_queue_fills_one_match_without_reusing_or_splitting_groups() {
        let groups: Vec<_> = (0..100).map(|i| team(1, i % 16)).collect();
        for capacity in [5, 6, 7, 10] {
            let result = place([Team::default(); 2], &groups, capacity).unwrap();
            assert_eq!(result.population(), capacity * 2);
            assert_eq!(result.level_gap(), 0.0);
            assert_eq!(result.assignments.len(), capacity * 2);
        }
        let even: Vec<_> = (0..1000).map(|i| team(1, 2 * (i % 8))).collect();
        let result = place([team(1, 0), team(1, 1)], &even, 10).unwrap();
        assert_eq!(result.population(), 20);
        assert!((result.level_gap() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn solver_matches_exhaustive_assignments_for_small_varied_queues() {
        // Independent oracle: enumerate every skip/A/B assignment, including
        // uneven initial teams, mixed squad sizes, levels and capacities.
        let mut seed = 7u32;
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            seed
        };
        for _ in 0..200 {
            let capacity = 2 + next() as usize % 4;
            let initial = [
                team(next() as usize % 3, next() % 30),
                team(next() as usize % 3, next() % 30),
            ];
            let groups: Vec<_> = (0..5)
                .map(|_| team(1 + next() as usize % 3, next() % 40))
                .collect();
            let mut oracle: Option<(usize, f64)> = None;
            for mut mask in 1..3usize.pow(groups.len() as u32) {
                let mut teams = initial;
                for &group in &groups {
                    match mask % 3 {
                        1 => teams[0].add(group),
                        2 => teams[1].add(group),
                        _ => {}
                    }
                    mask /= 3;
                }
                if teams.iter().any(|t| t.players == 0 || t.players > capacity)
                    || teams[0].players.abs_diff(teams[1].players) > 1
                {
                    continue;
                }
                let population = teams.iter().map(|t| t.players).sum();
                let gap = (teams[0].average() - teams[1].average()).abs();
                if oracle.is_none_or(|(p, g)| population > p || (population == p && gap < g)) {
                    oracle = Some((population, gap));
                }
            }
            let result = place(initial, &groups, capacity);
            assert_eq!(
                result.as_ref().map(|p| p.population()),
                oracle.map(|(p, _)| p)
            );
            if let (Some(result), Some((_, gap))) = (result, oracle) {
                assert!((result.level_gap() - gap).abs() < 1e-9);
                let mut reconstructed = initial;
                let mut used = std::collections::HashSet::new();
                for (index, team) in result.assignments {
                    assert!(used.insert(index));
                    reconstructed[usize::from(team)].add(groups[index]);
                }
                assert_eq!(reconstructed, result.teams);
            }
        }
    }
}
