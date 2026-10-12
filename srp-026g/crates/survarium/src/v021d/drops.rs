//! What a match lets a player find: the results screen's "Знайдено" bar and
//! the items on it, from the game's `gameplay/drops/drops.options` and each
//! item's drop value and weight (`../srp-021d/docs/notes/progression.md`).
//!
//! The bar's notches are `slots_open` (1000, 5000, 9000); the client shows
//! drop i only when the progress reaches notch i. How the original server
//! filled the bar is lost; srp makes it the score × 10, up to `drop_amount`.

use binary_config_parser::tree::{ConfigError, ConfigValue};

use super::item_params::item_params;
use crate::dictionary::{Dictionary, Item};

const DROPS: &[u8] =
    include_bytes!("../../../../resources/binary-config-examples/drops_021d.options");

/// Progress is the score times this [a choice].
pub const PROGRESS_PER_SCORE: u32 = 10;

/// srp's own bias against the best finds [a choice; the game's table alone
/// gives weapons and armour a tenth of all drops, a 5000-value rifle as
/// often as a 1000-value one]. A piece of gear (not a stack) is kept with
/// probability `CHEAP_DROP_VALUE / drop_value` once its value is above that,
/// and a rejected one is rolled again: gear worth 3000 comes a third as
/// often, worth 5000 a fifth, and stacks (ammo, medicine, grenades) are not
/// touched.
const CHEAP_DROP_VALUE: u32 = 1000;

/// One branch of the category tree: a weight and either categories or more
/// branches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryGroup {
    pub weight: u32,
    pub categories: Vec<u8>,
    pub groups: Vec<CategoryGroup>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropTable {
    /// The progress each drop needs.
    pub slots_open: Vec<u32>,
    /// The most progress a match gives (without premium).
    pub drop_amount: u32,
    pub categories: Vec<CategoryGroup>,
    /// By the wearer's item level: a weight for each item level.
    pub levels: Vec<Vec<u32>>,
}

fn number(value: &ConfigValue) -> Result<u32, ConfigError> {
    value
        .as_i32()
        .and_then(|number| u32::try_from(number).ok())
        .ok_or_else(|| ConfigError("a drop table number is not one".into()))
}

fn numbers(value: &ConfigValue) -> Result<Vec<u32>, ConfigError> {
    value.children().into_iter().map(number).collect()
}

fn groups(value: &ConfigValue) -> Result<Vec<CategoryGroup>, ConfigError> {
    value
        .children()
        .into_iter()
        .map(|group| {
            Ok(CategoryGroup {
                weight: number(group.req("drop_weight")?)?,
                categories: match group.get("categories") {
                    Some(categories) => numbers(categories)?
                        .into_iter()
                        .map(|category| category as u8)
                        .collect(),
                    None => Vec::new(),
                },
                groups: match group.get("item_category_probabilities") {
                    Some(inner) => groups(inner)?,
                    None => Vec::new(),
                },
            })
        })
        .collect()
}

impl DropTable {
    pub fn shared() -> &'static Self {
        static SHARED: std::sync::OnceLock<DropTable> = std::sync::OnceLock::new();
        SHARED.get_or_init(|| Self::parse(DROPS).expect("the embedded 0.21d drop table parses"))
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, ConfigError> {
        let root = ConfigValue::parse(bytes)?;
        Ok(Self {
            slots_open: numbers(root.req("slots_open")?)?,
            drop_amount: numbers(root.req("drop_amount")?)?
                .first()
                .copied()
                .ok_or_else(|| ConfigError("no drop_amount".into()))?,
            categories: groups(root.req("item_category_probabilities")?)?,
            levels: root
                .req("item_level_probabilities")?
                .children()
                .into_iter()
                .map(numbers)
                .collect::<Result<_, _>>()?,
        })
    }

    /// The bar's progress for a match's score.
    pub fn progress(&self, score: u16) -> u32 {
        (u32::from(score) * PROGRESS_PER_SCORE).min(self.drop_amount)
    }

    /// The progress with premium: half again, up to the bar's top,
    /// `drop_amount` × 1.5 [I: the screen's top is that, the premium bonus].
    pub fn premium_progress(&self, score: u16) -> u32 {
        super::premium::with_bonus(u32::from(score) * PROGRESS_PER_SCORE)
            .min(super::premium::with_bonus(self.drop_amount))
    }
}

