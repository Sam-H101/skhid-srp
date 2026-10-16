//! What an item keeps of its modification changes
//! ([`Reroll`], `super::super::items_rules`), in a column of `g26_items`,
//! and the camouflage a bound copy is of ([`Item::copy_of`]), in another.
//!
//! The item's modifications, defects and upgrades were columns from the
//! first step on (`modifications`, `upgrades`), and its two timers too
//! (`repair_time`, `reroll_time`: now the Unix second each ends at; they
//! were always 0 before).

use super::super::account::{Item, Reroll};
use super::{PersistError, bad_row};

/// The step that adds the column: an item of before has changed nothing.
pub const MIGRATION: &str = "
    -- `modification:count`, then `:replaced:first:second:third` while a
    -- paid change waits for its choice; empty for an item never changed.
    ALTER TABLE g26_items ADD COLUMN reroll TEXT NOT NULL DEFAULT '';
";

/// The step that adds the bound copies of camouflages ([`Item::copy_of`]):
/// every item of before is one of its own, and a camouflage it wore under
/// the rule of before (the camouflage itself on its host) stays on it.
pub const COPIES_MIGRATION: &str = "
    -- 0: an item of its own; else the item id of the camouflage this is a
    -- bound copy of.
    ALTER TABLE g26_items ADD COLUMN copy_of INTEGER NOT NULL DEFAULT 0;
";

/// The column's text for an item.
pub fn reroll_text(item: &Item) -> String {
    let reroll = &item.reroll;
    if *reroll == Reroll::default() {
        return String::new();
    }
    let mut text = format!("{}:{}", reroll.modification, reroll.count);
    if let Some((replaced, choices)) = reroll.offer {
        text.push_str(&format!(
            ":{replaced}:{}:{}:{}",
            choices[0], choices[1], choices[2]
        ));
    }
    text
}

