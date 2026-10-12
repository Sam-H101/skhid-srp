//! What a match earns, and what it used up. Given when the player is back in
//! the lobby, from the results the match server kept (`MatchResult`).
//!
//! - Experience is the match's points, as the Survarium wiki listed them in
//!   2015 (no 2013 figures survive): 10 a kill, 35 a battery taken from the
//!   enemy base. Assists aren't counted: this build has none.
//! - Levels follow the same page: the first takes 500 experience, each next
//!   250 more (listed to level 15; srp carries the rule on). A level gives a
//!   skill point (the 2013 wiki says so too) and the page's level reward.
//! - Money isn't from a source: 100 for staying to the end of a match, and
//!   10 a point (100 a kill).
//! - Rounds, drugs and traps used come off the stacks they were taken from.

use super::connection_state::ConnectionState;
use session::MatchResult;
use survarium::player_profile::raw::inventory_item_instance;

pub const POINTS_PER_KILL: u32 = 10;
pub const POINTS_PER_STEAL: u32 = 35;
pub const MONEY_FOR_FINISHING: u32 = 100;
pub const MONEY_PER_POINT: u32 = 10;

/// The experience `level` takes over the one before it.
pub fn experience_for_level(level: u32) -> u32 {
    500 + 250 * level.saturating_sub(1)
}