/// A small xorshift generator: drops need no more.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    /// Seeded from the clock.
    pub fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        Self::new(nanos ^ 0x9E37_79B9_7F4A_7C15)
    }

    /// A number below `bound` (0 when `bound` is 0).
    pub fn below(&mut self, bound: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        if bound == 0 {
            0
        } else {
            (self.0 % u64::from(bound)) as u32
        }
    }

    /// An index into `weights`, by weight; `None` when they are all 0.
    pub fn pick(&mut self, weights: &[u32]) -> Option<usize> {
        let total: u32 = weights.iter().sum();
        if total == 0 {
            return None;
        }
        let mut roll = self.below(total);
        weights.iter().position(|&weight| {
            if roll < weight {
                true
            } else {
                roll -= weight;
                false
            }
        })
    }
}

fn pick_category(groups: &[CategoryGroup], rng: &mut Rng) -> Option<u8> {
    let weights: Vec<u32> = groups.iter().map(|group| group.weight).collect();
    let group = &groups[rng.pick(&weights)?];
    if group.groups.is_empty() {
        let index = rng.below(group.categories.len() as u32) as usize;
        group.categories.get(index).copied()
    } else {
        pick_category(&group.groups, rng)
    }
}

/// What one drop gives of `item`: a pack of ammo, one of any other stack,
/// anything else at full condition.
fn drop_count(item: &Item) -> u16 {
    if !item.is_stack {
        return super::account::FULL_CONDITION;
    }
    item_params(item.dict_id)
        .map(|params| params.clip_size * params.clips_in_bag.max(1))
        .filter(|&rounds| rounds > 0)
        .map_or(1, |rounds| rounds.min(u16::MAX.into()) as u16)
}

/// Whether a rolled item stays a drop: gear above [`CHEAP_DROP_VALUE`] is
/// kept with a probability that falls as its value rises.
fn kept_by_value(item: &Item, rng: &mut Rng) -> bool {
    if item.is_stack {
        return true;
    }
    let value = item_params(item.dict_id).map_or(0, |params| params.drop_value);
    value <= CHEAP_DROP_VALUE || rng.below(value) < CHEAP_DROP_VALUE
}

/// One drop: a category from the tree, an item level from the wearer's row
/// of the level table, then an item of that category, the nearest level to
/// it, worth at most `progress`, by its drop weight.
fn roll_one(
    table: &DropTable,
    dictionary: &Dictionary,
    progress: u32,
    wear_level: u8,
    rng: &mut Rng,
) -> Option<(u16, u16)> {
    // A category with nothing to drop is rolled again, a few times.
    for _ in 0..8 {
        let category = pick_category(&table.categories, rng)?;
        let candidates: Vec<(&Item, u32)> = dictionary
            .items
            .iter()
            .filter(|item| item.category == category && !item.is_premium)
            .filter_map(|item| {
                let params = item_params(item.dict_id)?;
                (params.drop_weight > 0 && params.drop_value <= progress)
                    .then_some((item, params.drop_weight))
            })
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let row = &table.levels[usize::from(wear_level).min(table.levels.len() - 1)];
        let level = rng.pick(row).unwrap_or(0) as i32;
        let nearest = candidates
            .iter()
            .map(|(item, _)| (i32::from(item.level) - level).abs())
            .min()?;
        let at_level: Vec<&(&Item, u32)> = candidates
            .iter()
            .filter(|(item, _)| (i32::from(item.level) - level).abs() == nearest)
            .collect();
        let weights: Vec<u32> = at_level.iter().map(|(_, weight)| *weight).collect();
        let (item, _) = at_level[rng.pick(&weights)?];
        if !kept_by_value(item, rng) {
            continue;
        }
        return Some((item.dict_id, drop_count(item)));
    }
    None
}

