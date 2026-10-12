// Battery collection (`gather_victory_items`): where every battery of one match
// is, and what a player's "use" does with them.
//
// The stock client never asks to take or put a battery. It only sends its use
// key (`USE_ACTION` in `client_player_update`'s action mask) with its position;
// the original server ran its own use check and told every client the result
// (`victory_item_take_or_put`), from which the client draws the carry icon,
// prompts, combat log, minimap and team scores.
//
// The original check is a 1 m ray from the head bone of the animated body
// against the map's use zones (a battery's 1 m sphere, each base's box). srp
// has no animated body, so it uses the same zones and reach from an
// approximate eye position instead: looking at the object is not required.

use survarium::player_profile::raw::game_team_id;
use vostok::math::float3;

use crate::map::{self, BatteryBase};
use crate::message::{
    ServerGameMessageKind, VictoryItemAction, VictoryItemPlacement, VictoryItemsContainerContents,
};

/// The use key's bit in the client's action mask (`player::detect_usable_objects`).
pub const USE_ACTION: u32 = 0x1000_0000;
/// Eye height above the position a client reports (its feet), standing.
pub const EYE_HEIGHT: f32 = 1.6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    Ground(float3),
    /// Carried by this player id.
    Carried(u8),
    /// Stored in the base with this container id.
    Stored(u8),
}

/// What a player's use acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseTarget {
    Battery(u8),
    Base(u8),
}

#[derive(Clone, Debug)]
pub struct Batteries {
    /// Indexed by battery (victory item) id.
    places: Vec<Place>,
    /// Each base's stored batteries by container id, oldest first.
    stored: Vec<(u8, Vec<u8>)>,
}

impl Batteries {
    /// `count` batteries at distinct spawners; `pick(n)` chooses one of `n`.
    pub fn spawn(count: u8, mut pick: impl FnMut(usize) -> usize) -> Self {
        let mut free: Vec<usize> = (0..map::BATTERY_SPAWNS.len()).collect();
        let count = usize::from(count).min(free.len());
        let places = (0..count)
            .map(|_| Place::Ground(map::BATTERY_SPAWNS[free.swap_remove(pick(free.len()))]))
            .collect();
        Self {
            places,
            stored: map::BATTERY_BASES
                .iter()
                .map(|base| (base.id, Vec::new()))
                .collect(),
        }
    }

    #[cfg(test)]
    pub fn places(&self) -> &[Place] {
        &self.places
    }

    pub fn carried_by(&self, player_id: u8) -> Option<u8> {
        self.places
            .iter()
            .position(|place| *place == Place::Carried(player_id))
            .map(|item_id| item_id as u8)
    }

    fn stored_in(&self, base_id: u8) -> &[u8] {
        self.stored
            .iter()
            .find(|(id, _)| *id == base_id)
            .map_or(&[], |(_, items)| items)
    }

    fn stored_in_mut(&mut self, base_id: u8) -> Option<&mut Vec<u8>> {
        self.stored
            .iter_mut()
            .find(|(id, _)| *id == base_id)
            .map(|(_, items)| items)
    }

    /// A team's score: the batteries stored in its base.
    pub fn points(&self, team: game_team_id) -> i8 {
        map::battery_base_of(team).map_or(0, |base| self.stored_in(base.id).len() as i8)
    }

    /// The team that has stored every battery, if any.
    pub fn winner(&self) -> Option<game_team_id> {
        map::BATTERY_BASES
            .iter()
            .find(|base| self.stored_in(base.id).len() == self.places.len())
            .map(|base| base.team)
    }

    /// The whole state for one client; `position_of` gives a carrier's
    /// position (the client ignores it for carried batteries).
    pub fn initialize_message(&self, position_of: impl Fn(u8) -> float3) -> ServerGameMessageKind {
        let items = self
            .places
            .iter()
            .enumerate()
            .filter_map(|(item_id, place)| {
                let (carrier, position) = match *place {
                    Place::Ground(position) => (None, position),
                    Place::Carried(player_id) => (Some(player_id), position_of(player_id)),
                    Place::Stored(_) => return None,
                };
                Some(VictoryItemPlacement {
                    carrier,
                    item_id: item_id as u8,
                    position,
                })
            })
            .collect();
        ServerGameMessageKind::InitializeVictoryItems {
            team_1_points: self.points(game_team_id::team_1),
            team_2_points: self.points(game_team_id::team_2),
            items,
            containers: self
                .stored
                .iter()
                .map(|(container_id, item_ids)| VictoryItemsContainerContents {
                    container_id: *container_id,
                    item_ids: item_ids.clone(),
                })
                .collect(),
        }
    }

