//! Daily faction quests, from the game's `gameplay/quests/quests.options`
//! (`docs/notes/progression.md`).
//!
//! Each faction offers one active quest from the band for the player's
//! reputation level with it, and the rest of that band as proposals the
//! player can swap to. A quest lives `ttl_minutes` from when it is given. At
//! a match's end each active quest the player's gear faction allows is
//! checked as the client's own HUD counts it; a completed one pays its
//! awards, rests for its cooldown and makes way for a new one. An active
//! quest that runs out fails. Doing, changing and failing quests move the
//! faction's loyalty (`loyalty`), which adds to the reputation it gives.

use binary_config_parser::tree::{ConfigError, ConfigValue};

use super::account::Account;
use super::drops::Rng;
use super::loyalty;
use super::results::{PlayerStats, add_reputation, reputation_level};

const QUESTS: &[u8] =
    include_bytes!("../../../../resources/binary-config-examples/quests_021d.options");

/// The factions whose quests the lobby shows.
pub const FACTIONS: [u8; 4] = [1, 2, 3, 4];

/// `quest_type` values the data uses.
pub mod kind {
    pub const BRING_ITEMS: u32 = 6;
    pub const FIND_ARTEFACTS: u32 = 10;
    pub const KILL_ENEMIES: u32 = 12;
    pub const KILL_ENEMY_X: u32 = 15;
    pub const HEADSHOTS: u32 = 16;
    pub const KILLSTREAK: u32 = 19;
    pub const WIN_GAMES: u32 = 28;
    pub const KD_RATIO: u32 = 29;
    pub const ACCURACY: u32 = 30;
    pub const WIN_SCORE: u32 = 32;
    pub const DEFEND_ITEMS: u32 = 33;
}

/// Precondition and award types.
const COOLDOWN: i32 = 0;
const MINIMUM_REPUTATION: i32 = 1;
const MAXIMUM_REPUTATION: i32 = 2;
const AWARD_REPUTATION: i32 = 1;
const AWARD_MONEY: i32 = 2;

/// Important message types the client acts on.
pub const QUEST_MESSAGE: u8 = 1;
pub const REPUTATION_LEVEL_MESSAGE: u8 = 4;

/// A text the chat must deliver as an important message.
pub type Notice = (u8, String);

#[derive(Clone, Debug, PartialEq)]
pub struct Quest {
    pub id: u32,
    pub faction: u8,
    pub kind: u32,
    pub n: u32,
    pub x: u32,
    pub y: f32,
    pub max_games: u32,
    /// 0, or the faction whose gear the player must wear.
    pub representative: u8,
    pub ttl_minutes: u32,
    pub cooldown_minutes: u32,
    /// The reputation levels, with `level_faction`, it is offered at.
    pub level_faction: u8,
    pub min_level: u8,
    pub max_level: u8,
    pub money: u32,
    /// `(faction, amount)`.
    pub reputation: Vec<(u8, u32)>,
}

impl Quest {
    /// Whether the goal needs the battle won (the goals' texts).
    pub fn needs_win(&self) -> bool {
        matches!(
            self.kind,
            kind::WIN_GAMES
                | kind::KD_RATIO
                | kind::ACCURACY
                | kind::WIN_SCORE
                | kind::DEFEND_ITEMS
        )
    }

    /// Whether one match's play meets the goal (the client's HUD rules,
    /// `calculate_player_game_progress` 0x1B3859). A won battle counts for
    /// `win_x_games` through [`QuestInstance::games`] instead.
    pub fn met_by(&self, stats: &PlayerStats) -> bool {
        if self.needs_win() && !stats.won {
            return false;
        }
        let n = self.n;
        match self.kind {
            kind::BRING_ITEMS => u32::from(stats.delivered) >= n,
            kind::FIND_ARTEFACTS => u32::from(stats.artefacts) >= n,
            kind::KILL_ENEMIES => u32::from(stats.kills) >= n,
            kind::KILL_ENEMY_X => stats
                .faction_kills
                .get(self.x as usize)
                .is_some_and(|&kills| u32::from(kills) >= n),
            kind::HEADSHOTS => u32::from(stats.headshots) >= n,
            kind::KILLSTREAK => u32::from(stats.max_killstreak) >= n,
            kind::KD_RATIO => {
                // As a float divide: no deaths is infinite, unless no kills.
                if stats.deaths == 0 {
                    stats.kills > 0
                } else {
                    f32::from(stats.kills) / f32::from(stats.deaths) > self.y
                }
            }
            kind::ACCURACY => {
                stats.shots > 0
                    && f32::from(stats.hits) * 100.0 / f32::from(stats.shots) >= self.y * 100.0
            }
            kind::WIN_SCORE => u32::from(stats.team_batteries) >= n,
            kind::DEFEND_ITEMS => !stats.base_robbed,
            kind::WIN_GAMES => true,
            _ => false,
        }
    }
}

