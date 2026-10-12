//! What a 0.21d match is started with: the static info (0x51), its maps and
//! game modes (`../srp-021d/docs/notes/match_start_spec.md`).

use super::account::Profile;
use super::lobby::{Writer, write_profile_static};

/// `player_profile[20]` in `match_options`.
pub const MAX_PLAYERS: usize = 20;
/// The level project name buffer holds 31 characters and a terminator.
const MAX_PROJECT_LEN: usize = 31;
/// The HUD has battery icons for at most ten.
pub const MAX_BATTERIES: u8 = 10;

/// `game_mode`. Modes 0 and 1 need base points, which no 0.21d map has, so
/// they play as `GATHER_VICTORY_ITEMS` and are not offered.
pub mod game_mode {
    pub const CAPTURE_ENEMY_BASE: u8 = 0;
    pub const CAPTURE_NEUTRAL_BASE: u8 = 1;
    /// Batteries placed at the start; a team brings them to its container.
    pub const GATHER_VICTORY_ITEMS: u8 = 2;
    /// Batteries appear one by one during the match.
    pub const MATCH_TIME_VI_SPAWN: u8 = 3;
}

/// The lighting variants every map has, as its project names them.
pub const VARIANTS: [&str; 3] = ["day", "rain", "evening"];

/// The six maps: the project directory and the loading picture
/// (`switch_to_level_loading`) of each variant, in `VARIANTS` order.
pub const MAPS: [(&str, [u8; 3]); 6] = [
    ("level_01", [0x01, 0x13, 0x14]),
    ("level_02", [0x03, 0x15, 0x16]),
    ("level_03", [0x05, 0x09, 0x08]),
    ("level_04", [0x0A, 0x17, 0x18]),
    ("level_05", [0x19, 0x1B, 0x1A]),
    ("level_06", [0x1D, 0x24, 0x23]),
];

/// `match_stats_events_dict_enum` weights for the scoreboard. They only
/// count score, not the winner; the real values are unknown, these are
/// the spec's suggestion.
pub const SCORE_WEIGHTS: [f32; 24] = [
    100.0, // kill
    50.0,  // assist
    0.0,   // death
    0.0,   // suicide
    0.0,   // team kill
    20.0,  // artefact
    100.0, // bring victory item (x0.01 by the client)
    25.0,  // teammate cure
    25.0,  // teammate rescue
    25.0,  // revenge
    50.0,  // victory item carrier kill
    0.0,   // participating
    0.0,   // won
    25.0,  // melee kill
    25.0,  // silent kill
    25.0,  // headshot kill
    0.0,   // winning team victory item bonus
    100.0, // steal victory item (x0.01)
    100.0, // recover victory item (x0.01)
    25.0,  // victory item found
    50.0,  // put victory item in container
    25.0,  // entrap enemy
    0.0,   // bullet fired
    0.0,   // bullet hit
];

/// `match_options` as 0x51 carries it.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchOptions {
    /// The loading picture.
    pub level_id: u8,
    /// `level_0X/<variant>`.
    pub project: String,
    pub mode: u8,
    pub batteries: u8,
    pub respawn_seconds: u8,
    /// How long the match plays once it has begun.
    pub duration_seconds: u16,
    /// Batteries and anomalies; the same for every client.
    pub seed: u32,
    /// The share of the roster that must have entered for the countdown to
    /// start. 0 starts it with the first player in.
    pub start_fraction: f32,
    /// How long the match waits for players at most.
    pub waiting_minutes: u8,
    pub countdown_seconds: u8,
    pub score_weights: [f32; 24],
}

