//! The rows of `g26_results`: what [`crate::v026g::rewards::Results`] keeps
//! across a restart, as JSON under a key each (the finished matches'
//! reports, and one row for who has not seen which and for the play
//! periods). The rows are `Results`' own; this file only stores them.

use rusqlite::{Connection, params};

use crate::persist::PersistError;

/// The schema step that adds the table. A database of before has no rows:
/// no screen waits and every play period starts anew, as after every
/// restart until then.
pub(super) const MIGRATION: &str = "
    CREATE TABLE g26_results (
        key  TEXT PRIMARY KEY,
        body TEXT NOT NULL
    );
    ";

/// Every row, by key.
pub(super) fn load(connection: &Connection) -> Result<Vec<(String, String)>, PersistError> {
    let mut statement =
        connection.prepare_cached("SELECT key, body FROM g26_results ORDER BY key")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Writes one row, replacing what was kept under its key.
pub(super) fn save(connection: &Connection, key: &str, body: &str) -> Result<(), PersistError> {
    connection.execute(
        "INSERT INTO g26_results (key, body) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET body = excluded.body",
        params![key, body],
    )?;
    Ok(())
}

/// Forgets a row; one that is not there is no error.
pub(super) fn delete(connection: &Connection, key: &str) -> Result<(), PersistError> {
    connection.execute("DELETE FROM g26_results WHERE key = ?1", [key])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{GameRepository, MIGRATIONS, SqliteGameRepository};
    use super::*;
    use crate::persist::DB_FILE;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "srp-g26-results-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_rows_outlive_the_repository() {
        let dir = temp_dir("round-trip");
        let path = dir.join(DB_FILE);
        let mut repository = SqliteGameRepository::open(&path).unwrap();
        assert!(repository.load_results().unwrap().is_empty());
        repository.save_result("state", "{}").unwrap();
        repository.save_result("report:7", "seven").unwrap();
        repository.save_result("report:8", "eight").unwrap();
        // A key is one row: written again, it is replaced.
        repository
            .save_result("state", "{\"reports\":[7]}")
            .unwrap();
        repository.delete_result("report:8").unwrap();
        repository.delete_result("report:9").unwrap();
        drop(repository);

        let mut repository = SqliteGameRepository::open(&path).unwrap();
        assert_eq!(
            repository.load_results().unwrap(),
            [
                ("report:7".to_owned(), "seven".to_owned()),
                ("state".to_owned(), "{\"reports\":[7]}".to_owned()),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A database made before this step gets the table, empty, and keeps
    /// everything it had.
    #[test]
    fn a_database_of_before_gets_the_table() {
        let dir = temp_dir("upgrade");
        let path = dir.join(DB_FILE);
        let own = MIGRATIONS
            .iter()
            .position(|step| *step == MIGRATION)
            .expect("the step is in the list");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE g26_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);",
                )
                .unwrap();
            for step in &MIGRATIONS[..own] {
                connection.execute_batch(step).unwrap();
            }
            connection
                .execute(
                    "INSERT INTO g26_meta (key, value) VALUES ('schema', ?1)",
                    [own as i64],
                )
                .unwrap();
        }
        let mut repository = SqliteGameRepository::open(&path).unwrap();
        assert_eq!(repository.version().unwrap(), MIGRATIONS.len() as i64);
        assert!(repository.load_results().unwrap().is_empty());
        repository.save_result("state", "x").unwrap();
        assert_eq!(repository.load_results().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
