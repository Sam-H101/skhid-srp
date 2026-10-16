//! The maps and game modes of 0.26g (`docs/notes/modes_maps.md`,
//! `static_info.md` §4).
//!
//! A 0.26g match names its map, weather and mode by one byte: an id of
//! `maps_dict` in the client's own `gameplay/db_static_dictionaries`, which
//! names a file under `gameplay/game_modes/`. That file holds the mode, its
//! rules and every timing, and the project the client loads. The server
//! sends only the id, so its own clock and scores must use the same values:
//! the table here is derived from the players' files when first used
//! ([`all`], [`super::game_data`]).
//!
//! Of the 89 ids, 72 can be played ([`playable`]): modes 2, 3 and 4 on
//! eight maps in three weathers. The rest must never be sent:
//! - 114 and 143..=151 name a config the client does not have ("Wrong map
//!   config. Map id = %u", 0x2C9D18);
//! - 107 and 108 (level_08 at night) and the test maps 11, 45 and 115 name a
//!   project that `resources.db` does not have;
//! - 141 (`loopback_client`) and 142 (`gather_server_resources`) are mode 0.

use binary_config_parser::tree::{ConfigError, ConfigValue};

use super::dictionary::Dictionary;
use super::game_data::{GameData, GameDataError};

/// Where `maps_dict`'s `cfg_mode_name` is.
pub const GAME_MODES_DIR: &str = "gameplay/game_modes/";

/// `mode.mode` of the configs.
pub mod mode {
    /// Gather batteries ("Protective device"), rule class 88.
    pub const BATTERIES: u8 = 2;
    /// Batteries that appear over time, with charging stations; rule 89.
    pub const TIMED_BATTERIES: u8 = 3;
    /// Team Deathmatch with tickets; rule 90.
    pub const TEAM_DEATHMATCH: u8 = 4;
    /// The modes a match can be.
    pub const ALL: [u8; 3] = [BATTERIES, TIMED_BATTERIES, TEAM_DEATHMATCH];
}

/// `match_making_params.map_weather_id`.
pub mod weather {
    pub const DAY: u8 = 1;
    pub const RAIN: u8 = 2;
    pub const EVENING: u8 = 3;
    pub const NIGHT: u8 = 4;
    pub const PRERAIN: u8 = 5;
}

/// One `maps_dict` id and its `gameplay/game_modes/` config. A parameter the
/// config lacks is 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapMode {
    /// The `maps_dict` id: the match options' `u8 map_id`.
    pub id: u8,
    /// `cfg_mode_name`, the file under `gameplay/game_modes/`.
    pub config: &'static str,
    /// `mode.name`.
    pub name: &'static str,
    /// `mode.mode` ([`mode`]).
    pub mode: u8,
    /// `match_making_params.map_id`: 1..=8 are level_01..=level_08, 9 is
    /// London (level_09, resources.db.patch_13), 88 the test maps.
    pub level: u8,
    /// `match_making_params.map_weather_id` ([`weather`]).
    pub weather: u8,
    /// `match_making_params.map_name`: the project under `projects/`.
    pub project: &'static str,
    /// Whether `resources.db` has that project's `client_project`.
    pub has_project: bool,
    /// `ui.loading_screen_id` and `ui.icon`.
    pub loading_screen_id: u8,
    pub icon: u8,
    /// `rules`: the rule resource classes the client loads.
    pub rules: &'static [u8],
    /// Seconds a match lasts.
    pub match_time: u32,
    pub wait_time: u32,
    /// Seconds of countdown before the match starts.
    pub countdown_time: u32,
    /// The share of the roster that must be in before the countdown.
    pub wait_player_percent: f32,
    /// Seconds from a death to the respawn.
    pub respawn_time: u32,
    /// Modes 2 and 3: the batteries that win.
    pub victory_items_count: u32,
    /// Mode 3: seconds between batteries, give or take the variation, and
    /// the charging stations in play.
    pub vi_spawn_time: u32,
    pub vi_spawn_variation: u32,
    pub spawn_points_count: u32,
    /// Mode 4: a team's tickets.
    pub tickets_count: u32,
}

