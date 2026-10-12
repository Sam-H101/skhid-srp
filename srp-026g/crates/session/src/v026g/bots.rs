//! Test bots (`SRP_BOT_ACCOUNTS`). **For testing only; off unless set.**
//!
//! A 0.26g match needs two players, and one person with one PC has one
//! game. `srp-bot` (`servers/srp/src/bot.rs`) is a scripted client that
//! signs in, queues and sits in the match so that the one real client gets
//! one. It cannot simulate the game: it has no world to hash or upload.
//!
//! `SRP_BOT_ACCOUNTS=bot-1,bot-2` names the accounts that are such bots
//! (by the name the account shows, compared without case). The server then
//! knows, instead of the bot pretending:
//! - the match server takes a bot's place as a player and never as a
//!   witness of the world (`match_server::v026g`, `game.rs`, "Bots");
//! - a match with a bot in it moves no rating, a bot is paid no daily
//!   bonus and starts no run of bonus days, and bots are left out of the
//!   ranking ([`super::GameAccounts::ranking`]). Everything else of its
//!   results is settled as for any account.
//!
//! Do not set it on a public server: in a match of one real client and
//! bots, that client's world is trusted on its word alone, and anyone who
//! holds a named account is never checked. The name is taken by whoever
//! registers it first, as any name is: start the bot before anyone else
//! could, or name accounts that exist.

/// The setting's name.
pub const ENV: &str = "SRP_BOT_ACCOUNTS";

/// The names of a `SRP_BOT_ACCOUNTS` value: comma-separated, trimmed, in
/// lower case, the empty ones dropped.
pub fn parse(value: &str) -> Vec<String> {
    let mut names: Vec<String> = value
        .split(',')
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// What the environment names; none when the setting is not there.
pub fn from_env() -> Vec<String> {
    std::env::var(ENV).map_or(Vec::new(), |value| parse(&value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v026g::GameAccounts;

    #[test]
    fn the_setting_is_a_list_of_names() {
        assert_eq!(parse(""), Vec::<String>::new());
        assert_eq!(parse(" , "), Vec::<String>::new());
        assert_eq!(parse("Bot-1, bot-2,,bot-1"), ["bot-1", "bot-2"]);
    }

    #[test]
    fn nobody_is_a_bot_unless_named_and_bots_are_out_of_the_ranking() {
        let store = GameAccounts::new();
        store.get_or_create(1, "Alice");
        store.get_or_create(2, "Bot-1");
        assert!(!store.is_bot(2) && !store.is_bot_name("bot-1"));
        assert_eq!(store.ranking().len(), 2);

        store.set_bots(&parse("bot-1,bot-2"));
        assert!(store.is_bot(2) && store.is_bot_name("BOT-1"));
        assert!(!store.is_bot(1) && !store.is_bot(77));
        let names: Vec<String> = store.ranking().into_iter().map(|row| row.1).collect();
        assert_eq!(names, ["Alice"]);

        store.set_bots(&[]);
        assert!(!store.is_bot(2));
        assert_eq!(store.ranking().len(), 2);
    }
}