/// A quest given to an account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestInstance {
    pub instance_id: u32,
    pub quest_id: u32,
    /// Battles won towards `win_x_games`.
    pub games: u32,
    /// Unix seconds.
    pub expires_at: u64,
    /// Active, or a proposal to swap to.
    pub active: bool,
}

fn int(value: &ConfigValue, key: &str) -> Result<i32, ConfigError> {
    value.i32_at(key)
}

fn parse_quest(quest: &ConfigValue) -> Result<Quest, ConfigError> {
    let params = quest.req("params")?;
    let mut parsed = Quest {
        id: int(quest, "id")? as u32,
        faction: int(quest, "faction")? as u8,
        kind: int(quest, "type")? as u32,
        n: int(params, "n")?.max(0) as u32,
        x: int(params, "x")?.max(0) as u32,
        y: params.f32_at("y")?,
        max_games: int(params, "max_games_count")?.max(1) as u32,
        representative: int(quest, "faction_representative")? as u8,
        ttl_minutes: int(quest, "ttl_minutes")?.max(1) as u32,
        cooldown_minutes: 0,
        level_faction: 0,
        min_level: 0,
        max_level: u8::MAX,
        money: 0,
        reputation: Vec::new(),
    };
    parsed.level_faction = parsed.faction;
    for condition in quest.req("preconditions")?.children() {
        let n = int(condition, "n")?.max(0);
        match int(condition, "type")? {
            COOLDOWN => parsed.cooldown_minutes = n as u32,
            MINIMUM_REPUTATION => {
                parsed.min_level = n as u8;
                parsed.level_faction = int(condition, "x")? as u8;
            }
            MAXIMUM_REPUTATION => {
                parsed.max_level = n as u8;
                parsed.level_faction = int(condition, "x")? as u8;
            }
            _ => {}
        }
    }
    for award in quest.req("awards")?.children() {
        let amount = int(award, "amount")?.max(0) as u32;
        match int(award, "type")? {
            AWARD_MONEY => parsed.money += amount,
            AWARD_REPUTATION => parsed.reputation.push((int(award, "param")? as u8, amount)),
            _ => {}
        }
    }
    Ok(parsed)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quests {
    pub quests: Vec<Quest>,
}

impl Quests {
    pub fn shared() -> &'static Self {
        static SHARED: std::sync::OnceLock<Quests> = std::sync::OnceLock::new();
        SHARED.get_or_init(|| Self::parse(QUESTS).expect("the embedded 0.21d quests parse"))
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, ConfigError> {
        let root = ConfigValue::parse(bytes)?;
        let mut quests = root
            .req("quests")?
            .children()
            .into_iter()
            .map(parse_quest)
            .collect::<Result<Vec<_>, _>>()?;
        quests.sort_by_key(|quest| quest.id);
        Ok(Self { quests })
    }

    pub fn quest(&self, id: u32) -> Option<&Quest> {
        self.quests.iter().find(|quest| quest.id == id)
    }

    /// The quests `faction` offers an account at its reputation levels.
    pub fn band<'a>(
        &'a self,
        account: &'a Account,
        faction: u8,
    ) -> impl Iterator<Item = &'a Quest> {
        self.quests.iter().filter(move |quest| {
            let level = reputation_points(account, quest.level_faction)
                .map(|points| reputation_level(quest.level_faction, points));
            quest.faction == faction
                && level.is_some_and(|level| (quest.min_level..=quest.max_level).contains(&level))
        })
    }
}

fn reputation_points(account: &Account, faction: u8) -> Option<u32> {
    account
        .reputations
        .iter()
        .find(|(id, _)| *id == faction)
        .map(|&(_, points)| points.into())
}

/// Unix seconds now.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn faction_of(quests: &Quests, instance: &QuestInstance) -> Option<u8> {
    quests.quest(instance.quest_id).map(|quest| quest.faction)
}