impl MapMode {
    /// Whether a match can be played on it: a real mode, on a map whose
    /// project the client has. Level 9 is London: its modes and projects
    /// come with resources.db.patch_13 (day only), so the stock data has no
    /// level 9 to play.
    pub fn is_playable(&self) -> bool {
        self.has_project && mode::ALL.contains(&self.mode) && (1..=9).contains(&self.level)
    }
}

/// A number of a game mode config; a missing one is 0.
fn int<T: TryFrom<i32>>(table: Option<&ConfigValue>, key: &str) -> Result<T, ConfigError> {
    let number = match table.and_then(|table| table.get(key)) {
        None => 0,
        Some(value) => value
            .as_i32()
            .ok_or_else(|| ConfigError(format!("`{key}` is not a number")))?,
    };
    T::try_from(number).map_err(|_| ConfigError(format!("`{key}` = {number} is out of range")))
}

fn leak_str(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

impl MapMode {
    /// The row of a `maps_dict` id's config (`gameplay/game_modes/`
    /// `cfg_mode_name`); whether the archive has its project is the
    /// caller's to know. The strings are leaked: the rows live as long as
    /// the process.
    pub fn from_config(
        id: u8,
        cfg_mode_name: &str,
        config: &ConfigValue,
        has_project: bool,
    ) -> Result<Self, ConfigError> {
        let mode = config.req("mode")?;
        let p = mode.get("parameters");
        let making = mode.get("match_making_params");
        let ui = mode.get("ui");
        let text = |table: Option<&ConfigValue>, key: &str| -> Result<&'static str, ConfigError> {
            match table.and_then(|table| table.get(key)) {
                None => Ok(""),
                Some(value) => value
                    .as_str()
                    .map(leak_str)
                    .ok_or_else(|| ConfigError(format!("`{key}` is not a string"))),
            }
        };
        let rules: Vec<u8> = match mode.get("rules") {
            None => Vec::new(),
            Some(list) => list
                .children()
                .into_iter()
                .map(|rule| {
                    rule.as_i32()
                        .and_then(|rule| u8::try_from(rule).ok())
                        .ok_or_else(|| ConfigError("a `rules` entry is not a class id".into()))
                })
                .collect::<Result<_, _>>()?,
        };
        let wait_player_percent = match p.and_then(|p| p.get("wait_player_percent")) {
            None => 0.0,
            Some(value) => value
                .as_f32()
                .ok_or_else(|| ConfigError("`wait_player_percent` is not a number".into()))?,
        };
        Ok(Self {
            id,
            config: leak_str(cfg_mode_name),
            name: text(Some(mode), "name")?,
            mode: int(Some(mode), "mode")?,
            level: int(making, "map_id")?,
            weather: int(making, "map_weather_id")?,
            project: text(making, "map_name")?,
            has_project,
            loading_screen_id: int(ui, "loading_screen_id")?,
            icon: int(ui, "icon")?,
            rules: if rules.is_empty() {
                &[]
            } else {
                Box::leak(rules.into_boxed_slice())
            },
            match_time: int(p, "match_time")?,
            wait_time: int(p, "wait_time")?,
            countdown_time: int(p, "countdown_time")?,
            wait_player_percent,
            respawn_time: int(p, "respawn_time")?,
            victory_items_count: int(p, "victory_items_count")?,
            vi_spawn_time: int(p, "vi_spawn_time")?,
            vi_spawn_variation: int(p, "vi_spawn_variation")?,
            spawn_points_count: int(p, "spawn_points_count")?,
            tickets_count: int(p, "tickets_count")?,
        })
    }
}

/// Every `maps_dict` id whose config the players have, by id ([`derive`]).
pub fn all() -> &'static [MapMode] {
    static SHARED: std::sync::OnceLock<Vec<MapMode>> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| {
        derive(super::game_data::shared(), Dictionary::shared()).expect("the game modes read")
    })
}

