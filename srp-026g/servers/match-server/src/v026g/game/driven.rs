//! The match server's own bots (`MatchPlayer::driven`, `SRP_SERVER_BOTS`):
//! places with no client. Stages 1 and 2 of the player bots
//! (`docs/notes/player_bots.md` §6).
//!
//! A real client simulates every player from the inputs relayed to it,
//! and a relayed input is applied unchecked (`player_bots.md` §1). So the
//! server enters such a place itself (0xE9, as `finish_sync` does for a
//! client) and relays inputs it makes up for it, with nothing to receive:
//! - it enters once a real client simulates and the match has a clock,
//!   [`ROSTER_LEAD_MS`] ahead, so the 0xE9 reaches that client before its
//!   tick, as a second player's does; a client that starts later gets the
//!   entry and the inputs from the history, as it gets anyone's;
//! - its inputs are one a tick at most, on the grid, 50 to 100 ms ahead of
//!   the match clock, so never late, moved or dropped; each is relayed and
//!   kept in the history as a client's is (`Game::input`, from the relay
//!   on);
//! - it is no witness of the world (`Game::is_bot`): no reports, no
//!   uploads, never counted among the clients that agree;
//! - it never leaves by itself and never times out: a match with only
//!   bots in it is over after [`BOTS_ALONE`] (`check_bots_alone`).
//!
//! Where its inputs come from:
//! - with `SRP_MAPDATA_DIR` ([`Walkers`], stage 2): `super::super::bots`,
//!   bots that walk the map. They see their bodies in the worlds the match
//!   trusts (`observe` → [`Game::observe_driven`]); while they walk the
//!   clients are asked for a world every [`WALKING_REFRESH`], and every
//!   [`WAITING_REFRESH`] while one waits to see its body (just entered,
//!   or dead). Until the map's walk grid is built (0.3 to 0.8 s, on a
//!   thread of its own) they stand; if the map has no file they run the
//!   stage 0 script instead;
//! - without it, the stage 0 script (`super::super::drive`).

use std::sync::Arc;

use mapdata::World;
use survarium::v026g::match_msgs::motion::Motion;
use survarium::v026g::match_msgs::players;

use super::*;
use crate::v026g::bots::Bots;
use crate::v026g::bots::body::Fix;
use crate::v026g::bots::maps::{MapState, Maps};
use crate::v026g::drive::{self, Drive};

/// While a walking bot is on its way, a world this often (the clients'
/// upload: two of them, 30 to 60 KB each [LIKELY, the worlds saved]).
const WALKING_REFRESH: Duration = Duration::from_secs(2);
/// While a bot waits to see its body, sooner.
const WAITING_REFRESH: Duration = Duration::from_secs(1);

/// The walking bots of a match and their map.
pub(super) struct Walkers {
    bots: Bots,
    maps: Arc<Maps>,
    project: &'static str,
    /// Held once built, so the store may drop it while the match runs.
    world: Option<Arc<World>>,
    /// The map has no walk grid: the stage 0 script drives them.
    absent: bool,
}

impl Walkers {
    /// With `SRP_MAPDATA_DIR` and a server's bot in the match; the map's
    /// build starts now.
    pub(super) fn new(formed: &Formed, settings: &Settings) -> Option<Self> {
        let maps = settings.maps.clone()?;
        let driven: Vec<bool> = formed.places.iter().map(|place| place.driven).collect();
        if !driven.iter().any(|&driven| driven) {
            return None;
        }
        let project = formed.map.project;
        maps.request(project);
        Some(Self {
            bots: Bots::new(formed.match_id, &driven),
            maps,
            project,
            world: None,
            absent: false,
        })
    }

    /// The map's walk grid, once built; `absent` set if there is none.
    fn world(&mut self, id: u32) -> Option<Arc<World>> {
        if self.world.is_none() && !self.absent {
            match self.maps.get(self.project) {
                MapState::Ready(world) => {
                    log::info!(
                        "Match {id:#x}: the bots have the map {} ({} floors)",
                        self.project,
                        world.grid.len()
                    );
                    self.world = Some(world);
                }
                MapState::Loading => {}
                MapState::Absent(why) => {
                    log::warn!(
                        "Match {id:#x}: no walk grid for {} ({why}); the server's bots run the \
                         stage 0 script",
                        self.project
                    );
                    self.absent = true;
                }
            }
        }
        self.world.clone()
    }
}

impl Game {
    /// The scripts of the server's bots, by place.
    pub(super) fn new_drives(formed: &Formed) -> Vec<Option<Drive>> {
        formed
            .places
            .iter()
            .map(|place| place.driven.then(|| Drive::new(drive::stage_0())))
            .collect()
    }