/// The column's text read back.
pub fn parse_reroll(account_id: u32, text: &str) -> Result<Reroll, PersistError> {
    if text.is_empty() {
        return Ok(Reroll::default());
    }
    let bad = || bad_row(account_id, "reroll", text);
    let fields: Vec<u32> = text
        .split(':')
        .map(|field| field.parse().map_err(|_| bad()))
        .collect::<Result<_, _>>()?;
    match fields[..] {
        [modification, count] => Ok(Reroll {
            modification,
            count,
            offer: None,
        }),
        [modification, count, replaced, first, second, third] => Ok(Reroll {
            modification,
            count,
            offer: Some((replaced, [first, second, third])),
        }),
        _ => Err(bad()),
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::super::{GameRepository, MIGRATIONS, SqliteGameRepository};
    use super::*;
    use crate::persist::DB_FILE;
    use crate::v026g::account::Account;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "srp-g26-items-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_column_reads_back() {
        let mut account = Account::new("Alice");
        assert_eq!(reroll_text(&account.inventory[0]), "");
        assert_eq!(parse_reroll(1, "").unwrap(), Reroll::default());
        account.inventory[0].reroll = Reroll {
            modification: 2000,
            count: 3,
            offer: None,
        };
        assert_eq!(reroll_text(&account.inventory[0]), "2000:3");
        assert_eq!(
            parse_reroll(1, "2000:3").unwrap(),
            account.inventory[0].reroll
        );
        account.inventory[0].reroll.offer = Some((2000, [2010, 2020, 2031]));
        let text = reroll_text(&account.inventory[0]);
        assert_eq!(text, "2000:3:2000:2010:2020:2031");
        assert_eq!(parse_reroll(1, &text).unwrap(), account.inventory[0].reroll);
        for damaged in ["x", "1", "1:2:3", "1:2:3:4:5:6:7", "1:-2"] {
            assert!(parse_reroll(1, damaged).is_err(), "{damaged}");
        }
    }

    /// A file of the version before this step, with an account and its
    /// items in it, is brought up and keeps them; what the items gain since
    /// comes back after a save.
    #[test]
    fn the_file_before_is_brought_up_and_keeps_its_items() {
        let dir = temp_dir("migration");
        let path = dir.join(DB_FILE);
        let step = MIGRATIONS
            .iter()
            .position(|step| *step == MIGRATION)
            .expect("the items' step is one of the migrations");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE g26_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);",
                )
                .unwrap();
            for earlier in &MIGRATIONS[..step] {
                connection.execute_batch(earlier).unwrap();
            }
            connection
                .execute(
                    "INSERT INTO g26_meta (key, value) VALUES ('schema', ?1)",
                    [step as i64],
                )
                .unwrap();
            // The columns of the first step.
            connection
                .execute_batch(
                    "INSERT INTO g26_accounts (
                        account_id, name, clan_tag, generic_money, premium_money, premium_until,
                        exp_total, quest_faction, rating, rating_before, active_kit, autobuy,
                        next_item_id, next_quest_instance, tutorial_step, tutorial_done
                     ) VALUES (5, 'Alice', '', 777, 0, 0, 0, 0, 1000, 1000, 1, 0, 3, 1, 0, 0);
                     INSERT INTO g26_items (
                        account_id, item_id, seq, dict_id, condition_or_stack, repair_time,
                        reroll_time, modifications, upgrades
                     ) VALUES (5, 1, 0, 18, 900000, 0, 0, '2000:0,3000:1', ''),
                              (5, 2, 1, 50, 50, 0, 0, '', '');",
                )
                .unwrap();
        }
        let mut repository = SqliteGameRepository::open(&path).unwrap();
        assert_eq!(repository.version().unwrap(), MIGRATIONS.len() as i64);
        let mut loaded = repository.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        let (id, account) = &mut loaded[0];
        assert_eq!((*id, account.generic_money), (5, 777));
        assert_eq!(account.inventory.len(), 2);
        let pistol = &account.inventory[0];
        assert_eq!(pistol.condition_or_stack, 900_000);
        assert_eq!(pistol.modifications, [(2000, false), (3000, true)]);
        assert_eq!(pistol.reroll, Reroll::default());

        // And the new column is kept.
        account.inventory[0].reroll = Reroll {
            modification: 2000,
            count: 2,
            offer: Some((2000, [2010, 2020, 2030])),
        };
        account.inventory[0].reroll_time = 1_800_000_000;
        repository.save(5, account).unwrap();
        drop(repository);
        let mut repository = SqliteGameRepository::open(&path).unwrap();
        let again = repository.load_all().unwrap();
        assert_eq!(again[0].1.inventory[0], account.inventory[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file of before the bound copies, with a camouflage worn under the
    /// rule of before (the camouflage itself on its vest), is brought up
    /// and loads it as it was; the copies made since come back after a
    /// save.
    #[test]
    fn a_camouflage_worn_before_copies_loads_as_it_was() {
        use crate::v026g::account::Upgrade;
        use crate::v026g::items_rules;
        use survarium::v026g::dictionary::Dictionary;

        let dictionary = Dictionary::shared();
        let vest = dictionary.item_named("edge_torso_2").unwrap().dict_id;
        let spare = dictionary.item_named("bm_torso_1").unwrap().dict_id;
        let camo = dictionary.item_named("camo_city").unwrap().dict_id;
        let dir = temp_dir("copies");
        let path = dir.join(DB_FILE);
        let step = MIGRATIONS
            .iter()
            .position(|step| *step == COPIES_MIGRATION)
            .expect("the copies' step is one of the migrations");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE g26_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);",
                )
                .unwrap();
            for earlier in &MIGRATIONS[..step] {
                connection.execute_batch(earlier).unwrap();
            }
            connection
                .execute(
                    "INSERT INTO g26_meta (key, value) VALUES ('schema', ?1)",
                    [step as i64],
                )
                .unwrap();
            connection
                .execute_batch(&format!(
                    "INSERT INTO g26_accounts (
                        account_id, name, clan_tag, generic_money, premium_money, premium_until,
                        exp_total, quest_faction, rating, rating_before, active_kit, autobuy,
                        next_item_id, next_quest_instance, tutorial_step, tutorial_done
                     ) VALUES (5, 'Alice', '', 777, 0, 0, 0, 0, 1000, 1000, 1, 0, 4, 1, 0, 0);
                     INSERT INTO g26_items (
                        account_id, item_id, seq, dict_id, condition_or_stack, repair_time,
                        reroll_time, modifications, upgrades, reroll
                     ) VALUES (5, 1, 0, {vest}, 1000000, 0, 0, '', '3:{camo}:42:3', ''),
                              (5, 2, 1, {spare}, 1000000, 0, 0, '', '', ''),
                              (5, 3, 2, {camo}, 1000000, 0, 0, '', '', '');"
                ))
                .unwrap();
        }
        let mut repository = SqliteGameRepository::open(&path).unwrap();
        assert_eq!(repository.version().unwrap(), MIGRATIONS.len() as i64);
        let mut loaded = repository.load_all().unwrap();
        let (_, account) = &mut loaded[0];
        assert_eq!(account.inventory.len(), 3);
        assert!(account.inventory.iter().all(|item| item.copy_of == 0));
        let worn = Upgrade {
            id: 3,
            a: camo.into(),
            value: 42,
            param: 3,
        };
        assert_eq!(account.inventory[0].upgrades, [worn]);
        // What a load makes of it keeps it worn.
        let mut repaired = account.clone();
        repaired.repair();
        assert_eq!(repaired.inventory, account.inventory);

        // On the spare vest too: two copies, kept across a save.
        let attached = items_rules::attach_upgrade(account, 2, 3, 3).unwrap();
        assert!(attached.new_copy);
        assert_eq!(
            account
                .inventory
                .iter()
                .filter(|item| item.copy_of == 3)
                .count(),
            2
        );
        repository.save(5, account).unwrap();
        drop(repository);
        let mut repository = SqliteGameRepository::open(&path).unwrap();
        let again = repository.load_all().unwrap();
        assert_eq!(again[0].1.inventory, account.inventory);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