    /// What a `team` player with eyes at `eye` would use now, mirroring the
    /// client's prompts: a carrier can only store at its own base; anyone else
    /// picks up the nearest ground battery or steals from a stocked enemy base.
    pub fn target(
        &self,
        player_id: u8,
        team: game_team_id,
        eye: float3,
        reach: f32,
    ) -> Option<UseTarget> {
        if self.carried_by(player_id).is_some() {
            return map::battery_base_of(team)
                .filter(|base| distance_to_base(eye, base) <= reach)
                .map(|base| UseTarget::Base(base.id));
        }

        let batteries = self
            .places
            .iter()
            .enumerate()
            .filter_map(|(item_id, place)| match place {
                Place::Ground(position) => Some((
                    UseTarget::Battery(item_id as u8),
                    distance_to_battery(eye, *position),
                )),
                _ => None,
            });
        let enemy_bases = map::BATTERY_BASES
            .iter()
            .filter(|base| base.team != team && !self.stored_in(base.id).is_empty())
            .map(|base| (UseTarget::Base(base.id), distance_to_base(eye, base)));
        batteries
            .chain(enemy_bases)
            .filter(|(_, distance)| *distance <= reach)
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(target, _)| target)
    }

    /// Use `target` (from `target`): returns the battery and what happened to
    /// it, or `None` if the use does nothing.
    pub fn use_target(
        &mut self,
        player_id: u8,
        team: game_team_id,
        target: UseTarget,
    ) -> Option<(u8, VictoryItemAction)> {
        let carried = self.carried_by(player_id);
        match target {
            UseTarget::Battery(item_id) => {
                let place = self.places.get_mut(usize::from(item_id))?;
                if carried.is_some() || !matches!(place, Place::Ground(_)) {
                    return None;
                }
                *place = Place::Carried(player_id);
                Some((item_id, VictoryItemAction::TakeFromGround))
            }
            UseTarget::Base(container_id) => {
                let base = map::battery_base(container_id)?;
                match carried {
                    Some(item_id) if base.team == team => {
                        self.stored_in_mut(container_id)?.push(item_id);
                        self.places[usize::from(item_id)] = Place::Stored(container_id);
                        Some((item_id, VictoryItemAction::PutInBase { container_id }))
                    }
                    None if base.team != team => {
                        let item_id = self.stored_in_mut(container_id)?.pop()?;
                        self.places[usize::from(item_id)] = Place::Carried(player_id);
                        Some((item_id, VictoryItemAction::TakeFromBase { container_id }))
                    }
                    _ => None,
                }
            }
        }
    }

    /// Drop `player_id`'s battery (if any) on the ground at `position`.
    pub fn drop_carried(&mut self, player_id: u8, position: float3) -> Option<u8> {
        let item_id = self.carried_by(player_id)?;
        self.places[usize::from(item_id)] = Place::Ground(position);
        Some(item_id)
    }

    /// Distances from `eye` to the nearest ground battery and to each base,
    /// for logging use presses.
    pub fn describe_reach(&self, eye: float3) -> String {
        let nearest = self
            .places
            .iter()
            .enumerate()
            .filter_map(|(item_id, place)| match place {
                Place::Ground(position) => Some((item_id, distance_to_battery(eye, *position))),
                _ => None,
            })
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map_or(
                "no battery on the ground".to_owned(),
                |(item_id, distance)| format!("battery {item_id} {distance:.2} m"),
            );
        let bases: Vec<String> = map::BATTERY_BASES
            .iter()
            .map(|base| format!("{:?} base {:.2} m", base.team, distance_to_base(eye, base)))
            .collect();
        format!("nearest {nearest}, {}", bases.join(", "))
    }

    /// Every battery's place and the scores, for the console and logs.
    pub fn describe(&self) -> String {
        let places: Vec<String> = self
            .places
            .iter()
            .enumerate()
            .map(|(item_id, place)| match place {
                Place::Ground(p) => {
                    format!("{item_id}: ground ({:.1}, {:.1}, {:.1})", p.x, p.y, p.z)
                }
                Place::Carried(player_id) => format!("{item_id}: carried by player {player_id}"),
                Place::Stored(base_id) => format!("{item_id}: in base {base_id}"),
            })
            .collect();
        format!(
            "team_1 {} / team_2 {} of {}; {}",
            self.points(game_team_id::team_1),
            self.points(game_team_id::team_2),
            self.places.len(),
            places.join("; ")
        )
    }
}