/// Bring the account's quests up to date at `now`: an active quest that
/// ran out fails, each faction gets an active quest if it has none, and the
/// rest of its band is offered.
pub fn refresh(account: &mut Account, now: u64, rng: &mut Rng) -> Vec<Notice> {
    let quests = Quests::shared();
    let mut notices = Vec::new();
    let mut failed = Vec::new();
    account.quest_cooldowns.retain(|&(_, until)| until > now);
    account.quests.retain(|instance| {
        let alive = instance.expires_at > now && quests.quest(instance.quest_id).is_some();
        if !alive && instance.active {
            notices.push((
                QUEST_MESSAGE,
                format!("#!q[{}][{}]", instance.instance_id, instance.quest_id),
            ));
            failed.extend(faction_of(quests, instance));
        }
        alive
    });
    for faction in failed {
        loyalty::reset(account, faction);
    }

    for faction in FACTIONS {
        let band: Vec<(u32, u32)> = quests
            .band(account, faction)
            .map(|quest| (quest.id, quest.ttl_minutes))
            .collect();
        // Proposals from a band the player has left go.
        account.quests.retain(|instance| {
            instance.active
                || faction_of(quests, instance) != Some(faction)
                || band.iter().any(|&(id, _)| id == instance.quest_id)
        });
        let available: Vec<(u32, u32)> = band
            .iter()
            .copied()
            .filter(|&(id, _)| {
                !account
                    .quest_cooldowns
                    .iter()
                    .any(|&(quest, _)| quest == id)
                    && !account
                        .quests
                        .iter()
                        .any(|instance| instance.quest_id == id)
            })
            .collect();
        let has_active = account
            .quests
            .iter()
            .any(|instance| instance.active && faction_of(quests, instance) == Some(faction));
        for &(quest_id, ttl) in &available {
            let instance_id = account.next_quest_instance;
            account.next_quest_instance += 1;
            account.quests.push(QuestInstance {
                instance_id,
                quest_id,
                games: 0,
                expires_at: now + u64::from(ttl) * 60,
                active: false,
            });
        }
        if has_active {
            continue;
        }
        // One of the faction's proposals becomes its active quest, fresh: a
        // done quest's band left as proposals too.
        let proposals: Vec<usize> = account
            .quests
            .iter()
            .enumerate()
            .filter(|(_, instance)| faction_of(quests, instance) == Some(faction))
            .map(|(index, _)| index)
            .collect();
        if proposals.is_empty() {
            continue;
        }
        let index = proposals[rng.below(proposals.len() as u32) as usize];
        let instance = &mut account.quests[index];
        let ttl = band
            .iter()
            .find(|&&(id, _)| id == instance.quest_id)
            .map_or(0, |&(_, ttl)| ttl);
        instance.active = true;
        instance.games = 0;
        instance.expires_at = now + u64::from(ttl) * 60;
    }
    notices
}

/// `2B 00`: make the proposal `chosen` the faction's active quest instead of
/// `active`. Whether it was done.
pub fn swap(account: &mut Account, active: u32, chosen: u32, now: u64) -> bool {
    let quests = Quests::shared();
    let find = |account: &Account, id: u32| {
        account
            .quests
            .iter()
            .position(|instance| instance.instance_id == id)
    };
    let (Some(from), Some(to)) = (find(account, active), find(account, chosen)) else {
        return false;
    };
    let (old, new) = (&account.quests[from], &account.quests[to]);
    let faction = faction_of(quests, old);
    if !old.active || new.active || faction != faction_of(quests, new) {
        return false;
    }
    let Some(ttl) = quests.quest(new.quest_id).map(|quest| quest.ttl_minutes) else {
        return false;
    };
    account.quests[from].active = false;
    let new = &mut account.quests[to];
    new.active = true;
    new.games = 0;
    new.expires_at = now + u64::from(ttl) * 60;
    // The client warns of it: `st_quest_change_warning`.
    if let Some(faction) = faction {
        loyalty::lower(account, faction);
    }
    true
}