/// The level reached with `total` experience, and where it starts and the
/// next one does.
pub fn level_at(total: u32) -> (u32, u32, u32) {
    let (mut level, mut start, mut next) = (0, 0_u32, experience_for_level(1));
    while total >= next {
        level += 1;
        start = next;
        next = next.saturating_add(experience_for_level(level + 1));
        if next == u32::MAX {
            break;
        }
    }
    (level, start, next)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelReward {
    Money(u32),
    Premium(u32),
}

/// The 2015 wiki's reward for reaching `level`: 100 gold at level 1 and
/// every fifth, 250 at 100; otherwise money, 500 at levels 2-4 and 250
/// more every five levels, to 5,000.
pub fn level_reward(level: u32) -> Option<LevelReward> {
    match level {
        0 => None,
        100 => Some(LevelReward::Premium(250)),
        101.. => None,
        _ if level == 1 || level.is_multiple_of(5) => Some(LevelReward::Premium(100)),
        _ => Some(LevelReward::Money(
            (500 + 250 * ((level - 1) / 5)).min(5_000),
        )),
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reward {
    pub experience: u32,
    pub money: u32,
    /// Each level reached, with its reward.
    pub levels: Vec<(u32, Option<LevelReward>)>,
}

impl Reward {
    /// The client's stats notice (`lobby_menu::on_stats_message_arrived`): it
    /// shows this in the chat when the name is its own, and the experience
    /// as the gain on its level bar. English: the client decodes the text
    /// with the system's code page.
    pub fn notice(&self, name: &str) -> String {
        let mut text = format!(
            "Player [ {name} ] match reward: +{} experience, +{} money",
            self.experience, self.money
        );
        for (level, reward) in &self.levels {
            text += &format!("; level {level}");
            match reward {
                Some(LevelReward::Money(money)) => text += &format!(": +{money} money"),
                Some(LevelReward::Premium(gold)) => text += &format!(": +{gold} gold"),
                None => {}
            }
        }
        text + &format!(". #e:[{}]", self.experience)
    }
}

/// Give a player what its matches earned and take what they used up.
pub fn apply(state: &mut ConnectionState, results: &[MatchResult]) -> Reward {
    let mut reward = Reward::default();
    for result in results {
        let points = result
            .kills
            .saturating_mul(POINTS_PER_KILL)
            .saturating_add(result.steals.saturating_mul(POINTS_PER_STEAL));
        reward.experience = reward.experience.saturating_add(points);
        reward.money = reward
            .money
            .saturating_add(points.saturating_mul(MONEY_PER_POINT))
            .saturating_add(if result.finished {
                MONEY_FOR_FINISHING
            } else {
                0
            });
        for &(dict_id, count) in &result.used {
            use_up(state, dict_id, count);
        }
    }

    let (before, ..) = level_at(state.total_experience);
    state.total_experience = state.total_experience.saturating_add(reward.experience);
    let (after, start, next) = level_at(state.total_experience);
    state.prev_level_experience = start;
    state.next_level_experience = next;
    state.generic_money = state.generic_money.saturating_add(reward.money);
    for level in before + 1..=after {
        let level_reward = level_reward(level);
        match level_reward {
            Some(LevelReward::Money(money)) => {
                state.generic_money = state.generic_money.saturating_add(money);
            }
            Some(LevelReward::Premium(gold)) => {
                state.premium_money = state.premium_money.saturating_add(gold);
            }
            None => {}
        }
        state.skills_points = state.skills_points.saturating_add(1);
        reward.levels.push((level, level_reward));
    }
    reward
}

/// Take `count` of an item off the stacks: first those the selected profile
/// carries, then any other. A stack keeps at most what is left equipped in
/// each profile; an empty one is gone, and so are the slots that held it.
fn use_up(state: &mut ConnectionState, dict_id: u16, count: u32) {
    let carried: Vec<u32> = state
        .profile_contents
        .iter()
        .filter(|profile| profile.profile_id == state.selected_profile_id)
        .flat_map(|profile| profile.slots.iter())
        .filter(|slot| slot.id != 0 && slot.dict_id == dict_id)
        .map(|slot| slot.id)
        .collect();
    let mut stacks: Vec<u32> = state
        .inventory
        .iter()
        .filter(|item| item.dict_id == dict_id)
        .map(|item| item.id)
        .collect();
    stacks.sort_by_key(|id| !carried.contains(id));

    let mut left = count;
    for id in stacks {
        if left == 0 {
            break;
        }
        let Some(item) = state.inventory.iter_mut().find(|item| item.id == id) else {
            continue;
        };
        let taken = left.min(item.condition_or_stack);
        left -= taken;
        let remaining = item.condition_or_stack - taken;
        item.condition_or_stack = remaining;
        item.amount_in_inventory = remaining;
        if remaining == 0 {
            state.inventory.retain(|item| item.id != id);
        }
        for profile in &mut state.profile_contents {
            let mut free = remaining;
            for slot in profile.slots.iter_mut().filter(|slot| slot.id == id) {
                if remaining == 0 {
                    *slot = inventory_item_instance::default();
                    continue;
                }
                slot.amount_in_inventory = remaining;
                slot.condition_or_stack = slot.condition_or_stack.min(free);
                free -= slot.condition_or_stack;
                if slot.condition_or_stack == 0 {
                    *slot = inventory_item_instance::default();
                }
            }
        }
    }
    if left > 0 {
        log::debug!("{left} of item {dict_id} used beyond what the account had");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::Account;
    use survarium::player_profile::raw::profile_slot_enum;

    #[test]
    fn levels_follow_the_2015_table() {
        assert_eq!(level_at(0), (0, 0, 500));
        assert_eq!(level_at(499), (0, 0, 500));
        assert_eq!(level_at(500), (1, 500, 1250));
        assert_eq!(level_at(1250), (2, 1250, 2250));
        // Level 15 takes 4,000 over level 14.
        assert_eq!(experience_for_level(15), 4_000);
        assert_eq!(level_at(u32::MAX).0, level_at(u32::MAX - 1).0);
    }

    #[test]
    fn level_rewards_follow_the_2015_table() {
        assert_eq!(level_reward(1), Some(LevelReward::Premium(100)));
        assert_eq!(level_reward(2), Some(LevelReward::Money(500)));
        assert_eq!(level_reward(4), Some(LevelReward::Money(500)));
        assert_eq!(level_reward(5), Some(LevelReward::Premium(100)));
        assert_eq!(level_reward(6), Some(LevelReward::Money(750)));
        assert_eq!(level_reward(14), Some(LevelReward::Money(1_000)));
        assert_eq!(level_reward(94), Some(LevelReward::Money(5_000)));
        assert_eq!(level_reward(99), Some(LevelReward::Money(5_000)));
        assert_eq!(level_reward(100), Some(LevelReward::Premium(250)));
        assert_eq!(level_reward(101), None);
    }

    fn connection() -> ConnectionState {
        ConnectionState::from_account(0xDD00, &Account::new_dummy(1, "fighter".to_owned()))
    }

    #[test]
    fn kills_and_steals_earn_experience_money_and_levels() {
        let mut state = connection();
        let (money, gold, points) = (
            state.generic_money,
            state.premium_money,
            state.skills_points,
        );
        let reward = apply(
            &mut state,
            &[
                MatchResult {
                    kills: 5,
                    steals: 1,
                    finished: true,
                    used: Vec::new(),
                },
                MatchResult {
                    kills: 1,
                    ..MatchResult::default()
                },
            ],
        );

        // 6 kills and a steal: 95 points.
        assert_eq!(reward.experience, 95);
        assert_eq!(reward.money, 100 + 950);
        assert_eq!(state.total_experience, 95);
        assert_eq!(state.generic_money, money + 1_050);
        assert!(reward.levels.is_empty());

        // Over 500: level 1, with its gold and a skill point.
        let reward = apply(
            &mut state,
            &[MatchResult {
                kills: 50,
                ..MatchResult::default()
            }],
        );
        assert_eq!(reward.levels, [(1, Some(LevelReward::Premium(100)))]);
        assert_eq!(state.premium_money, gold + 100);
        assert_eq!(state.skills_points, points + 1);
        assert_eq!(
            (state.prev_level_experience, state.next_level_experience),
            (500, 1250)
        );
    }

    #[test]
    fn the_notice_is_the_one_the_client_reads() {
        let reward = Reward {
            experience: 20,
            money: 300,
            levels: vec![(2, Some(LevelReward::Money(500)))],
        };
        assert_eq!(
            reward.notice("stepan"),
            "Player [ stepan ] match reward: +20 experience, +300 money; level 2: +500 money. \
             #e:[20]"
        );
    }

    #[test]
    fn used_supplies_come_off_the_carried_stack_and_its_slots() {
        let mut state = connection();
        let slot = |state: &ConnectionState| {
            state.profile_contents[0].slots[profile_slot_enum::ammo1_weapon1_slot]
        };
        state.selected_profile_id = state.profile_contents[0].profile_id;
        let ammo = slot(&state);
        let owned = ammo.amount_in_inventory;

        apply(
            &mut state,
            &[MatchResult {
                used: vec![(ammo.dict_id, 25)],
                ..MatchResult::default()
            }],
        );
        let stack = state
            .inventory
            .iter()
            .find(|item| item.id == ammo.id)
            .unwrap();
        assert_eq!(stack.condition_or_stack, owned - 25);
        assert_eq!(slot(&state).amount_in_inventory, owned - 25);
        assert_eq!(slot(&state).condition_or_stack, ammo.condition_or_stack);

        // Down to fewer than the loadout carries: the loadout shrinks.
        apply(
            &mut state,
            &[MatchResult {
                used: vec![(ammo.dict_id, owned - 25 - 40)],
                ..MatchResult::default()
            }],
        );
        let slot_now = slot(&state);
        assert_eq!(slot_now.amount_in_inventory, 40);
        // The profile's two slots of this stack share the 40: the first takes
        // them, and the emptied second is cleared.
        assert_eq!(slot_now.condition_or_stack, 40);
        let other = state.profile_contents[0].slots[profile_slot_enum::ammo2_weapon1_slot];
        assert_eq!(other.id, 0);

        // Used up: the stack and its slots are gone, and more is no harm.
        apply(
            &mut state,
            &[MatchResult {
                used: vec![(ammo.dict_id, 1_000)],
                ..MatchResult::default()
            }],
        );
        assert!(!state.inventory.iter().any(|item| item.id == ammo.id));
        assert_eq!(slot(&state).id, 0);
    }
}