/// Eye position of a player standing at `feet`.
pub fn eye_at(feet: float3) -> float3 {
    float3 {
        y: feet.y + EYE_HEIGHT,
        ..feet
    }
}

fn length(x: f32, y: f32, z: f32) -> f32 {
    (x * x + y * y + z * z).sqrt()
}

/// From `eye` to the surface of a battery's use sphere (0 inside it).
fn distance_to_battery(eye: float3, battery: float3) -> f32 {
    let center = length(eye.x - battery.x, eye.y - battery.y, eye.z - battery.z);
    (center - map::BATTERY_USE_RADIUS).max(0.0)
}

/// From `eye` to a base's use box (0 inside it).
fn distance_to_base(eye: float3, base: &BatteryBase) -> f32 {
    let outside = |eye: f32, center: f32, half: f32| ((eye - center).abs() - half).max(0.0);
    let (center, half) = (base.use_zone_center, base.use_zone_half_extents);
    length(
        outside(eye.x, center.x, half.x),
        outside(eye.y, center.y, half.y),
        outside(eye.z, center.z, half.z),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: u8 = 3;
    const REACH: f32 = 1.0;

    fn base(team: game_team_id) -> &'static BatteryBase {
        map::battery_base_of(team).unwrap()
    }

    /// Feet just beside a point, so the eyes are within reach of it.
    fn standing_at(point: float3) -> float3 {
        eye_at(float3 {
            x: point.x + 0.5,
            y: point.y - 0.8,
            ..point
        })
    }

    fn in_order(count: u8) -> Batteries {
        Batteries::spawn(count, |_| 0)
    }

    fn battery_position(batteries: &Batteries, item_id: usize) -> float3 {
        let Place::Ground(position) = batteries.places()[item_id] else {
            panic!("battery {item_id} is not on the ground");
        };
        position
    }

    #[test]
    fn spawns_each_battery_at_a_different_spawner() {
        let batteries = Batteries::spawn(map::BATTERY_SPAWNS.len() as u8 + 5, |n| n / 2);
        assert_eq!(batteries.places().len(), map::BATTERY_SPAWNS.len());
        for (index, place) in batteries.places().iter().enumerate() {
            let Place::Ground(position) = place else {
                panic!("{place:?}")
            };
            assert!(
                !batteries.places()[..index].contains(&Place::Ground(*position)),
                "two batteries at {position:?}"
            );
        }
    }

    #[test]
    fn picks_up_a_battery_within_reach_only() {
        let mut batteries = in_order(2);
        let position = battery_position(&batteries, 0);
        let team = game_team_id::team_1;

        let far = eye_at(float3 {
            x: position.x + 3.0,
            ..position
        });
        assert_eq!(batteries.target(PLAYER, team, far, REACH), None);

        let near = standing_at(position);
        assert_eq!(
            batteries.target(PLAYER, team, near, REACH),
            Some(UseTarget::Battery(0))
        );
        assert_eq!(
            batteries.use_target(PLAYER, team, UseTarget::Battery(0)),
            Some((0, VictoryItemAction::TakeFromGround))
        );
        assert_eq!(batteries.carried_by(PLAYER), Some(0));
        assert_eq!(
            batteries.use_target(PLAYER + 1, team, UseTarget::Battery(0)),
            None,
            "a carried battery cannot be picked up"
        );
    }

    #[test]
    fn a_carrier_cannot_pick_up_another_battery() {
        let mut batteries = in_order(2);
        let team = game_team_id::team_1;
        batteries.use_target(PLAYER, team, UseTarget::Battery(0));

        let second = standing_at(battery_position(&batteries, 1));
        assert_eq!(batteries.target(PLAYER, team, second, REACH), None);
        assert_eq!(
            batteries.use_target(PLAYER, team, UseTarget::Battery(1)),
            None
        );
    }

    #[test]
    fn stores_at_its_own_base_only_and_scores() {
        let mut batteries = in_order(2);
        let team = game_team_id::team_1;
        batteries.use_target(PLAYER, team, UseTarget::Battery(0));

        let enemy = base(game_team_id::team_2);
        let at_enemy = standing_at(enemy.use_zone_center);
        assert_eq!(batteries.target(PLAYER, team, at_enemy, REACH), None);

        let own = base(team);
        let at_own = standing_at(own.use_zone_center);
        assert_eq!(
            batteries.target(PLAYER, team, at_own, REACH),
            Some(UseTarget::Base(own.id))
        );
        assert_eq!(
            batteries.use_target(PLAYER, team, UseTarget::Base(own.id)),
            Some((
                0,
                VictoryItemAction::PutInBase {
                    container_id: own.id
                }
            ))
        );
        assert_eq!(batteries.carried_by(PLAYER), None);
        assert_eq!(batteries.points(team), 1);
        assert_eq!(batteries.points(game_team_id::team_2), 0);
        assert_eq!(batteries.winner(), None);
    }

    #[test]
    fn an_enemy_steals_the_newest_stored_battery() {
        let mut batteries = in_order(3);
        let defender = game_team_id::team_1;
        let own = base(defender);
        for item_id in [0, 1] {
            batteries.use_target(PLAYER, defender, UseTarget::Battery(item_id));
            batteries.use_target(PLAYER, defender, UseTarget::Base(own.id));
        }

        let thief = PLAYER + 1;
        let attacker = game_team_id::team_2;
        let at_base = standing_at(own.use_zone_center);
        assert_eq!(
            batteries.target(PLAYER, defender, at_base, REACH),
            None,
            "a team cannot take back its own stored batteries"
        );
        assert_eq!(
            batteries.target(thief, attacker, at_base, REACH),
            Some(UseTarget::Base(own.id))
        );
        assert_eq!(
            batteries.use_target(thief, attacker, UseTarget::Base(own.id)),
            Some((
                1,
                VictoryItemAction::TakeFromBase {
                    container_id: own.id
                }
            ))
        );
        assert_eq!(batteries.points(defender), 1);
        assert_eq!(batteries.carried_by(thief), Some(1));
    }

    #[test]
    fn a_dropped_battery_lies_where_its_carrier_was() {
        let mut batteries = in_order(1);
        batteries.use_target(PLAYER, game_team_id::team_1, UseTarget::Battery(0));
        let there = float3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        };

        assert_eq!(batteries.drop_carried(PLAYER, there), Some(0));
        assert_eq!(batteries.places()[0], Place::Ground(there));
        assert_eq!(batteries.drop_carried(PLAYER, there), None);
    }

    #[test]
    fn storing_every_battery_wins() {
        let mut batteries = in_order(2);
        let team = game_team_id::team_2;
        let own = base(team);
        for item_id in [0, 1] {
            batteries.use_target(PLAYER, team, UseTarget::Battery(item_id));
            batteries.use_target(PLAYER, team, UseTarget::Base(own.id));
        }
        assert_eq!(batteries.winner(), Some(team));
    }

    #[test]
    fn the_state_lists_ground_and_carried_batteries_and_each_base() {
        let mut batteries = in_order(3);
        let team = game_team_id::team_1;
        let own = base(team);
        let ground = battery_position(&batteries, 2);
        batteries.use_target(PLAYER, team, UseTarget::Battery(0));
        batteries.use_target(PLAYER, team, UseTarget::Base(own.id));
        batteries.use_target(PLAYER, team, UseTarget::Battery(1));
        let carrier_at = float3 {
            x: 5.0,
            y: 0.5,
            z: 5.0,
        };

        let ServerGameMessageKind::InitializeVictoryItems {
            team_1_points,
            team_2_points,
            items,
            containers,
        } = batteries.initialize_message(|_| carrier_at)
        else {
            panic!("expected InitializeVictoryItems");
        };
        assert_eq!((team_1_points, team_2_points), (1, 0));
        assert_eq!(
            items,
            [
                VictoryItemPlacement {
                    carrier: Some(PLAYER),
                    item_id: 1,
                    position: carrier_at,
                },
                VictoryItemPlacement {
                    carrier: None,
                    item_id: 2,
                    position: ground,
                },
            ]
        );
        assert_eq!(containers.len(), map::BATTERY_BASES.len());
        assert!(containers.contains(&VictoryItemsContainerContents {
            container_id: own.id,
            item_ids: vec![0],
        }));
    }
}
