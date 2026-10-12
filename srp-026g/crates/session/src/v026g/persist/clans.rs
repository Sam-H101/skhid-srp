//! Clans in srp's SQLite file (`super::super::clans`): three `g26_clan*`
//! tables beside the accounts'. A clan is written whole, in one
//! transaction; invites and clan teams are not kept.

use rusqlite::{Connection, params};

use crate::persist::PersistError;
use crate::v026g::clans::{Clan, Member, Rank};

/// The schema step that adds the clans.
pub(super) const MIGRATION: &str = "
    CREATE TABLE g26_clans (
        clan_id       INTEGER PRIMARY KEY,
        name          TEXT NOT NULL,
        tag           TEXT NOT NULL,
        registered    INTEGER NOT NULL,
        -- The founder of a registration, the commander of a clan.
        leader        INTEGER NOT NULL,
        daily_message TEXT NOT NULL,
        experience    INTEGER NOT NULL,
        -- The silver paid into the registration.
        money         INTEGER NOT NULL
    );
    CREATE TABLE g26_clan_ranks (
        clan_id     INTEGER NOT NULL REFERENCES g26_clans(clan_id) ON DELETE CASCADE,
        seq         INTEGER NOT NULL,
        name        TEXT NOT NULL,
        permissions INTEGER NOT NULL,
        is_default  INTEGER NOT NULL,
        PRIMARY KEY (clan_id, seq)
    );
    -- Also the players invited to a registration (status 1 and 2).
    CREATE TABLE g26_clan_members (
        clan_id    INTEGER NOT NULL REFERENCES g26_clans(clan_id) ON DELETE CASCADE,
        account_id INTEGER NOT NULL,
        seq        INTEGER NOT NULL,
        rank       INTEGER NOT NULL,
        status     INTEGER NOT NULL,
        PRIMARY KEY (clan_id, account_id)
    );
    ";

pub(super) fn load(connection: &mut Connection) -> Result<Vec<Clan>, PersistError> {
    let transaction = connection.transaction()?;
    let mut clans: Vec<Clan> = Vec::new();
    {
        let mut statement = transaction.prepare(
            "SELECT clan_id, name, tag, registered, leader, daily_message, experience, money
             FROM g26_clans ORDER BY clan_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Clan {
                id: row.get(0)?,
                name: row.get(1)?,
                tag: row.get(2)?,
                registered: row.get(3)?,
                leader: row.get(4)?,
                ranks: Vec::new(),
                members: Vec::new(),
                daily_message: row.get(5)?,
                experience: row.get(6)?,
                money: row.get(7)?,
            })
        })?;
        for row in rows {
            clans.push(row?);
        }
    }
    for clan in &mut clans {
        {
            let mut statement = transaction.prepare_cached(
                "SELECT name, permissions, is_default FROM g26_clan_ranks
                 WHERE clan_id = ?1 ORDER BY seq",
            )?;
            let rows = statement.query_map([clan.id], |row| {
                Ok(Rank {
                    name: row.get(0)?,
                    permissions: row.get(1)?,
                    is_default: row.get(2)?,
                })
            })?;
            clan.ranks = rows.collect::<Result<_, _>>()?;
        }
        let mut statement = transaction.prepare_cached(
            "SELECT account_id, rank, status FROM g26_clan_members
             WHERE clan_id = ?1 ORDER BY seq",
        )?;
        let rows = statement.query_map([clan.id], |row| {
            Ok(Member {
                account_id: row.get(0)?,
                rank: row.get(1)?,
                status: row.get(2)?,
            })
        })?;
        clan.members = rows.collect::<Result<_, _>>()?;
        // A member's rank is one the clan has.
        let lowest = clan.ranks.len().saturating_sub(1) as u8;
        if clan.ranks.is_empty() || clan.members.iter().any(|member| member.rank > lowest) {
            return Err(PersistError::BadRow(
                clan.id,
                "a clan member's rank is none of the clan's".to_owned(),
            ));
        }
    }
    transaction.commit()?;
    Ok(clans)
}