    /// A place a late joiner takes: never one of the server's bots.
    pub(super) fn push_driven_place(&mut self) {
        self.drives.push(None);
        if let Some(walkers) = self.walkers.as_mut() {
            walkers.bots.push_place();
        }
    }

    /// How often to ask for a world, `every` being what the match asks:
    /// sooner while the server's bots walk.
    pub(super) fn driven_refresh(&self, every: Duration) -> Duration {
        let Some(walkers) = self.walkers.as_ref().filter(|walkers| !walkers.absent) else {
            return every;
        };
        if walkers.bots.need_world() {
            every.min(WAITING_REFRESH)
        } else if walkers.bots.any() {
            every.min(WALKING_REFRESH)
        } else {
            every
        }
    }

    /// A trusted world: what it shows of the server's bots' bodies.
    pub(super) fn observe_driven(&mut self, snapshot: &Snapshot) {
        let id = self.id;
        let Some(walkers) = self.walkers.as_mut() else {
            return;
        };
        let world = walkers.world.clone();
        let Some((blocks, _)) = players::walk(&snapshot.body) else {
            return;
        };
        for block in blocks {
            let place = usize::from(block.place);
            if !walkers.bots.has_entered(place) {
                continue;
            }
            let Some((_, motion)) = Motion::find(block.state, snapshot.time) else {
                log::debug!(
                    "Match {id:#x}: player {place} (the server's bot) is not read in the world at \
                     {} ms",
                    snapshot.time
                );
                continue;
            };
            let fix = Fix {
                time: snapshot.time,
                alive: motion.alive,
                inserted: motion.inserted_time_ms,
                feet: motion
                    .body
                    .as_ref()
                    .map(|body| mapdata::Vec3::from_array(body.feet())),
                yaw: motion.rotation_y,
                pitch: motion.look_pitch,
                keys: motion.actions_mask,
                speed: motion
                    .damage
                    .as_ref()
                    .map(|damage| damage.linear_horizontal_speed),
            };
            let notes = walkers.bots.fixed(place, fix, world.as_deref());
            if let Some(bot) = walkers.bots.bot(place)
                && let Some(error) = bot.track().last_error()
            {
                log::debug!(
                    "Match {id:#x}: player {place} (the server's bot) in the world at {} ms: {error:.2} m from its reckoning",
                    snapshot.time
                );
            }
            for note in notes {
                log::info!(
                    "Match {id:#x}: player {} (the server's bot) at {} ms {}",
                    note.place,
                    note.time,
                    note.text
                );
            }
        }
    }

    /// Enter the server's bots once somebody simulates, and relay the
    /// inputs they have due.
    pub(super) fn poll_driven(&mut self, now: Instant) {
        if self.clock_start.is_none()
            || self.finished_at.is_some()
            || self.ending.is_some()
            || self.mode_end_at.is_some()
        {
            return;
        }
        let clock = self.clock(now);
        let mut due = Vec::new();
        for index in 0..self.drives.len() {
            if self.drives[index].is_none() {
                continue;
            }
            let entered = self
                .events
                .iter()
                .any(|(_, who, entered)| *who == index && *entered);
            if !entered {
                if self.live() == 0 {
                    continue;
                }
                let at = grid_up(clock + ROSTER_LEAD_MS);
                self.roster_change(None, index, true, at);
                if let Some(drive) = self.drives[index].as_mut() {
                    drive.entered(at);
                }
                if let Some(walkers) = self.walkers.as_mut() {
                    walkers.bots.entered(index, at);
                }
                log::info!(
                    "Match {:#x}: player {index} (the server's bot) entered at {at} ms",
                    self.id
                );
                continue;
            }
            if self.walkers.as_ref().is_some_and(|walkers| !walkers.absent) {
                continue;
            }
            if let Some(drive) = self.drives[index].as_mut() {
                for sent in drive.due(clock) {
                    if let Some(part) = sent.what {
                        log::debug!(
                            "Match {:#x}: player {index} (the server's bot) at {} ms: {part}",
                            self.id,
                            sent.time
                        );
                    }
                    due.push((index, sent.time, sent.input));
                }
            }
        }
        let id = self.id;
        if let Some(walkers) = self.walkers.as_mut() {
            let world = walkers.world(id);
            if !walkers.absent {
                let (made, notes) = walkers.bots.poll(clock, world.as_deref());
                for note in notes {
                    log::info!(
                        "Match {id:#x}: player {} (the server's bot) at {} ms {}",
                        note.place,
                        note.time,
                        note.text
                    );
                }
                due.extend(
                    made.into_iter()
                        .map(|made| (made.place, made.time, made.input)),
                );
            }
        }
        for (index, time, input) in due {
            let what = Relayed::Input(input);
            self.relay_to_others(index, &what, time);
            self.inputs.push((time, index, what));
        }
    }
}