/// Check the account's active quests against a finished match. `gear` is
/// the gear faction the player wore. Completed quests pay their awards and
/// rest; new ones replace them.
pub fn evaluate(
    account: &mut Account,
    stats: &PlayerStats,
    gear: u8,
    now: u64,
    rng: &mut Rng,
) -> Vec<Notice> {
    let quests = Quests::shared();
    let mut notices = refresh(account, now, rng);
    let mut done = Vec::new();
    for instance in account.quests.iter_mut().filter(|instance| instance.active) {
        let Some(quest) = quests.quest(instance.quest_id) else {
            continue;
        };
        if quest.representative != 0 && quest.representative != gear {
            continue;
        }
        let met = if quest.kind == kind::WIN_GAMES {
            if stats.won {
                instance.games += 1;
            }
            instance.games >= quest.max_games
        } else {
            quest.met_by(stats)
        };
        if met {
            done.push((instance.instance_id, quest));
        }
    }
    for (instance_id, quest) in done {
        account
            .quests
            .retain(|instance| instance.instance_id != instance_id);
        account
            .quest_cooldowns
            .push((quest.id, now + u64::from(quest.cooldown_minutes) * 60));
        account.generic_money = account.generic_money.saturating_add(quest.money);
        notices.push((QUEST_MESSAGE, format!("#q![{instance_id}][{}]", quest.id)));
        for &(faction, amount) in &quest.reputation {
            let amount = loyalty::with_bonus(account, faction, amount);
            for level in add_reputation(account, faction, amount) {
                notices.push((REPUTATION_LEVEL_MESSAGE, format!("#ru[{faction}:{level}]")));
            }
        }
        loyalty::raise(account, quest.faction);
        log::info!(
            "{}: quest {} done: {} money, reputation {:?}",
            account.name,
            quest.id,
            quest.money,
            quest.reputation
        );
    }
    notices.extend(refresh(account, now, rng));
    notices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::Dictionary;

    fn account() -> Account {
        Account::with_test_loadout("alice", Dictionary::shared())
    }

    #[test]
    fn reads_the_021d_quests() {
        let quests = Quests::shared();
        assert_eq!(quests.quests.len(), 271);
        let first = quests.quest(10100).unwrap();
        assert_eq!((first.faction, first.kind), (1, kind::WIN_GAMES));
        assert_eq!((first.min_level, first.max_level), (0, 0));
        assert_eq!(
            (first.money, first.reputation.clone()),
            (1000, vec![(1, 100)])
        );
        assert_eq!((first.ttl_minutes, first.cooldown_minutes), (1440, 120));
        assert_eq!(first.representative, 0);
        let kd = quests.quest(10101).unwrap();
        assert!((kd.y - 0.99).abs() < 1e-6);
        // Level 1 Scavenger quests need the faction's gear.
        let higher = quests.quest(10200).unwrap();
        assert_eq!((higher.min_level, higher.max_level), (1, 1));
        assert_eq!(higher.representative, 1);
    }

    #[test]
    fn each_faction_offers_one_active_quest_from_its_band() {
        let mut account = account();
        let notices = refresh(&mut account, 1000, &mut Rng::new(5));
        assert!(notices.is_empty());
        for faction in FACTIONS {
            let mine: Vec<&QuestInstance> = account
                .quests
                .iter()
                .filter(|instance| faction_of(Quests::shared(), instance) == Some(faction))
                .collect();
            assert_eq!(mine.iter().filter(|instance| instance.active).count(), 1);
            // Level 0 bands have 7 or 8 quests.
            assert!((7..=8).contains(&mine.len()), "{faction}: {}", mine.len());
            assert!(
                mine.iter()
                    .all(|instance| instance.quest_id % 10000 / 100 == 1)
            );
        }
        // Refreshing again changes nothing.
        let before = account.quests.clone();
        refresh(&mut account, 2000, &mut Rng::new(6));
        assert_eq!(account.quests, before);
    }

    #[test]
    fn a_done_quest_is_replaced_from_the_proposals() {
        let mut account = account();
        refresh(&mut account, 0, &mut Rng::new(5));
        let quests = Quests::shared();
        let of_faction = |account: &Account| -> Vec<QuestInstance> {
            account
                .quests
                .iter()
                .filter(|instance| faction_of(quests, instance) == Some(3))
                .cloned()
                .collect()
        };
        // Faction 3's active quest done, its reputation level unchanged:
        // the rest of its band is still there, as proposals.
        let done = of_faction(&account)
            .into_iter()
            .find(|instance| instance.active)
            .unwrap();
        account
            .quests
            .retain(|instance| instance.instance_id != done.instance_id);
        account.quest_cooldowns.push((done.quest_id, 7200));
        refresh(&mut account, 60, &mut Rng::new(7));
        let mine = of_faction(&account);
        let active: Vec<_> = mine.iter().filter(|instance| instance.active).collect();
        assert_eq!(active.len(), 1);
        assert_ne!(active[0].quest_id, done.quest_id);
        assert_eq!(active[0].expires_at, 60 + 1440 * 60);
        assert!(
            mine.iter()
                .all(|instance| instance.quest_id != done.quest_id)
        );
    }

    #[test]
    fn an_active_quest_that_runs_out_fails() {
        let mut account = account();
        refresh(&mut account, 0, &mut Rng::new(5));
        let notices = refresh(&mut account, 1440 * 60, &mut Rng::new(5));
        assert_eq!(notices.len(), 4);
        assert!(
            notices
                .iter()
                .all(|(kind, text)| *kind == QUEST_MESSAGE && text.starts_with("#!q["))
        );
        assert_eq!(
            account
                .quests
                .iter()
                .filter(|instance| instance.active)
                .count(),
            4
        );
        // And takes the faction's loyalty back to 0.
        account.loyalty = [15; 4];
        refresh(&mut account, 2 * 1440 * 60, &mut Rng::new(5));
        assert_eq!(account.loyalty, [0; 4]);
    }

    #[test]
    fn a_proposal_can_replace_the_active_quest() {
        let mut account = account();
        refresh(&mut account, 0, &mut Rng::new(5));
        let quests = Quests::shared();
        let active = account
            .quests
            .iter()
            .find(|instance| instance.active && faction_of(quests, instance) == Some(1))
            .unwrap()
            .instance_id;
        let proposal = account
            .quests
            .iter()
            .find(|instance| !instance.active && faction_of(quests, instance) == Some(1))
            .unwrap()
            .instance_id;
        let other_faction = account
            .quests
            .iter()
            .find(|instance| !instance.active && faction_of(quests, instance) == Some(2))
            .unwrap()
            .instance_id;
        account.loyalty = [10, 10, 0, 0];
        assert!(!swap(&mut account, active, other_faction, 10));
        assert_eq!(account.loyalty, [10, 10, 0, 0]);
        assert!(swap(&mut account, active, proposal, 10));
        // Changing a quest costs the faction 5 % of loyalty.
        assert_eq!(account.loyalty, [5, 10, 0, 0]);
        let find = |id| {
            account
                .quests
                .iter()
                .find(|instance| instance.instance_id == id)
                .unwrap()
        };
        assert!(find(proposal).active && !find(active).active);
        assert_eq!(find(proposal).expires_at, 10 + 1440 * 60);
    }

    #[test]
    fn goals_follow_the_client_rules() {
        let quests = Quests::shared();
        let won = PlayerStats {
            won: true,
            kills: 5,
            deaths: 2,
            headshots: 3,
            max_killstreak: 2,
            shots: 10,
            hits: 4,
            delivered: 1,
            team_batteries: 6,
            ..PlayerStats::default()
        };
        let lost = PlayerStats {
            won: false,
            ..won.clone()
        };
        // kd 2.5 > 0.99 and a win.
        assert!(quests.quest(10101).unwrap().met_by(&won));
        assert!(!quests.quest(10101).unwrap().met_by(&lost));
        // One battery delivered, no win needed.
        assert!(quests.quest(10102).unwrap().met_by(&lost));
        // Nothing stolen from the base.
        assert!(quests.quest(10103).unwrap().met_by(&won));
        let robbed = PlayerStats {
            base_robbed: true,
            ..won.clone()
        };
        assert!(!quests.quest(10103).unwrap().met_by(&robbed));
        // 6 batteries home.
        assert!(quests.quest(10104).unwrap().met_by(&won));
        // 40 % accuracy ≥ 30 %.
        assert!(quests.quest(10108).unwrap().met_by(&won));
        // 5 kills.
        assert!(quests.quest(10110).unwrap().met_by(&lost));
        // One kill of a Scavenger-geared player.
        let hunter = quests.quest(20105).unwrap();
        assert!(!hunter.met_by(&won));
        let mut faction_kills = [0; 7];
        faction_kills[1] = 1;
        assert!(hunter.met_by(&PlayerStats {
            faction_kills,
            ..won.clone()
        }));
        // No deaths: any kill beats the ratio.
        let clean = PlayerStats {
            deaths: 0,
            kills: 1,
            ..won.clone()
        };
        assert!(quests.quest(10101).unwrap().met_by(&clean));
    }

    #[test]
    fn a_completed_quest_pays_and_is_replaced() {
        let mut account = account();
        let mut rng = Rng::new(5);
        refresh(&mut account, 0, &mut rng);
        let quests = Quests::shared();
        let scavengers = account
            .quests
            .iter()
            .find(|instance| instance.active && faction_of(quests, instance) == Some(1))
            .unwrap()
            .clone();
        let money = account.generic_money;
        // A match that meets every level 0 Scavenger goal.
        let stats = PlayerStats {
            won: true,
            kills: 10,
            deaths: 1,
            headshots: 10,
            max_killstreak: 10,
            shots: 10,
            hits: 10,
            delivered: 10,
            artefacts: 10,
            team_batteries: 10,
            faction_kills: [10; 7],
            ..PlayerStats::default()
        };
        let notices = evaluate(&mut account, &stats, 0, 60, &mut rng);
        assert!(notices.contains(&(
            QUEST_MESSAGE,
            format!("#q![{}][{}]", scavengers.instance_id, scavengers.quest_id)
        )));
        // Four factions' level 0 quests: 1000 money each for 1 and 2, 2000
        // for 3 and 4; Scavenger and Black Market reputation 100 reaches
        // their level 1.
        assert_eq!(account.generic_money, money + 6000);
        // Each done quest adds 5 % of loyalty to its faction.
        assert_eq!(account.loyalty, [5; 4]);
        assert!(notices.contains(&(REPUTATION_LEVEL_MESSAGE, "#ru[1:1]".into())));
        assert!(notices.contains(&(REPUTATION_LEVEL_MESSAGE, "#ru[2:1]".into())));
        assert!(
            account
                .quest_cooldowns
                .iter()
                .any(|&(id, until)| id == scavengers.quest_id && until == 60 + 120 * 60)
        );
        // Level 1 now: the Scavengers offer their level 1 band, active again.
        let active: Vec<u32> = account
            .quests
            .iter()
            .filter(|instance| instance.active && faction_of(quests, instance) == Some(1))
            .map(|instance| instance.quest_id)
            .collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0] % 10000 / 100, 2);
    }

    #[test]
    fn faction_quests_need_their_gear() {
        let mut account = account();
        account.reputations[0].1 = 100; // Scavengers level 1
        let mut rng = Rng::new(9);
        refresh(&mut account, 0, &mut rng);
        let quests = Quests::shared();
        let scavengers = account
            .quests
            .iter()
            .find(|instance| instance.active && faction_of(quests, instance) == Some(1))
            .unwrap()
            .clone();
        let stats = PlayerStats {
            won: true,
            kills: 50,
            headshots: 50,
            max_killstreak: 50,
            shots: 10,
            hits: 10,
            delivered: 50,
            artefacts: 50,
            team_batteries: 50,
            faction_kills: [50; 7],
            ..PlayerStats::default()
        };
        // Renaissance gear: the Scavenger quest is not tracked.
        let notices = evaluate(&mut account, &stats, 3, 60, &mut rng);
        assert!(
            !notices
                .iter()
                .any(|(_, text)| text.contains(&format!("[{}]", scavengers.quest_id)))
        );
        let notices = evaluate(&mut account, &stats, 1, 120, &mut rng);
        assert!(notices.iter().any(|(_, text)| text
            == &format!("#q![{}][{}]", scavengers.instance_id, scavengers.quest_id)));
    }

    #[test]
    fn loyalty_adds_to_the_reputation_a_quest_gives() {
        let mut account = account();
        account.loyalty = [20, 0, 0, 0];
        let mut rng = Rng::new(7);
        refresh(&mut account, 0, &mut rng);
        let stats = PlayerStats {
            won: true,
            kills: 10,
            deaths: 1,
            headshots: 10,
            max_killstreak: 10,
            shots: 10,
            hits: 10,
            delivered: 10,
            artefacts: 10,
            team_batteries: 10,
            faction_kills: [10; 7],
            ..PlayerStats::default()
        };
        evaluate(&mut account, &stats, 0, 60, &mut rng);
        // Level 0 quests give 100: the Scavengers' with 20 % more.
        assert_eq!(account.reputations[0], (1, 120));
        assert_eq!(account.reputations[1], (2, 100));
        assert_eq!(account.loyalty, [25, 5, 5, 5]);
    }
}