pub(super) fn save(connection: &mut Connection, clan: &Clan) -> Result<(), PersistError> {
    let transaction = connection.transaction()?;
    for table in ["g26_clan_ranks", "g26_clan_members"] {
        transaction.execute(
            &format!("DELETE FROM {table} WHERE clan_id = ?1"),
            [clan.id],
        )?;
    }
    transaction.execute(
        "INSERT INTO g26_clans (
            clan_id, name, tag, registered, leader, daily_message, experience, money
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(clan_id) DO UPDATE SET
            name = excluded.name, tag = excluded.tag, registered = excluded.registered,
            leader = excluded.leader, daily_message = excluded.daily_message,
            experience = excluded.experience, money = excluded.money",
        params![
            clan.id,
            clan.name,
            clan.tag,
            clan.registered,
            clan.leader,
            clan.daily_message,
            clan.experience,
            clan.money,
        ],
    )?;
    for (seq, rank) in clan.ranks.iter().enumerate() {
        transaction.execute(
            "INSERT INTO g26_clan_ranks (clan_id, seq, name, permissions, is_default)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                clan.id,
                seq as i64,
                rank.name,
                rank.permissions,
                rank.is_default
            ],
        )?;
    }
    for (seq, member) in clan.members.iter().enumerate() {
        transaction.execute(
            "INSERT INTO g26_clan_members (clan_id, account_id, seq, rank, status)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                clan.id,
                member.account_id,
                seq as i64,
                member.rank,
                member.status
            ],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

pub(super) fn delete(connection: &mut Connection, clan_id: u32) -> Result<(), PersistError> {
    // The ranks and members go with it (ON DELETE CASCADE).
    connection.execute("DELETE FROM g26_clans WHERE clan_id = ?1", [clan_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{GameRepository, MIGRATIONS, SqliteGameRepository};
    use super::*;
    use crate::persist::DB_FILE;
    use crate::v026g::clans::{Clans, Config};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("srp-g26-clans-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A registered clan of three with a rank of its own, and a
    /// registration with a pending player.
    fn two_clans() -> Vec<Clan> {
        let clans = Clans::new();
        clans.set_config(Config {
            cost: 1_000,
            max_members: 100,
            ..Config::default()
        });
        clans.reserve(1, "Stalkers", "STK").unwrap();
        for target in [2, 3] {
            clans.invite(1, target, target, "Stalkers").unwrap();
            clans.answer_invite(target, 1, true).unwrap();
        }
        clans.pay(1, 1_000).unwrap();
        clans.create(1).unwrap();
        clans.create_rank(1, "Разведка").unwrap();
        clans.set_permission(1, 3, 1, true).unwrap();
        clans
            .set_daily_message(1, "Сбор в девять\nне опаздывать")
            .unwrap();
        clans.add_experience(2, 12_345).unwrap();
        // Two characters are no abbreviation.
        clans.reserve(7, "Чистое Небо", "ЧН").unwrap_err();
        clans.reserve(7, "Чистое Небо", "ЧН-1").unwrap();
        clans.invite(7, 8, 80, "Чистое Небо").unwrap();
        clans.pay(7, 300).unwrap();
        clans.take_changes().0
    }

    #[test]
    fn clans_come_back_as_they_were_saved() {
        let dir = temp_dir("round-trip");
        let clans = two_clans();
        assert_eq!(clans.len(), 2);
        {
            let mut repository = SqliteGameRepository::open(&dir.join(DB_FILE)).unwrap();
            assert!(repository.load_clans().unwrap().is_empty());
            for clan in &clans {
                repository.save_clan(clan).unwrap();
            }
        }
        let mut repository = SqliteGameRepository::open(&dir.join(DB_FILE)).unwrap();
        assert_eq!(repository.load_clans().unwrap(), clans);

        // A second save replaces the first whole; a deleted clan is gone
        // with its rows.
        let mut changed = clans[0].clone();
        changed.members.pop();
        changed.ranks.remove(3);
        changed.members[1].rank = 3;
        repository.save_clan(&changed).unwrap();
        repository.delete_clan(clans[1].id).unwrap();
        assert_eq!(repository.load_clans().unwrap(), [changed]);
        let left: i64 = repository
            .connection
            .query_row(
                "SELECT COUNT(*) FROM g26_clan_members WHERE clan_id = ?1",
                [clans[1].id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
        drop(repository);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_member_of_no_rank_is_an_error_not_a_guess() {
        let dir = temp_dir("damaged");
        let mut repository = SqliteGameRepository::open(&dir.join(DB_FILE)).unwrap();
        repository.save_clan(&two_clans()[0]).unwrap();
        repository
            .connection
            .execute(
                "UPDATE g26_clan_members SET rank = 9 WHERE account_id = 2",
                [],
            )
            .unwrap();
        assert!(matches!(
            repository.load_clans(),
            Err(PersistError::BadRow(1, _))
        ));
        drop(repository);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_store_writes_its_clans_and_loads_them_again() {
        use crate::v026g::{GameAccounts, MemoryGameRepository};

        let repository = MemoryGameRepository::default();
        let store = GameAccounts::with_repository(Box::new(repository.clone())).unwrap();
        store.clans().set_config(Config {
            cost: 1_000,
            max_members: 100,
            ..Config::default()
        });
        store.clans().reserve(1, "Stalkers", "STK").unwrap();
        store.clans().invite(1, 2, 5, "Stalkers").unwrap();
        store.flush();
        let again = GameAccounts::with_repository(Box::new(repository.clone())).unwrap();
        let clan = again.clans().clan_of(1).expect("the clan was written");
        assert_eq!((clan.name.as_str(), clan.members.len()), ("Stalkers", 2));
        // Nothing changed: nothing is written again.
        assert_eq!(store.clans().take_changes(), (Vec::new(), Vec::new()));
        // A disbanded clan is forgotten.
        store.clans().leave_or_disband(1).unwrap();
        store.flush();
        let again = GameAccounts::with_repository(Box::new(repository)).unwrap();
        assert!(again.clans().is_empty());
    }

    /// A file of the version before the clans', with an account in it, is
    /// brought up and keeps the account.
    #[test]
    fn the_file_before_the_clans_is_brought_up_and_keeps_its_accounts() {
        let dir = temp_dir("migration");
        let path = dir.join(DB_FILE);
        let step = MIGRATIONS
            .iter()
            .position(|step| *step == MIGRATION)
            .expect("the clans' step is one of the migrations");
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
            // The columns of the first step; later steps before the clans'
            // gave theirs defaults.
            connection
                .execute(
                    "INSERT INTO g26_accounts (
                        account_id, name, clan_tag, generic_money, premium_money, premium_until,
                        exp_total, quest_faction, rating, rating_before, active_kit, autobuy,
                        next_item_id, next_quest_instance, tutorial_step, tutorial_done
                     ) VALUES (5, 'Alice', '', 777, 0, 0, 0, 0, 1000, 1000, 1, 0, 1, 1, 0, 0)",
                    [],
                )
                .unwrap();
        }
        let mut repository = SqliteGameRepository::open(&path).unwrap();
        assert_eq!(repository.version().unwrap(), MIGRATIONS.len() as i64);
        assert!(repository.load_clans().unwrap().is_empty());
        let loaded = repository.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, 5);
        assert_eq!(loaded[0].1.name, "Alice");
        assert_eq!(loaded[0].1.generic_money, 777);
        // And clans are kept in it from then on.
        repository.save_clan(&two_clans()[1]).unwrap();
        assert_eq!(repository.load_clans().unwrap().len(), 1);
        drop(repository);
        let _ = std::fs::remove_dir_all(dir);
    }
}