/// A row for each `maps_dict` id of `dictionary` whose config `data` has,
/// by id. An id whose config the game lacks is left out, and so never sent
/// (114 and 143..=151 of 0.26g); `has_project` is whether `data` has the
/// config's project.
pub fn derive(data: &GameData, dictionary: &Dictionary) -> Result<Vec<MapMode>, GameDataError> {
    let mut rows = Vec::new();
    for entry in &dictionary.maps {
        let path = format!("{GAME_MODES_DIR}{}", entry.cfg_mode_name);
        let Some(config) = super::game_data::config(data, &path)? else {
            continue;
        };
        let project = config
            .get("mode")
            .and_then(|mode| mode.get("match_making_params"))
            .and_then(|making| making.get("map_name"))
            .and_then(ConfigValue::as_str)
            .unwrap_or("");
        let row = MapMode::from_config(
            entry.id,
            &entry.cfg_mode_name,
            &config,
            data.has_project(project),
        )
        .map_err(|cause| GameDataError(format!("{path}: {}", cause.0)))?;
        rows.push(row);
    }
    rows.sort_by_key(|row| row.id);
    if let Some(pair) = rows.windows(2).find(|w| w[0].id == w[1].id) {
        return Err(GameDataError(format!(
            "map id {} is in maps_dict twice",
            pair[0].id
        )));
    }
    Ok(rows)
}

/// The config of a `maps_dict` id, if the client has one.
pub fn map_mode(id: u8) -> Option<&'static MapMode> {
    let all = all();
    all.binary_search_by_key(&id, |map| map.id)
        .ok()
        .map(|index| &all[index])
}

/// Every id a match may use, by id.
pub fn playable() -> impl Iterator<Item = &'static MapMode> {
    all().iter().filter(|map| map.is_playable())
}

/// The ids a match of `mode` may use.
pub fn playable_in(mode: u8) -> impl Iterator<Item = &'static MapMode> {
    playable().filter(move |map| map.mode == mode)
}

/// The id for a level (1..=8), weather and mode, if it can be played.
pub fn find(level: u8, weather: u8, mode: u8) -> Option<&'static MapMode> {
    playable().find(|map| (map.level, map.weather, map.mode) == (level, weather, mode))
}

#[cfg(test)]
mod tests {
    use super::super::dictionary::Dictionary;
    use super::*;

    fn ids(mode: u8) -> Vec<u8> {
        playable_in(mode).map(|map| map.id).collect()
    }

    #[test]
    fn the_whitelist_is_that_of_the_notes() {
        // modes_maps.md §8.
        let mode_2: Vec<u8> = (61..=105).step_by(2).chain([109]).collect();
        let mode_3: Vec<u8> = (62..=106).step_by(2).chain([110]).collect();
        let mode_4: Vec<u8> = (117..=140).collect();
        assert_eq!(ids(mode::BATTERIES), mode_2);
        assert_eq!(ids(mode::TIMED_BATTERIES), mode_3);
        assert_eq!(ids(mode::TEAM_DEATHMATCH), mode_4);
        assert_eq!(playable().count(), 72);
        for bad in [11, 45, 107, 108, 114, 115].into_iter().chain(141..=151) {
            assert!(playable().all(|map| map.id != bad), "{bad}");
        }
        // 114 and 143..=151 have no config at all.
        for missing in [114].into_iter().chain(143..=151) {
            assert!(map_mode(missing).is_none(), "{missing}");
        }
        // 9 ids a level: 3 weathers × 3 modes.
        for level in 1..=8 {
            assert_eq!(playable().filter(|map| map.level == level).count(), 9);
        }
    }