impl MatchOptions {
    /// A match on `MAPS[map]` in `VARIANTS[variant]`, with the spec's
    /// defaults.
    pub fn new(map: usize, variant: usize, mode: u8, seed: u32) -> Self {
        let (directory, pictures) = MAPS[map % MAPS.len()];
        let variant = variant % VARIANTS.len();
        Self {
            level_id: pictures[variant],
            project: format!("{directory}/{}", VARIANTS[variant]),
            mode,
            batteries: 5,
            respawn_seconds: 10,
            duration_seconds: 600,
            seed,
            start_fraction: 0.0,
            waiting_minutes: 5,
            countdown_seconds: 10,
            score_weights: SCORE_WEIGHTS,
        }
    }
}

/// One roster place.
pub struct MatchPlayer<'a> {
    /// 0 or 1: every map has spawn points for those two only.
    pub team: u8,
    /// The list shows players with the same non-zero id as a squad.
    pub squad: u8,
    pub name: &'a str,
    pub profile: &'a Profile,
}

/// 0x51's payload for the whole roster, without the trailing local player
/// index each client gets its own of.
pub fn static_info(options: &MatchOptions, players: &[MatchPlayer]) -> Vec<u8> {
    let players = &players[..players.len().min(MAX_PLAYERS)];
    let mut writer = Writer::default();
    writer
        .u8(options.level_id)
        .str(&options.project, MAX_PROJECT_LEN)
        .u8(options.mode)
        .u8(players.len() as u8)
        .u8(options.batteries.min(MAX_BATTERIES))
        .u8(options.respawn_seconds)
        .u16(options.duration_seconds)
        .u32(options.seed)
        .f32(options.start_fraction)
        .u8(options.waiting_minutes)
        .u8(0)
        .u8(options.countdown_seconds)
        // Read by nothing in the client.
        .u16(0);
    for weight in options.score_weights {
        writer.f32(weight);
    }
    for player in players {
        writer.u8(player.squad);
    }
    for player in players {
        write_profile_static(&mut writer, player.profile, player.team, player.name);
    }
    writer.done()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::Dictionary;
    use crate::v021d::account::Account;

    #[test]
    fn the_options_come_in_the_clients_order() {
        let options = MatchOptions::new(0, 0, game_mode::GATHER_VICTORY_ITEMS, 0x1234_5678);
        let info = static_info(&options, &[]);
        let mut expected = vec![1, 12];
        expected.extend(b"level_01/day");
        expected.extend([2, 0, 5, 10, 0x58, 2, 0x78, 0x56, 0x34, 0x12]);
        expected.extend([0, 0, 0, 0, 5, 0, 10, 0, 0]);
        expected.extend([0, 0, 0xC8, 0x42, 0, 0, 0x48, 0x42]);
        assert_eq!(info[..expected.len()], expected);
        // 129 bytes of options for map names of 12 characters.
        assert_eq!(info.len(), 1 + 13 + 10 + 4 + 5 + 96);
    }

    #[test]
    fn each_player_has_a_squad_id_then_a_profile_with_its_team_and_name() {
        let account = Account::with_test_loadout("alice", Dictionary::shared());
        let profile = &account.profiles[0];
        let options = MatchOptions::new(5, 2, game_mode::MATCH_TIME_VI_SPAWN, 1);
        assert_eq!(options.project, "level_06/evening");
        assert_eq!(options.level_id, 0x23);
        let players = [
            MatchPlayer {
                team: 0,
                squad: 0,
                name: "alice",
                profile,
            },
            MatchPlayer {
                team: 1,
                squad: 3,
                name: "bob",
                profile,
            },
        ];
        let info = static_info(&options, &players);
        let options_len = 1 + 17 + 10 + 4 + 5 + 96;
        assert_eq!(info[1 + 17 + 1], 2, "player count");
        assert_eq!(info[options_len..options_len + 2], [0, 3]);
        let profiles = &info[options_len + 2..];
        assert_eq!(profiles[..8], [0, 0, 5, b'a', b'l', b'i', b'c', b'e']);
        let bob_len = static_info(&options, &players[1..]).len() - options_len - 1;
        let second = profiles.len() - bob_len;
        assert_eq!(profiles[second..second + 6], [1, 0, 3, b'b', b'o', b'b']);
    }
}
