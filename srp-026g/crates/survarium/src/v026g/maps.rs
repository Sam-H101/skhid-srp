//! The maps and game modes of 0.26g (`docs/notes/modes_maps.md`,
//! `static_info.md` §4).
//!
//! A 0.26g match names its map, weather and mode by one byte: an id of
//! `maps_dict` in the client's own `gameplay/db_static_dictionaries`, which
//! names a file under `gameplay/game_modes/`. That file holds the mode, its
//! rules and every timing, the names the client shows, and the project the
//! client loads. The server sends only the id, so its own clock and scores
//! must use the same values: the table here is derived from the players'
//! files when first used ([`all`], [`super::game_data`]), and so is what
//! the server needs of each project: how many respawn points it has and
//! whose its battery containers are (its `server_objects`).
//!
//! An id can be played ([`MapMode::is_playable`]) when the game data has
//! its config, the config is of a mode srp plays (2, 3 or 4) and the data
//! has its project. No level or id is known here beforehand: a map a patch
//! adds is played as soon as the server runs on the data with it. Of
//! 0.26g's 89 ids, 72 can be played (modes 2, 3 and 4 of its maps in their
//! weathers); with London (`level_09`, patch 14 of skhid) 75. The others
//! must never be sent:
//! - 114 and 143..=151 name a config the client does not have ("Wrong map
//!   config. Map id = %u", 0x2C9D18);
//! - 107 and 108 (level_08 at night) and the test maps 11, 45 and 115 name a
//!   project that `resources.db` does not have;
//! - 141 (`loopback_client`) and 142 (`gather_server_resources`) are mode 0.

use binary_config_parser::tree::{ConfigError, ConfigValue};

use super::dictionary::Dictionary;
use super::game_data::{GameData, GameDataError};

#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;

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
    /// The modes whose maps have the teams' battery containers.
    pub const WITH_BATTERIES: [u8; 2] = [BATTERIES, TIMED_BATTERIES];
}

/// `match_making_params.map_weather_id`.
pub mod weather {
    pub const DAY: u8 = 1;
    pub const RAIN: u8 = 2;
    pub const EVENING: u8 = 3;
    pub const NIGHT: u8 = 4;
    pub const PRERAIN: u8 = 5;
}

/// One `maps_dict` id and its `gameplay/game_modes/` config, with what the
/// server reads of its project. A parameter the config lacks is 0.
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
    /// `match_making_params.map_id`: the map, whatever the weather and mode
    /// (`level_0N` is N; the test maps are 88). A number of the data's.
    pub level: u8,
    /// `match_making_params.map_weather_id` ([`weather`]).
    pub weather: u8,
    /// `match_making_params.map_name`: the project under `projects/`.
    pub project: &'static str,
    /// Whether the game data has that project's `client_project`.
    pub has_project: bool,
    /// `ui.loading_screen_id` and `ui.icon`.
    pub loading_screen_id: u8,
    pub icon: u8,
    /// `ui.map_name` and `ui.mode_name`: the keys of the map's and the
    /// mode's names in the game's localizations (`st_level_01_name`,
    /// `st_mode_04_name`; [`super::localization`]).
    pub map_name_key: &'static str,
    pub mode_name_key: &'static str,
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
    /// How many respawn points the project has: every entry of its
    /// `server_objects.respawn_points`, whatever its team or layer, is
    /// loaded (`player_respawn_rule_cook` 0x2C6910), so this is how many
    /// pairs of bytes the respawn rule writes into a snapshot
    /// (snapshot_rules.md §2.3). PROVEN (code and data). 0 without a
    /// project.
    pub respawn_points: usize,
    /// Modes 2 and 3: whose the two battery containers are, by container
    /// id: `victory_items_containers[..].team` of the project by its `id`.
    /// PROVEN (data). A battery's container id in a snapshot is this id,
    /// not the team. `None` in mode 4 and without a project.
    pub container_teams: Option<[u8; 2]>,
}