/// The drops for a match's progress: one for each notch it reaches.
/// `wear_level` is the highest item level the player wore.
pub fn roll(
    table: &DropTable,
    dictionary: &Dictionary,
    progress: u32,
    wear_level: u8,
    rng: &mut Rng,
) -> Vec<(u16, u16)> {
    table
        .slots_open
        .iter()
        .filter(|&&notch| progress >= notch)
        .filter_map(|_| roll_one(table, dictionary, progress, wear_level, rng))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_021d_drop_table() {
        let table = DropTable::shared();
        assert_eq!(table.slots_open, [1000, 5000, 9000]);
        assert_eq!(table.drop_amount, 10_000);
        assert_eq!(table.levels.len(), 21);
        assert_eq!(table.levels[0][..3], [70, 25, 5]);
        assert_eq!(table.categories.len(), 2);
        assert_eq!(table.categories[0].weight, 90);
        assert_eq!(
            table.categories[0].groups[0].categories,
            [9, 18, 19, 20, 21]
        );
        assert_eq!(
            table.categories[1].groups[1].categories,
            [8, 13, 14, 15, 16, 17]
        );
    }

    #[test]
    fn progress_is_the_score_times_ten_up_to_the_top() {
        let table = DropTable::shared();
        assert_eq!(table.progress(375), 3750);
        assert_eq!(table.progress(5000), 10_000);
        assert_eq!(table.premium_progress(375), 5625);
        assert_eq!(table.premium_progress(5000), 15_000);
    }

    #[test]
    fn each_notch_reached_drops_an_item_worth_the_progress() {
        let table = DropTable::shared();
        let dictionary = Dictionary::shared();
        let mut rng = Rng::new(7);
        assert!(roll(table, dictionary, 999, 3, &mut rng).is_empty());
        for _ in 0..200 {
            let drops = roll(table, dictionary, 3750, 3, &mut rng);
            assert_eq!(drops.len(), 1);
            let (dict_id, count) = drops[0];
            let item = dictionary.item(dict_id).unwrap();
            let params = item_params(dict_id).unwrap();
            assert!(params.drop_value <= 3750 && params.drop_weight > 0);
            assert!(!item.is_premium);
            if item.is_stack {
                assert!(count >= 1);
            } else {
                assert_eq!(count, 100);
            }
        }
        assert_eq!(roll(table, dictionary, 10_000, 7, &mut rng).len(), 3);
    }

    /// The share of a thousand matches' drops that are gear and its mean
    /// value, for a player of `wear` level with a full bar.
    fn gear_share(wear: u8, seed: u64) -> (f64, f64) {
        let table = DropTable::shared();
        let dictionary = Dictionary::shared();
        let mut rng = Rng::new(seed);
        let (mut gear, mut drops, mut value) = (0u32, 0u32, 0u64);
        for _ in 0..4000 {
            for (dict_id, _) in roll(table, dictionary, 10_000, wear, &mut rng) {
                drops += 1;
                if !dictionary.item(dict_id).unwrap().is_stack {
                    gear += 1;
                    value += u64::from(item_params(dict_id).unwrap().drop_value);
                }
            }
        }
        (
            f64::from(gear) / f64::from(drops),
            value as f64 / f64::from(gear.max(1)),
        )
    }

    #[test]
    fn expensive_gear_drops_much_less_often_and_stacks_are_untouched() {
        let dictionary = Dictionary::shared();
        let id = |name: &str| dictionary.item_named(name).unwrap();
        // 5000-value gear is kept a fifth of the time, 1000-value always, and
        // a stack always.
        let mut rng = Rng::new(3);
        let (mut rifle, mut ppsh, mut medkit) = (0, 0, 0);
        for _ in 0..10_000 {
            rifle += u32::from(kept_by_value(id("rem_700"), &mut rng));
            ppsh += u32::from(kept_by_value(id("ppsh"), &mut rng));
            medkit += u32::from(kept_by_value(id("medkit"), &mut rng));
        }
        assert!((1700..2300).contains(&rifle), "rifle kept {rifle} of 10000");
        assert_eq!(ppsh, 10_000);
        assert_eq!(medkit, 10_000);
        // Over whole matches. The game alone makes gear 10.5% of drops at
        // every wear level, worth 2100 on average for level 0 and 4800 for
        // level 7; with the bias a high-level player gets gear in 2.4% of
        // drops, and a new player's cheap gear is hardly touched.
        let (share, _) = gear_share(7, 11);
        assert!(share < 0.035, "gear is {share} of drops at level 7");
        let (share, mean) = gear_share(0, 11);
        assert!(
            (0.06..0.10).contains(&share),
            "gear is {share} of drops at level 0"
        );
        assert!(mean < 1600.0, "the mean gear value at level 0 is {mean}");
    }

    #[test]
    fn ammo_drops_a_pack() {
        let dictionary = Dictionary::shared();
        // 5.45x39: 50 rounds a clip, one clip a pack.
        let ammo = dictionary.item_named("ammo_5.45x39_7n6m").unwrap();
        assert_eq!(drop_count(ammo), 50);
        let medkit = dictionary.item_named("medkit").unwrap();
        assert_eq!(drop_count(medkit), 1);
    }
}