    #[test]
    fn the_table_is_the_dictionarys_maps_dict() {
        let dictionary = Dictionary::shared();
        assert!(all().windows(2).all(|w| w[0].id < w[1].id));
        // Every row is a maps_dict id with that config name.
        for map in all() {
            let entry = dictionary.maps.iter().find(|entry| entry.id == map.id);
            assert_eq!(entry.unwrap().cfg_mode_name, map.config, "{}", map.id);
        }
        // The ids without a row are the ten whose config the db lacks.
        let missing: Vec<u8> = dictionary
            .maps
            .iter()
            .map(|entry| entry.id)
            .filter(|&id| map_mode(id).is_none())
            .collect();
        assert_eq!(missing, [114, 143, 144, 145, 146, 147, 148, 149, 150, 151]);
        // A playable id's config name spells its level, weather and mode,
        // and its project is `level_0X/<weather>_<mode>`.
        for map in playable() {
            let weather = match map.weather {
                weather::DAY => "day",
                weather::RAIN => "rain",
                weather::EVENING => "evn",
                weather::PRERAIN => "prerain",
                other => panic!("{} has weather {other}", map.id),
            };
            let config = format!("level_0{}_{weather}_{}.options", map.level, map.mode);
            assert_eq!(map.config, config);
            let long = if weather == "evn" { "evening" } else { weather };
            let project = format!("level_0{}/{long}_{}", map.level, map.mode);
            assert_eq!(map.project, project);
            assert!(map.project.len() <= 31);
        }
    }

    #[test]
    fn spot_checks() {
        let map = map_mode(61).unwrap();
        assert_eq!((map.level, map.weather, map.mode), (1, weather::DAY, 2));
        assert_eq!(map.project, "level_01/day_2");
        assert_eq!(map.rules, [93, 91, 88, 94]);
        let map = map_mode(109).unwrap();
        assert_eq!((map.level, map.weather, map.mode), (8, weather::PRERAIN, 2));
        assert_eq!(map.loading_screen_id, 25);
        assert_eq!(find(8, weather::PRERAIN, 4).unwrap().id, 140);
        assert_eq!(find(1, weather::EVENING, 3).unwrap().id, 64);
        assert_eq!(find(4, weather::RAIN, 4).unwrap().id, 128);
        // Level 8 has no rain, and its night has no project.
        assert!(find(8, weather::RAIN, 2).is_none());
        assert!(find(8, weather::NIGHT, 2).is_none());
        assert!(!map_mode(107).unwrap().has_project);
        // Loading screens, static_info.md §4.
        assert_eq!(find(5, weather::EVENING, 2).unwrap().loading_screen_id, 14);
        assert_eq!(find(7, weather::DAY, 4).unwrap().loading_screen_id, 18);
    }

    #[test]
    fn every_playable_mode_has_the_same_timings() {
        for map in playable() {
            assert_eq!(map.match_time, 900, "{}", map.id);
            assert_eq!(map.wait_time, 5, "{}", map.id);
            assert_eq!(map.countdown_time, 20, "{}", map.id);
            assert_eq!(map.wait_player_percent, 0.8, "{}", map.id);
            assert_eq!(map.respawn_time, 8, "{}", map.id);
            let (rule, icon) = match map.mode {
                mode::BATTERIES => (88, 0),
                mode::TIMED_BATTERIES => (89, 0),
                _ => (90, 2),
            };
            assert_eq!(map.rules, [93, 91, rule, 94], "{}", map.id);
            assert_eq!(map.icon, icon, "{}", map.id);
            match map.mode {
                mode::TEAM_DEATHMATCH => {
                    // 150 tickets on level_05 and level_08, else 100.
                    let tickets = if matches!(map.level, 5 | 8) { 150 } else { 100 };
                    assert_eq!(map.tickets_count, tickets, "{}", map.id);
                    assert_eq!(map.victory_items_count, 0);
                }
                _ => {
                    assert_eq!(map.victory_items_count, 10, "{}", map.id);
                    assert_eq!(map.tickets_count, 0);
                }
            }
            if map.mode == mode::TIMED_BATTERIES {
                assert_eq!((map.vi_spawn_time, map.vi_spawn_variation), (60, 10));
                let stations = if map.level == 8 { 2 } else { 3 };
                assert_eq!(map.spawn_points_count, stations, "{}", map.id);
            } else {
                assert_eq!((map.vi_spawn_time, map.spawn_points_count), (0, 0));
            }
        }
    }
}