impl MapMode {
    /// Whether a match can be played on it: a mode srp plays ([`mode::ALL`]),
    /// on a map whose project the game data has. Its config is there, or
    /// it would have no row.
    pub fn is_playable(&self) -> bool {
        self.has_project && mode::ALL.contains(&self.mode)
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

/// What the server reads of a project (its `client_project`, or the
/// server data's cut of it: [`resources_db::server::server_objects_of`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectObjects {
    /// [`MapMode::respawn_points`].
    pub respawn_points: usize,
    /// The battery containers' teams by container id, if the project has
    /// containers ([`MapMode::container_teams`]).
    pub container_teams: Option<[u8; 2]>,
}

impl ProjectObjects {
    /// The respawn points and the battery containers of the project
    /// `project`, a parsed `client_project`. Containers there must be two,
    /// ids 0 and 1, of teams 0 and 1, one each: the gather rules and the
    /// scores know no other.
    /// `with_batteries`: the containers are read (and must be one of each
    /// team) only for a mode that has batteries; a Team Deathmatch project
    /// copied from a batteries one may keep containers it never uses.
    pub fn from_project(project: &ConfigValue, with_batteries: bool) -> Result<Self, ConfigError> {
        let objects = project.get("server_objects");
        let respawn_points = objects
            .and_then(|objects| objects.get("respawn_points"))
            .map_or(0, |points| points.children().len());
        let containers = objects
            .filter(|_| with_batteries)
            .and_then(|objects| objects.get("victory_items_containers"))
            .map(ConfigValue::children)
            .unwrap_or_default();
        let container_teams = if containers.is_empty() {
            None
        } else {
            let mut teams: [Option<u8>; 2] = [None, None];
            for container in &containers {
                let id = container.i32_at("id")?;
                let team = container.i32_at("team")?;
                let place = usize::try_from(id)
                    .ok()
                    .filter(|&id| id < 2)
                    .ok_or_else(|| ConfigError(format!("a battery container has id {id}")))?;
                if !(0..2).contains(&team) {
                    return Err(ConfigError(format!(
                        "battery container {id} is of team {team}"
                    )));
                }
                if teams[place].replace(team as u8).is_some() {
                    return Err(ConfigError(format!("two battery containers have id {id}")));
                }
            }
            match teams {
                [Some(first), Some(second)] if first != second => Some([first, second]),
                _ => {
                    return Err(ConfigError(format!(
                        "the battery containers are not one of each team: {teams:?}"
                    )));
                }
            }
        };
        Ok(Self {
            respawn_points,
            container_teams,
        })
    }
}

impl MapMode {
    /// The row of a `maps_dict` id's config (`gameplay/game_modes/`
    /// `cfg_mode_name`) and what the server reads of its project, if the
    /// data has the project ([`ProjectObjects::from_project`]). A playable
    /// id ([`MapMode::is_playable`]) must have a respawn point, and one of
    /// modes 2 and 3 the two teams' battery containers. The strings are
    /// leaked: the rows live as long as the process.
    pub fn from_config(
        id: u8,
        cfg_mode_name: &str,
        config: &ConfigValue,
        project: Option<&ConfigValue>,
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
        let game_mode: u8 = int(Some(mode), "mode")?;
        let objects = project
            .map(|project| {
                ProjectObjects::from_project(project, mode::WITH_BATTERIES.contains(&game_mode))
            })
            .transpose()?
            .unwrap_or(ProjectObjects {
                respawn_points: 0,
                container_teams: None,
            });
        let row = Self {
            id,
            config: leak_str(cfg_mode_name),
            name: text(Some(mode), "name")?,
            mode: game_mode,
            level: int(making, "map_id")?,
            weather: int(making, "map_weather_id")?,
            project: text(making, "map_name")?,
            has_project: project.is_some(),
            loading_screen_id: int(ui, "loading_screen_id")?,
            icon: int(ui, "icon")?,
            map_name_key: text(ui, "map_name")?,
            mode_name_key: text(ui, "mode_name")?,
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
            respawn_points: objects.respawn_points,
            container_teams: objects
                .container_teams
                .filter(|_| mode::WITH_BATTERIES.contains(&game_mode)),
        };
        if row.is_playable() {
            if row.respawn_points == 0 {
                return Err(ConfigError(format!(
                    "its project {} has no respawn points",
                    row.project
                )));
            }
            if mode::WITH_BATTERIES.contains(&row.mode) && row.container_teams.is_none() {
                return Err(ConfigError(format!(
                    "its project {} has no battery containers, and it is mode {}",
                    row.project, row.mode
                )));
            }
        }
        Ok(row)
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
/// by id, with what the server reads of its project if `data` has it
/// ([`MapMode::from_config`]). An id whose config the game lacks is left
/// out, and so never sent (114 and 143..=151 of 0.26g).
pub fn derive(data: &GameData, dictionary: &Dictionary) -> Result<Vec<MapMode>, GameDataError> {
    let mut rows = Vec::new();
    for entry in &dictionary.maps {
        let path = format!("{GAME_MODES_DIR}{}", entry.cfg_mode_name);
        let Some(config) = super::game_data::config(data, &path)? else {
            continue;
        };
        let project_name = config
            .get("mode")
            .and_then(|mode| mode.get("match_making_params"))
            .and_then(|making| making.get("map_name"))
            .and_then(ConfigValue::as_str)
            .unwrap_or("");
        let project = data
            .project(project_name)
            .map(|bytes| {
                ConfigValue::parse(bytes).map_err(|cause| {
                    GameDataError(format!(
                        "projects/{project_name}/client_project: {}",
                        cause.0
                    ))
                })
            })
            .transpose()?;
        let row = MapMode::from_config(entry.id, &entry.cfg_mode_name, &config, project.as_ref())
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

/// The id for a level, weather and mode, if it can be played.
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

    /// What srp had by hand before it read the projects, levels 1 to 9:
    /// the respawn points of modes 2 and 3 and of mode 4, and the battery
    /// containers' teams by id.
    const GATHER_BEFORE: [usize; 9] = [39, 30, 28, 36, 29, 30, 31, 26, 20];
    const TDM_BEFORE: [usize; 9] = [43, 31, 22, 75, 29, 30, 31, 26, 20];
    fn teams_before(level: u8) -> [u8; 2] {
        if matches!(level, 2 | 7) {
            [0, 1]
        } else {
            [1, 0]
        }
    }

    /// Every playable row of `table` of a level of 1 to 9 has what srp
    /// had by hand.
    fn as_before(table: &[MapMode]) {
        for map in table.iter().filter(|map| map.is_playable()) {
            let level = usize::from(map.level);
            assert!((1..=9).contains(&level), "{}", map.id);
            let (points, teams) = match map.mode {
                mode::TEAM_DEATHMATCH => (TDM_BEFORE[level - 1], None),
                _ => (GATHER_BEFORE[level - 1], Some(teams_before(map.level))),
            };
            assert_eq!(map.respawn_points, points, "{} ({})", map.id, map.project);
            assert_eq!(map.container_teams, teams, "{} ({})", map.id, map.project);
        }
    }

    #[test]
    fn the_projects_give_what_srp_had_by_hand() {
        // The test data: 0.26g's maps, levels 1 to 8, all 72.
        assert_eq!(playable().count(), 72);
        as_before(all());
        // Spot checks: level_04's Team Deathmatch has 75 points (39 of
        // layer `tdm`), level_02's 31 (one of team 2).
        assert_eq!(find(4, weather::RAIN, 4).unwrap().respawn_points, 75);
        assert_eq!(find(2, weather::DAY, 4).unwrap().respawn_points, 31);
        assert_eq!(
            find(7, weather::DAY, 3).unwrap().container_teams,
            Some([0, 1])
        );
        // Without a project, nothing: level_08 at night.
        let night = map_mode(107).unwrap();
        assert_eq!((night.respawn_points, night.container_teams), (0, None));
        // The names are the localizations' keys.
        let map = map_mode(61).unwrap();
        assert_eq!(
            (map.map_name_key, map.mode_name_key),
            ("st_level_01_name", "st_mode_02_name")
        );
    }

    #[test]
    fn a_map_of_a_patch_is_played_from_its_data() {
        // Team Deathmatch on a level 12, a map no code names.
        let added = fixtures::AddedMap::level_12_tdm();
        let data = fixtures::with_map(&added);
        let table = fixtures::table_of(&data).unwrap();
        // The stock rows are as they were; the map is one more.
        assert_eq!(table.len(), all().len() + 1);
        let map = *table.iter().find(|map| map.id == added.id).unwrap();
        assert_eq!(
            table.iter().filter(|row| row.id != added.id).count(),
            all().len()
        );
        assert!(
            table
                .iter()
                .filter(|row| row.id != added.id)
                .eq(all().iter())
        );
        assert!(map.is_playable());
        assert_eq!(
            (map.level, map.mode, map.project),
            (12, 4, "level_12/day_4")
        );
        // Its respawn points are its project's, and it has no containers.
        assert_eq!(map.respawn_points, 17);
        assert_eq!(map.container_teams, None);
        // It is played in mode 4 only.
        let playable_in = |game_mode: u8| -> Vec<u8> {
            table
                .iter()
                .filter(|map| map.is_playable() && map.mode == game_mode)
                .map(|map| map.id)
                .collect()
        };
        assert!(playable_in(mode::TEAM_DEATHMATCH).contains(&added.id));
        assert_eq!(playable_in(mode::TEAM_DEATHMATCH).len(), 25);
        for game_mode in mode::WITH_BATTERIES {
            assert!(!playable_in(game_mode).contains(&added.id));
            assert_eq!(playable_in(game_mode), ids(game_mode));
        }
        assert_eq!(table.iter().filter(|map| map.is_playable()).count(), 73);
    }

    #[test]
    fn a_map_whose_project_the_server_cannot_play_stops_it() {
        // Batteries on a map without its two containers, or with two of
        // one team; a map without a respawn point.
        let batteries = |containers: Vec<(i32, i32)>, respawn_points| fixtures::AddedMap {
            id: 201,
            config: "level_12_day_2.options",
            project: "level_12/day_2",
            mode: mode::BATTERIES,
            containers,
            respawn_points,
            ..fixtures::AddedMap::level_12_tdm()
        };
        for (map, why) in [
            (batteries(Vec::new(), 10), "no battery containers"),
            (batteries(vec![(0, 1), (1, 1)], 10), "not one of each team"),
            (
                batteries(vec![(0, 1), (0, 0)], 10),
                "two battery containers have id 0",
            ),
            (batteries(vec![(0, 1), (2, 0)], 10), "id 2"),
            (batteries(vec![(0, 1), (1, 0)], 0), "no respawn points"),
        ] {
            let message = fixtures::table_of(&fixtures::with_map(&map))
                .unwrap_err()
                .to_string();
            assert!(message.contains("level_12_day_2.options"), "{message}");
            assert!(message.contains(why), "{message}");
        }
        // With them: played, its containers' teams by id.
        let table =
            fixtures::table_of(&fixtures::with_map(&batteries(vec![(1, 0), (0, 1)], 12))).unwrap();
        let map = table.iter().find(|map| map.id == 201).unwrap();
        assert!(map.is_playable());
        assert_eq!(
            (map.respawn_points, map.container_teams),
            (12, Some([1, 0]))
        );
        // Team Deathmatch reads no containers: a project copied from a
        // batteries one with a stray container (one, of an odd id) is played.
        let tdm = fixtures::AddedMap {
            containers: vec![(3, 1)],
            ..fixtures::AddedMap::level_12_tdm()
        };
        let table = fixtures::table_of(&fixtures::with_map(&tdm)).unwrap();
        let map = table.iter().find(|map| map.id == 200).unwrap();
        assert!(map.is_playable());
        assert_eq!(map.container_teams, None);
        // A mode srp does not play is in the table, not played, whatever
        // its project.
        let other = fixtures::AddedMap {
            mode: 5,
            respawn_points: 0,
            ..fixtures::AddedMap::level_12_tdm()
        };
        let table = fixtures::table_of(&fixtures::with_map(&other)).unwrap();
        assert!(
            !table
                .iter()
                .find(|map| map.id == 200)
                .unwrap()
                .is_playable()
        );
    }

    fn env_path(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name}")))
    }

    /// The live game data: 0.26g's archive (`SRP_TEST_RESOURCES_DB`) with
    /// skhid's patches (`SRP_TEST_SKHID_PATCHES`, by spaces: patch 12 and
    /// patch 14, London) over it. The maps of 0.26g, levels 1 to 8, have
    /// what srp had by hand; London is played too (ids 181 to 183) with
    /// its project's values, and named by its patch.
    #[test]
    #[ignore = "needs 0.26g's resources.db and skhid's patches"]
    fn the_live_data_gives_what_srp_had_by_hand_and_london() {
        use resources_db::server::{PatchFile, build, url_in};
        let db = env_path("SRP_TEST_RESOURCES_DB");
        let patches: Vec<PatchFile> = std::env::var("SRP_TEST_SKHID_PATCHES")
            .expect("SRP_TEST_SKHID_PATCHES")
            .split_whitespace()
            .map(|path| {
                let path = std::path::PathBuf::from(path);
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                PatchFile {
                    urls: vec![url_in("http://203.0.113.5/patches", &name)],
                    path,
                }
            })
            .collect();
        let built = build(&db, &patches).unwrap();
        let mut archive = resources_db::Archive::new(std::io::Cursor::new(built.bytes)).unwrap();
        let data = GameData::from_archive(&mut archive, "live".into()).unwrap();
        super::super::tables::check(&data).unwrap();
        let table = fixtures::table_of(&data).unwrap();
        let played: Vec<&MapMode> = table.iter().filter(|map| map.is_playable()).collect();
        let stock: Vec<u8> = playable().map(|map| map.id).collect();
        let mut expected = stock.clone();
        expected.extend([181, 182, 183]);
        assert_eq!(
            played.iter().map(|map| map.id).collect::<Vec<_>>(),
            expected
        );
        for map in &played {
            eprintln!(
                "{:3} level {} weather {} mode {} {:24} respawn points {:2} containers {:?}",
                map.id,
                map.level,
                map.weather,
                map.mode,
                map.project,
                map.respawn_points,
                map.container_teams
            );
        }
        // The stock maps are as in the test data, and as before.
        for map in table.iter().filter(|map| stock.contains(&map.id)) {
            assert_eq!(Some(map), map_mode(map.id), "{}", map.id);
        }
        as_before(&table);
        let london: Vec<&&MapMode> = played.iter().filter(|map| map.level == 9).collect();
        assert_eq!(london.len(), 3);
        for map in london {
            assert_eq!(map.respawn_points, 20, "{}", map.id);
            if map.mode != mode::TEAM_DEATHMATCH {
                assert_eq!(map.container_teams, Some([1, 0]), "{}", map.id);
            }
            let names = |language| {
                super::super::localization::of(&data, language)
                    .unwrap()
                    .unwrap()
                    .get(map.map_name_key)
                    .map(str::to_owned)
            };
            use super::super::localization::Language;
            assert_eq!(names(Language::English).as_deref(), Some("London"));
            assert_eq!(names(Language::Ukrainian).as_deref(), Some("Лондон"));
        }
    }
}
