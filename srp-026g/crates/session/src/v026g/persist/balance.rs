//! The row an account has in `g26_balance`: its balance rating and the
//! matches that moved it ([`crate::v026g::balance`]).

use rusqlite::{Connection, OptionalExtension, params};

use crate::persist::PersistError;
use crate::v026g::balance::Balance;

/// The schema step that adds the table. An account of before has no row:
/// its rating is the average and no match has moved it.
pub(super) const MIGRATION: &str = "
    CREATE TABLE g26_balance (
        account_id INTEGER PRIMARY KEY REFERENCES g26_accounts(account_id) ON DELETE CASCADE,
        -- Thousandths of a point; 0 is the average.
        rating     INTEGER NOT NULL,
        matches    INTEGER NOT NULL
    );
    ";

/// The account's rating; the average of no match without a row.
pub(super) fn load(connection: &Connection, account_id: u32) -> Result<Balance, PersistError> {
    let mut statement = connection
        .prepare_cached("SELECT rating, matches FROM g26_balance WHERE account_id = ?1")?;
    let row = statement
        .query_row([account_id], |row| {
            Ok(Balance {
                rating: row.get(0)?,
                matches: row.get(1)?,
            })
        })
        .optional()?;
    Ok(row.unwrap_or_default())
}

/// Replaces the account's row.
pub(super) fn save(
    connection: &Connection,
    account_id: u32,
    balance: Balance,
) -> Result<(), PersistError> {
    connection.execute(
        "INSERT INTO g26_balance (account_id, rating, matches) VALUES (?1, ?2, ?3)
         ON CONFLICT(account_id) DO UPDATE SET
            rating = excluded.rating, matches = excluded.matches",
        params![account_id, balance.rating, balance.matches],
    )?;
    Ok(())
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
            "srp-g26-balance-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_rating_comes_back_and_a_new_account_has_the_average() {
        let dir = temp_dir("round-trip");
        let mut rated = Account::new("Alice");
        rated.balance = Balance {
            rating: -1375,
            matches: 12,
        };
        {
            let mut repository = SqliteGameRepository::open(&dir.join(DB_FILE)).unwrap();
            repository.save(1, &rated).unwrap();
            repository.save(2, &Account::new("Bob")).unwrap();
            // A second save replaces the row.
            rated.balance = rated.balance.moved(400);
            repository.save(1, &rated).unwrap();
        }
        let mut repository = SqliteGameRepository::open(&dir.join(DB_FILE)).unwrap();
        let loaded = repository.load_all().unwrap();
        assert_eq!(
            loaded[0].1.balance,
            Balance {
                rating: -975,
                matches: 13
            }
        );
        assert_eq!(loaded[1].1.balance, Balance::default());
        drop(repository);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_accounts_of_before_the_step_have_the_average() {
        let dir = temp_dir("upgrade");
        let path = dir.join(DB_FILE);
        {
            // The tables as the step before left them, with an account.
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE g26_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);",
                )
                .unwrap();
            let before = MIGRATIONS
                .iter()
                .position(|step| *step == MIGRATION)
                .unwrap();
            for step in &MIGRATIONS[..before] {
                connection.execute_batch(step).unwrap();
            }
            connection
                .execute(
                    "INSERT INTO g26_meta (key, value) VALUES ('schema', ?1)",
                    [before as i64],
                )
                .unwrap();
            connection
                .execute_batch(
                    "INSERT INTO g26_accounts (
                        account_id, name, clan_tag, generic_money, premium_money,
                        premium_until, exp_total, quest_faction, rating, rating_before,
                        active_kit, autobuy, next_item_id, next_quest_instance,
                        tutorial_step, tutorial_done
                     ) VALUES (5, 'Carol', '', 4321, 5, 0, 900, 2, 118, 100, 1, 0, 3, 1, 0, 1);",
                )
                .unwrap();
        }
        let mut repository = SqliteGameRepository::open(&path).unwrap();
        assert_eq!(repository.version().unwrap(), MIGRATIONS.len() as i64);
        let loaded = repository.load_all().unwrap();
        let (id, mut account) = loaded.into_iter().next().unwrap();
        assert_eq!(
            (id, account.rating, account.balance),
            (5, 118, Balance::default())
        );
        // And it is kept from then on.
        account.balance = account.balance.moved(-250);
        repository.save(id, &account).unwrap();
        let loaded = repository.load_all().unwrap();
        assert_eq!(
            loaded[0].1.balance,
            Balance {
                rating: -250,
                matches: 1
            }
        );
        drop(repository);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
