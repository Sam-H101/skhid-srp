//! The match server's bots: stages 2 and 3 of the player bots
//! (`docs/notes/player_bots.md` §6), built to the design of
//! `docs/notes/player_bots_research.md` part B:
//!
//! ```text
//! view    -> what the match is: every body, sound, death, battery  (view.rs)
//! sense   -> what one bot sees and hears                           (sense.rs, cheats.rs)
//! know    -> what it and its team know, each fact with its source  (know.rs)
//! team    -> the commander hands out plays every ~0.5 s             (team.rs, signals.rs)
//! play    -> one play at a time, a small state machine              (plays/)
//! tactics -> spots and routes; hazards and spawn areas kept off     (tactics.rs, hazards.rs, path.rs)
//! motor   -> aim, move, fire, smooth as a player's hands           (motor.rs, reflex.rs, stuck.rs)
//! guns    -> each weapon's rounds, shots and reloads, from the game (guns.rs)
//! recoil  -> its kick, which moves the view, as the client moves it  (recoil.rs)
//! encode  -> the inputs a client would send                         (encode.rs)
//! ```
//!
//! Orders go down, reports come up. Two tracks per bot: the play (where to
//! be and why) and the combat reflex (target, aim, fire), which overrides
//! only the look and the trigger. Every number is the tuning's
//! (`tuning.rs`, `SRP_BOT_TUNING`); each bot's random numbers are its own
//! (`rng.rs`); every decision can be traced (`trace.rs`,
//! `SRP_BOT_TRACE_DIR`).
//!
//! [`Bots`] holds a match's bots and is all the match (`game/driven.rs`)
//! sees of them: entries, every player's inputs, the trusted worlds' word
//! on bodies and statistics, the map once its grid is built; and the
//! inputs the bots make.

pub mod aimpoint;
pub mod anomalies;
pub mod body;
pub mod cheats;
pub mod curve;
pub mod encode;
pub mod guns;
pub mod hazards;
pub mod kin;
pub mod know;
pub mod maps;
pub mod motor;
pub mod path;
pub mod plays;
pub mod recoil;
pub mod reflex;
pub mod rng;
pub mod routes;
pub mod sense;
pub mod signals;
pub mod stuck;
pub mod tactics;
pub mod team;
pub mod trace;
pub mod tuning;
pub mod view;

use std::sync::Arc;

use mapdata::{Vec3, World};
use survarium::v026g::match_msgs::PlayerInput;

use crate::v026g::drive::{half_value, key};
use body::Fix;
use cheats::CheatClock;
use encode::{Decision, Encoder};
use know::{DangerSpot, Memory, Percept, TeamKnowledge, Theft};
use motor::{Motor, Pose, Walked};
use path::Route;
use plays::{BotFacts, Ctx, Go, Intent, Look, Pace, PlayKind, Running, Step};
use reflex::Reflex;
use rng::Rng;
use sense::RayBudget;
use signals::{Duel, MatchSignals, Mods, Skill};
use stuck::{Recovery, Stuck};
use tactics::Tactics;
use team::{Asking, Assignment};
use trace::Trace;
use tuning::Tuning;
use view::{BatteryState, Clock, MatchView, Place, PlaceView, SoundKind, Tracker, WorldFacts};

/// From a bot's entry to its first move, ms.
const START_AFTER_MS: u32 = 2_000;
/// A body farther than this off its route plans again, metres.
const OFF_ROUTE_M: f32 = 3.0;
/// A goal that moved this much is planned to again, metres.
const REPLAN_MOVED_M: f32 = 2.0;
/// A world that puts the body this far from its reckoning plans again.
const REPLAN_ERROR_M: f32 = 2.0;
/// The reflex's side step is taken only with a clear walk this far that
/// side, metres (no step off a ledge or into a wall).
const STRAFE_CLEAR_M: f32 = 1.5;
/// A goal no way was found to is not planned to again for this long, ms.
const NO_WAY_RETRY_MS: u32 = 500;
/// How long the plays leave a goal no way was found to (`Bot::no_way_to`):
/// a zone on a timer may open meanwhile, and the bot does something else.
pub const NO_WAY_KEEP_MS: u32 = 20_000;
/// Grid searches a poll may make for all the match's bots: a search costs
/// up to tens of ms on the host, and the match loop is the server's one
/// thread. A bot left without one plans at a later poll (5 ms on).
const SEARCHES_PER_POLL: u32 = 2;
/// A death this recent is a fight where it fell, for the choice of the bot
/// that gives way to a late joiner ([`Bots::to_give_way`]), ms.
const FIGHT_DEATH_MS: u32 = 10_000;

/// An input a bot made.
#[derive(Clone, Debug)]
pub struct Made {
    pub place: usize,
    /// Its tick's time on the match clock.
    pub time: u32,
    pub input: PlayerInput,
}

/// Something worth a line in the log.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub place: usize,
    pub time: u32,
    pub text: String,
}

/// A team's shared state: knowledge, modifiers, skill, the commander's
/// clock.
struct TeamState {
    knowledge: TeamKnowledge,
    mods: Mods,
    skill: Skill,
    cheats: CheatClock,
    signals: MatchSignals,
    next_commander: u32,
}

/// One bot.
pub struct Bot {
    place: Place,
    team: u8,
    rng: Rng,
    encoder: Encoder,
    motor: Motor,
    reflex: Reflex,
    memory: Memory,
    stuck: Stuck,
    personality: usize,
    /// The skill it took at its last spawn.
    skill: f32,
    /// Its map side (the spawns' team it first came out of).
    side: Option<i32>,
    running: Option<Running>,
    assignment: Option<Assignment>,
    last_ran: Vec<(PlayKind, u32)>,
    intent: Intent,
    /// The route and the goal it was planned to.
    route: Option<(Route, Vec3)>,
    /// A goal no way was found to, and until when it is not tried again.
    no_way: Option<(Vec3, u32)>,
    /// Goals a search from its own floor proved no way to (over the floors
    /// it may walk on then), and until when the plays leave them
    /// ([`NO_WAY_KEEP_MS`]): a battery ringed by fire is not fetched again
    /// and again, standing.
    no_way_to: Vec<(Vec3, u32)>,
    /// Keys to press for one tick.
    press: u32,
    /// Keys held for a few ticks to get unstuck.
    unstick: Option<(u32, u32)>,
    /// Standing still until, after giving a spot up.
    pause_until: u32,
    next_think: u32,
    /// Sounds up to this time were heard already.
    heard_until: u32,
    entered: Option<u32>,
    last_trace: u32,
    partner: Option<Place>,
    /// Medkits used this life.
    medkits_used: u8,
}

impl Bot {
    fn new(match_id: u32, place: Place, team: u8, role: Option<&str>, tuning: &Tuning) -> Self {
        let mut rng = Rng::for_bot(match_id, place);
        let motor = Motor::new(tuning.feel.clone(), &mut rng);
        // The personality of its role (play 14: the lobby gave it that
        // role's gear, `survarium::v026g::loadouts::BotRole`), else one
        // drawn by the shares.
        let mut pick = rng.unit();
        let personality = role
            .and_then(|role| tuning.personalities.iter().position(|p| p.name == role))
            .unwrap_or_else(|| {
                tuning
                    .personalities
                    .iter()
                    .position(|p| {
                        pick -= p.share;
                        pick < 0.0
                    })
                    .unwrap_or(0)
            });
        // What it carries: its role's gear, else its personality's name as
        // a role (the tests' bots), else the fallback.
        let arms = role.and_then(guns::Arms::of_role).or_else(|| {
            tuning
                .personalities
                .get(personality)
                .and_then(|p| guns::Arms::of_role(&p.name))
        });
        Self {
            place,
            team,
            rng,
            encoder: Encoder::default(),
            motor,
            reflex: Reflex::armed(arms),
            memory: Memory::default(),
            stuck: Stuck::default(),
            personality,
            skill: tuning.skill.start,
            side: None,
            running: None,
            assignment: None,
            last_ran: Vec::new(),
            intent: Intent::stay(),
            route: None,
            no_way: None,
            no_way_to: Vec::new(),
            press: 0,
            unstick: None,
            pause_until: 0,
            next_think: 0,
            heard_until: 0,
            entered: None,
            last_trace: 0,
            partner: None,
            medkits_used: 0,
        }
    }

    fn note(&self, notes: &mut Vec<Note>, time: u32, text: String) {
        notes.push(Note {
            place: self.place,
            time,
            text,
        });
    }

    fn personality<'t>(&self, tuning: &'t Tuning) -> &'t tuning::Personality {
        &tuning.personalities[self.personality.min(tuning.personalities.len() - 1)]
    }

    /// Its play ended: remembered, and the commander asked again.
    fn end_play(&mut self, now: u32, why: &str, notes: &mut Vec<Note>) {
        if let Some(running) = self.running.take() {
            let kind = running.kind();
            self.last_ran.retain(|(k, _)| *k != kind);
            self.last_ran.push((kind, now));
            self.note(notes, now, format!("ends {kind:?}: {why}"));
        }
        self.assignment = None;
        self.route = None;
        self.intent = Intent::stay();
    }

    /// A new life: a fresh start with the team's present skill (it changes
    /// only at a spawn, play 23). The radio needs no key: it shortens the
    /// respawn by being carried (owner's word; being checked in the
    /// client's code).
    fn new_life(&mut self, skill: f32, tactics: Option<&Tactics>, feet: Vec3) {
        self.running = None;
        self.assignment = None;
        self.route = None;
        self.intent = Intent::stay();
        self.stuck.reset();
        self.unstick = None;
        self.reflex = Reflex::armed(self.reflex.arms());
        self.memory = Memory::default();
        self.skill = skill;
        self.medkits_used = 0;
        if self.side.is_none() {
            self.side = tactics.and_then(|tactics| tactics.side_at(feet));
        }
    }
}

/// A match's bots.
pub struct Bots {
    level: u8,
    /// The mode's id (2 batteries, 3 timed batteries, 4 TDM).
    mode: u8,
    /// The static info's seed (`registry::static_id`): every anomaly's
    /// random stream starts from it (`anomalies.rs`).
    static_seed: u32,
    pub tuning: Arc<Tuning>,
    tracker: Tracker,
    tactics: Option<Tactics>,
    teams: [TeamState; 2],
    bots: Vec<Option<Bot>>,
    trace: Trace,
    /// Every place's weapon 1 kind, where known (play 11).
    weapons: Vec<Option<reflex::WeaponKind>>,
    /// Votes on the frame of a lying battery's position in the worlds: as
    /// written, or with z mirrored (physics); settled against the map's
    /// battery places.
    battery_votes: (u32, u32),
    /// The human routes of this map and mode (play 17), till the map is
    /// built and they are put on its floors.
    routes: Vec<routes::HumanRoute>,
    /// Grid searches left to this poll ([`SEARCHES_PER_POLL`]).
    searches_left: u32,
}

impl Bots {
    /// `teams[place]`, `driven[place]` (a bot's place), `roles[place]` (a
    /// bot's role by its gear); `mode` and `level` the map's.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        match_id: u32,
        mode: u8,
        level: u8,
        teams: &[u8],
        driven: &[bool],
        roles: &[Option<&str>],
        tuning: Arc<Tuning>,
        trace: Trace,
    ) -> Self {
        let team_state = |tuning: &Tuning| TeamState {
            knowledge: TeamKnowledge::default(),
            mods: Mods::default(),
            skill: Skill::new(&tuning.skill),
            cheats: CheatClock::default(),
            signals: MatchSignals::default(),
            next_commander: 0,
        };
        let bots = teams
            .iter()
            .zip(driven)
            .enumerate()
            .map(|(place, (&team, &driven))| {
                let role = roles.get(place).copied().flatten();
                driven.then(|| Bot::new(match_id, place, team, role, &tuning))
            })
            .collect();
        Self {
            level,
            mode,
            static_seed: crate::v026g::registry::static_id(match_id),
            tracker: Tracker::new(mode, teams, driven, Vec::new()),
            tactics: None,
            teams: [team_state(&tuning), team_state(&tuning)],
            bots,
            trace,
            tuning,
            battery_votes: (0, 0),
            routes: Vec::new(),
            weapons: Vec::new(),
            searches_left: SEARCHES_PER_POLL,
        }
    }

    /// Every place's weapon 1 kind (the matchup, play 11).
    pub fn set_weapons(&mut self, weapons: Vec<Option<reflex::WeaponKind>>) {
        self.weapons = weapons;
    }

    /// A lying battery's position from a world, in the map's frame: its
    /// frame is settled by which reading lies nearer the map's battery
    /// places (the worlds' frame is LIKELY the physics', z mirrored).
    fn battery_frame(&mut self, raw: Vec3) -> Vec3 {
        let mirrored = Vec3::new(raw.x, raw.y, -raw.z);
        if let Some(tactics) = self.tactics.as_ref() {
            let nearest = |p: Vec3| {
                tactics
                    .world
                    .map
                    .battery_spawners
                    .iter()
                    .map(|s| s.item.distance_xz(p))
                    .fold(f32::INFINITY, f32::min)
            };
            let (a, b) = (nearest(raw), nearest(mirrored));
            if a < 3.0 && b > 10.0 {
                self.battery_votes.0 += 1;
            } else if b < 3.0 && a > 10.0 {
                self.battery_votes.1 += 1;
            }
        }
        if self.battery_votes.0 > self.battery_votes.1 {
            raw
        } else {
            mirrored
        }
    }

    pub fn tracker(&self) -> &Tracker {
        &self.tracker
    }

    pub fn tactics(&self) -> Option<&Tactics> {
        self.tactics.as_ref()
    }

    /// The map's walk grid is built: the tactics (hazards, exits, the
    /// regions) and the containers. 0.15-0.3 s on the big maps on a fast
    /// machine: the match server builds the tactics on a thread of its own
    /// ([`Self::tactics_ready`]). Returns a line for the log.
    pub fn map_ready(&mut self, world: Arc<World>) -> String {
        let tactics = Tactics::new(world, self.tuning.hazard_margin);
        self.tactics_ready(tactics)
    }

    /// [`Self::map_ready`] with the tactics built ([`Tactics::new`] with
    /// this match's `tuning.hazard_margin`): the routes, the anomalies'
    /// clocks and the containers, a few ms.
    pub fn tactics_ready(&mut self, mut tactics: Tactics) -> String {
        tactics.set_routes(std::mem::take(&mut self.routes));
        // The anomalies that switch (level_03's electric pairs, level_02's
        // cloud) run on the match clock from the seed.
        let anomalies = anomalies::Anomalies::new(&tactics.world.map, self.static_seed);
        tactics.hazards.attach(anomalies);
        let teams = survarium::v026g::match_msgs::rules::container_teams(self.level);
        let mut containers: Vec<(u8, Vec3)> = Vec::new();
        for container in &tactics.world.map.containers {
            let id = container.id.max(0) as usize;
            if containers.len() <= id {
                containers.resize(id + 1, (0, Vec3::ZERO));
            }
            let team = teams.map_or(container.team.clamp(0, 1) as u8, |teams| teams[id.min(1)]);
            containers[id] = (team, container.use_point());
        }
        self.tracker.set_containers(containers);
        let sides: std::collections::BTreeSet<i32> =
            tactics.world.map.spawns.iter().map(|s| s.team).collect();
        let summary = format!(
            "{} damage zones over {} floors; exits by side {:?}; human routes by side {:?}",
            tactics.hazards.zones.len(),
            tactics.hazards.marked(),
            sides
                .iter()
                .map(|side| (*side, tactics.exits(*side).len()))
                .collect::<Vec<_>>(),
            sides
                .iter()
                .map(|side| (*side, tactics.routes_of(*side).count()))
                .collect::<Vec<_>>()
        );
        self.tactics = Some(tactics);
        summary
    }

    /// The human routes of the match's map and mode (play 17), from
    /// `SRP_BOT_ROUTES`; before [`Self::map_ready`].
    pub fn set_routes(&mut self, table: &routes::RouteTable) {
        self.routes = table.for_map(self.level, self.mode);
    }

    pub fn has_map(&self) -> bool {
        self.tactics.is_some()
    }

    /// A place added to the match (a late joiner: never a bot).
    pub fn push_place(&mut self, team: u8) {
        self.bots.push(None);
        self.tracker.push_place(team);
    }

    pub fn is_bot(&self, place: Place) -> bool {
        self.bots.get(place).is_some_and(Option::is_some)
    }

    pub fn any(&self) -> bool {
        self.bots.iter().any(Option::is_some)
    }

    /// The place entered (`time` its 0xE9) or left.
    ///
    /// A bot that left gave its place up to a late joiner (`game/driven.rs`)
    /// and never comes back (its place is not used again): it is forgotten,
    /// so it makes no input, holds no play, is nobody's partner and is in
    /// no commander's count. Whoever leaves, a bot's memory of it and its
    /// team's callouts about it go: nobody is to be hunted where it stood.
    pub fn entered(&mut self, place: Place, entered: bool, time: u32) {
        self.tracker.entered(place, entered);
        if entered {
            if let Some(Some(bot)) = self.bots.get_mut(place) {
                bot.entered = Some(time);
            }
            return;
        }
        if let Some(slot) = self.bots.get_mut(place)
            && let Some(bot) = slot.take()
        {
            self.trace.line(serde_json::json!({
                "t": time, "bot": place, "team": bot.team, "left": true,
            }));
        }
        for bot in self.bots.iter_mut().flatten() {
            bot.memory.forget(place);
            if bot.partner == Some(place) {
                bot.partner = None;
            }
        }
        for team in &mut self.teams {
            team.knowledge.forget(place);
        }
    }

    /// Which of `candidates` (bots of one side, in the match) gives its
    /// place up to a human who joined late (`game/driven.rs`, "Giving
    /// way"), with the clock at `clock`: one dead or waiting to see its
    /// body first, else the living one farthest from any fight (an enemy,
    /// a shot heard, a death of the last [`FIGHT_DEATH_MS`]); never one
    /// carrying a battery as the view has it, and never a living one of a
    /// side that can no longer respawn (Team Deathmatch out of tickets: it
    /// could be its side's last life, and the mode's rule ends the match
    /// when that goes, `rules::tdm_finished`). `None`: none may go now.
    pub fn to_give_way(&self, candidates: &[Place], clock: u32) -> Option<Place> {
        let world = self.tactics.as_ref().map(|t| t.world.clone());
        let at = Clock {
            clock,
            time_left_s: 0.0,
            total_s: 0.0,
        };
        let view = self.tracker.view(&at, &self.tuning.kin, world.as_deref());
        candidates
            .iter()
            .filter_map(|&place| {
                let me = view.place(place)?;
                if me.carrying.is_some()
                    || (me.alive && !view.can_respawn[usize::from(me.team & 1)])
                {
                    return None;
                }
                let away = if me.alive {
                    from_fight(&view, me)
                } else {
                    f32::INFINITY
                };
                Some((!me.alive, away, place))
            })
            .max_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)))
            .map(|(_, _, place)| place)
    }

    pub fn has_entered(&self, place: Place) -> bool {
        self.bots
            .get(place)
            .and_then(Option::as_ref)
            .is_some_and(|bot| bot.entered.is_some())
    }

    /// Whether a bot waits to see its body in a world (just entered, or
    /// dead).
    pub fn need_world(&self) -> bool {
        self.bots.iter().flatten().any(|bot| {
            bot.entered.is_some()
                && self
                    .tracker
                    .body(bot.place)
                    .is_none_or(|body| body.estimate().is_none())
        })
    }

    /// A human's input as relayed.
    pub fn input(&mut self, place: Place, time: u32, input: &PlayerInput) {
        if self.is_bot(place) {
            return;
        }
        let r = input.rotation;
        let turn = (
            half_value(u16::from_le_bytes([r[0], r[1]])),
            half_value(u16::from_le_bytes([r[2], r[3]])),
        );
        let world = self.tactics.as_ref().map(|t| t.world.clone());
        // A human's kick is not known.
        self.tracker.input(
            place,
            time,
            input.actions,
            turn,
            0.0,
            &self.tuning.kin,
            world.as_deref(),
        );
    }

    /// A trusted world's word on one place's body.
    pub fn fixed(&mut self, place: Place, fix: Fix, health: Option<f32>) -> Vec<Note> {
        let mut notes = Vec::new();
        let (time, feet) = (fix.time, fix.feet);
        let world = self.tactics.as_ref().map(|t| t.world.clone());
        let new_life = self
            .tracker
            .fixed(place, fix, health, &self.tuning.kin, world.as_deref());
        let Some(Some(bot)) = self.bots.get_mut(place) else {
            return notes;
        };
        let team = usize::from(bot.team & 1);
        if new_life && let Some(feet) = feet {
            bot.new_life(self.teams[team].skill.skill, self.tactics.as_ref(), feet);
            bot.note(
                &mut notes,
                time,
                format!("is alive at {:?} (side {:?})", feet.to_array(), bot.side),
            );
            return notes;
        }
        let (Some(feet), Some((_, goal))) = (feet, bot.route.as_ref()) else {
            return notes;
        };
        let goal = *goal;
        if let Some(recovery) = bot.stuck.walking(time, feet) {
            bot.note(
                &mut notes,
                time,
                format!("is stuck at {:?}: {recovery:?}", feet.to_array()),
            );
            let side = if bot.rng.unit() < 0.5 {
                key::LEFT
            } else {
                key::RIGHT
            };
            let ticks = |rng: &mut Rng, s: (f32, f32)| (rng.range(s.0, s.1) / kin::TICK_S) as u32;
            match recovery {
                Recovery::BackOff => {
                    bot.unstick = Some((
                        key::BACK | side | key::JUMP,
                        ticks(&mut bot.rng, (0.5, 0.8)),
                    ));
                    bot.route = None;
                }
                Recovery::Sidestep => {
                    bot.unstick = Some((side | key::JUMP, ticks(&mut bot.rng, (0.8, 1.3))));
                    bot.end_play(time, "stuck twice there", &mut notes);
                    self.teams[team].next_commander = time;
                }
                Recovery::GiveUp => {
                    bot.end_play(time, "stuck there again", &mut notes);
                    bot.pause_until = time + 3_000;
                }
            }
            return notes;
        }
        let error = self
            .tracker
            .body(place)
            .and_then(|body| body.last_error())
            .unwrap_or(0.0);
        if error > REPLAN_ERROR_M {
            bot.note(
                &mut notes,
                time,
                format!(
                    "was {error:.1} m off its reckoning; plans again to {:?}",
                    goal.to_array()
                ),
            );
            bot.route = None;
        }
        notes
    }

    /// A trusted world's objects section: the anomalies' states put right.
    /// Returns (cores read, cores missed).
    pub fn anomalies_from_world(&mut self, objects: &[u8], time: u32) -> (usize, usize) {
        self.tactics.as_mut().map_or((0, 0), |tactics| {
            let report = tactics.hazards.sync_from_world(objects, time);
            (report.synced.len(), report.missed.len())
        })
    }

    /// The rest of a trusted world: deaths become danger spots and duels,
    /// a battery of ours carried off is a theft.
    pub fn facts(&mut self, facts: WorldFacts) -> Vec<Note> {
        let mut notes = Vec::new();
        let time = facts.time;
        let mut facts = facts;
        for battery in &mut facts.batteries {
            if let BatteryState::Free(raw) = battery {
                *battery = BatteryState::Free(self.battery_frame(*raw));
            }
        }
        let at = |clock| Clock {
            clock,
            time_left_s: 0.0,
            total_s: 0.0,
        };
        let before = self.tracker.view(&at(time), &self.tuning.kin, None);
        self.tracker.facts(facts);
        let after = self.tracker.view(&at(time), &self.tuning.kin, None);
        let kills_by = |bot: bool| {
            after
                .places
                .iter()
                .zip(&before.places)
                .any(|(a, b)| a.bot == bot && a.kills > b.kills)
        };
        let (human_kills, bot_kills) = (kills_by(false), kills_by(true));
        for (a, b) in after.places.iter().zip(&before.places) {
            if a.deaths <= b.deaths {
                continue;
            }
            let team = usize::from(a.team & 1);
            // A body never placed (feet at the origin) gives no spot.
            if b.feet != Vec3::ZERO {
                self.teams[team].knowledge.danger.push(DangerSpot {
                    at: b.feet,
                    time,
                    victim: a.place,
                });
            }
            // Self-balancing counts duels between bots and humans.
            if a.bot && human_kills {
                self.teams[team].skill.duel(Duel {
                    time,
                    bot_won: false,
                });
            }
            if !a.bot && bot_kills {
                self.teams[1 - team].skill.duel(Duel {
                    time,
                    bot_won: true,
                });
            }
            for bot in self.bots.iter_mut().flatten() {
                bot.memory.forget(a.place);
            }
        }
        // A battery of ours taken from home.
        for (battery, (now, then)) in after.batteries.iter().zip(&before.batteries).enumerate() {
            if let (BatteryState::Carried(thief), BatteryState::Home(id)) = (now, then)
                && let Some(container) = after.containers.iter().find(|c| c.id == *id)
            {
                let team = usize::from(container.team & 1);
                self.teams[team].knowledge.thefts.push(Theft {
                    battery,
                    at: container.at,
                    time,
                    thief: *thief,
                });
                notes.push(Note {
                    place: usize::MAX,
                    time,
                    text: format!("team {team}'s battery {battery} is stolen"),
                });
            }
        }
        notes
    }

    /// The inputs due with the clock at `clock`.
    pub fn poll(&mut self, clock: Clock) -> (Vec<Made>, Vec<Note>) {
        let (mut made, mut notes) = (Vec::new(), Vec::new());
        let now = clock.clock;
        let world = self.tactics.as_ref().map(|t| t.world.clone());
        self.tracker.held_sounds(now);
        if let Some(tactics) = self.tactics.as_mut() {
            tactics.hazards.advance(now);
        }
        let view = self
            .tracker
            .view(&clock, &self.tuning.kin, world.as_deref());
        if self.tactics.is_some() {
            for team in 0..2u8 {
                if now >= self.teams[usize::from(team)].next_commander {
                    self.commander(team, &view, &mut notes);
                }
            }
        }
        let mut rays = RayBudget(self.tuning.think.rays_per_poll);
        self.searches_left = SEARCHES_PER_POLL;
        for index in 0..self.bots.len() {
            if self.bots[index].is_none() {
                continue;
            }
            self.think(index, &view, &mut rays, &mut notes);
            self.ticks(index, &clock, &view, &mut made);
        }
        (made, notes)
    }

    /// A team's commander tick: signals, modifiers, skill, and a play for
    /// each living bot.
    fn commander(&mut self, team: u8, view: &MatchView, notes: &mut Vec<Note>) {
        let Some(tactics) = self.tactics.as_ref() else {
            return;
        };
        let tuning = self.tuning.clone();
        let now = view.clock;
        let state = &mut self.teams[usize::from(team)];
        state.next_commander = now + tuning.think.commander_ms;
        state.knowledge.forget_old(now);
        let since_theft = state
            .knowledge
            .thefts
            .iter()
            .map(|t| now.saturating_sub(t.time) as f32 / 1000.0)
            .fold(1e6, f32::min);
        state.signals = MatchSignals::read(view, team, since_theft);
        state.mods.update(&state.signals, &tuning.mods);
        // A team that can no longer respawn (Team Deathmatch, its tickets
        // out) plays it safe.
        if !view.can_respawn[usize::from(team & 1)] {
            state.mods.aggression = state.mods.aggression.min(0.2);
        }
        if let Some((before, after, kd)) = state.skill.balance(now, &tuning.skill) {
            notes.push(Note {
                place: usize::MAX,
                time: now,
                text: format!(
                    "team {team}'s skill {before:.2} -> {after:.2} (K/D against humans {kd:.2})"
                ),
            });
        }
        // Partners for play 21: the team's bots two by two.
        let mates: Vec<Place> = self
            .bots
            .iter()
            .flatten()
            .filter(|b| b.team == team)
            .map(|b| b.place)
            .collect();
        for pair in mates.chunks(2) {
            if let [a, b] = pair {
                for (me, other) in [(*a, *b), (*b, *a)] {
                    if let Some(Some(bot)) = self.bots.get_mut(me) {
                        bot.partner = Some(other);
                    }
                }
            }
        }
        let state = &self.teams[usize::from(team)];
        let mut asked: Vec<(Place, Vec<plays::Candidate>, Option<Assignment>)> = Vec::new();
        for bot in self
            .bots
            .iter()
            .flatten()
            .filter(|b| b.team == team && b.entered.is_some())
        {
            let Some(me) = view.place(bot.place).filter(|p| p.alive) else {
                continue;
            };
            let cx = Ctx {
                now,
                view,
                me,
                bot: BotFacts {
                    personality: bot.personality(&tuning),
                    side: bot.side,
                    current: bot.assignment.as_ref().map(|a| (a.kind, a.target, a.since)),
                    last_ran: &bot.last_ran,
                    partner: bot.partner,
                    medkits_used: bot.medkits_used,
                    no_way_to: &bot.no_way_to,
                },
                memory: &bot.memory,
                team: &state.knowledge,
                sig: &state.signals,
                mods: &state.mods,
                tactics,
                tuning: &tuning,
            };
            let mut candidates = Vec::new();
            plays::candidates(&cx, &mut candidates);
            asked.push((bot.place, candidates, bot.assignment.clone()));
        }
        let asking = asked
            .iter()
            .map(|(place, candidates, current)| Asking {
                place: *place,
                candidates: candidates.clone(),
                current: current.as_ref(),
            })
            .collect();
        for (place, assignment, changed) in team::assign(asking, now) {
            let Some(me) = view.place(place) else {
                continue;
            };
            let Some(Some(bot)) = self.bots.get_mut(place) else {
                continue;
            };
            if !changed {
                bot.assignment = Some(assignment);
                continue;
            }
            // The new play starts.
            if let Some(running) = bot.running.take() {
                let kind = running.kind();
                bot.last_ran.retain(|(k, _)| *k != kind);
                bot.last_ran.push((kind, now));
            }
            let cx = Ctx {
                now,
                view,
                me,
                bot: BotFacts {
                    personality: bot.personality(&tuning),
                    side: bot.side,
                    current: Some((assignment.kind, assignment.target, now)),
                    last_ran: &bot.last_ran,
                    partner: bot.partner,
                    medkits_used: bot.medkits_used,
                    no_way_to: &bot.no_way_to,
                },
                memory: &bot.memory,
                team: &state.knowledge,
                sig: &state.signals,
                mods: &state.mods,
                tactics,
                tuning: &tuning,
            };
            let mut rng = bot.rng.clone();
            let running = plays::start(assignment.kind, assignment.target, &cx, &mut rng);
            bot.rng = rng;
            bot.note(
                notes,
                now,
                format!(
                    "takes {:?} {:?} (score {:.2}: {})",
                    assignment.kind, assignment.target, assignment.score, assignment.reason
                ),
            );
            if self.trace.enabled() {
                let mut top = asked
                    .iter()
                    .find(|(p, _, _)| *p == place)
                    .map(|(_, c, _)| c.clone())
                    .unwrap_or_default();
                top.sort_by(|a, b| {
                    b.rank
                        .cmp(&a.rank)
                        .then(b.score.get().total_cmp(&a.score.get()))
                });
                let top: Vec<serde_json::Value> = top
                    .into_iter()
                    .take(5)
                    .map(|c| {
                        serde_json::json!({
                            "play": format!("{:?}", c.kind),
                            "target": format!("{:?}", c.target),
                            "rank": format!("{:?}", c.rank),
                            "score": c.score.get(),
                            "parts": c.why.parts.iter()
                                .map(|(i, v, s)| serde_json::json!([format!("{i:?}"), v, s]))
                                .collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                self.trace.line(serde_json::json!({
                    "t": now, "bot": place, "team": team,
                    "pos": me.feet.to_array(), "health": me.health,
                    "takes": format!("{:?}", assignment.kind),
                    "reason": assignment.reason,
                    "top": top,
                    "mods": [state.mods.aggression, state.mods.risk, state.mods.objective_urgency],
                    "skill": state.skill.skill,
                }));
            }
            bot.running = Some(running);
            bot.route = None;
            bot.assignment = Some(assignment);
        }
    }

    /// A bot's think, when due: senses, memory, reflex, the play's step.
    fn think(
        &mut self,
        index: usize,
        view: &MatchView,
        rays: &mut RayBudget,
        notes: &mut Vec<Note>,
    ) {
        let Some(tactics) = self.tactics.as_ref() else {
            return;
        };
        let tuning = self.tuning.clone();
        let now = view.clock;
        let Some(bot) = self.bots[index].as_mut() else {
            return;
        };
        if bot.entered.is_none() || now < bot.next_think {
            return;
        }
        bot.next_think = now
            + bot.rng.range(
                tuning.think.think_ms.0 as f32,
                tuning.think.think_ms.1 as f32,
            ) as u32;
        let Some(me) = view.place(bot.place).filter(|p| p.alive) else {
            return;
        };
        let team = usize::from(bot.team & 1);
        // Senses: sight (the order turns each think, so no enemy waits
        // long for a ray), hearing, the cheats, the team's callouts.
        let mut order: Vec<Place> = view.enemies_of(bot.team).map(|p| p.place).collect();
        if !order.is_empty() {
            let k = (now / 100) as usize % order.len();
            order.rotate_left(k);
        }
        let mut percepts = sense::sense(
            me,
            view,
            &tactics.world.sight,
            &tuning.sense,
            &tuning.cheats,
            bot.heard_until,
            &order,
            rays,
            &mut bot.rng,
        );
        bot.heard_until = now;
        let quiet = percepts.is_empty() && bot.memory.all_seen(now).is_empty();
        let state = &mut self.teams[team];
        for percept in &percepts {
            state
                .knowledge
                .call_out(bot.place, percept, tuning.sense.callout_ms);
        }
        percepts.extend(cheats::cheat_percepts(
            bot.team,
            view,
            &tuning.cheats,
            &mut state.cheats,
            me.feet,
            quiet,
            &mut bot.rng,
        ));
        percepts.extend(state.knowledge.heard_by(bot.place, now));
        let skill = bot.skill;
        let notice_factor = signals::mix(tuning.aim.notice, skill);
        let feet = me.feet;
        bot.memory.update(
            &percepts,
            now,
            |p: &Percept| {
                sense::notice_ms(&tuning.sense, p.at.distance_xz(feet), p.busy, notice_factor)
            },
            tuning.cheats.forget_ms.max(tuning.sense.forget_ms),
            tuning.sense.drop_ms,
        );
        let sit = reflex::Situation {
            view,
            tactics: Some(tactics),
            team: Some(&self.teams[team].knowledge),
            side: bot.side,
            partner: bot.partner,
            weapon: reflex::WeaponKind::of_role(&bot.personality(&tuning).name),
            weapons: &self.weapons,
        };
        bot.reflex
            .think_with(me, &sit, &bot.memory, now, &tuning.aim, skill, &mut bot.rng);
        if now < bot.pause_until {
            bot.intent = Intent::stay();
            return;
        }
        // The play's step.
        let state = &self.teams[team];
        let Some(mut running) = bot.running.take() else {
            bot.intent = Intent::stay();
            return;
        };
        let target = bot
            .assignment
            .as_ref()
            .map_or(plays::TargetRef::None, |a| a.target);
        let cx = Ctx {
            now,
            view,
            me,
            bot: BotFacts {
                personality: bot.personality(&tuning),
                side: bot.side,
                current: bot.assignment.as_ref().map(|a| (a.kind, a.target, a.since)),
                last_ran: &bot.last_ran,
                partner: bot.partner,
                medkits_used: bot.medkits_used,
                no_way_to: &bot.no_way_to,
            },
            memory: &bot.memory,
            team: &state.knowledge,
            sig: &state.signals,
            mods: &state.mods,
            tactics,
            tuning: &tuning,
        };
        let mut rng = bot.rng.clone();
        let step = plays::step(&mut running, target, &cx, &mut rng);
        bot.rng = rng;
        let step_name = running.step_name();
        bot.running = Some(running);
        match step {
            Step::Continue(intent) => {
                bot.press |= intent.press;
                bot.intent = intent;
            }
            Step::Done(why) | Step::Failed(why) => {
                bot.end_play(now, why, notes);
                self.teams[team].next_commander = now;
            }
        }
        if self.trace.enabled() && now >= bot.last_trace + tuning.think.trace_every_ms {
            bot.last_trace = now;
            let line = serde_json::json!({
                "t": now, "bot": bot.place, "pos": me.feet.to_array(), "health": me.health,
                "play": format!("{:?}", bot.assignment.as_ref().map(|a| a.kind)),
                "step": step_name,
                "aim_at": bot.reflex.target(),
                "known": bot.memory.tracks().iter()
                    .map(|t| serde_json::json!([t.who, format!("{:?}", t.source), t.error_m]))
                    .collect::<Vec<_>>(),
            });
            self.trace.line(line);
        }
    }

    /// A bot's ticks due: the motor drives the intent (the reflex
    /// overriding the look and the trigger), the encoder makes inputs.
    fn ticks(&mut self, index: usize, clock: &Clock, view: &MatchView, made: &mut Vec<Made>) {
        let tuning = self.tuning.clone();
        let Some(bot) = self.bots[index].as_mut() else {
            return;
        };
        let Some(entered) = bot.entered else {
            return;
        };
        let world = self.tactics.as_ref().map(|t| t.world.clone());
        let searches = &mut self.searches_left;
        for time in bot.encoder.ticks(clock.clock, entered + START_AFTER_MS) {
            let estimate = self
                .tracker
                .body(bot.place)
                .and_then(|body| body.estimate().copied());
            let me = view.place(bot.place).filter(|p| p.alive);
            let decision = match (estimate, me, self.tactics.as_ref()) {
                (Some(at), Some(me), Some(tactics)) => {
                    // The base pitch: `drive_tick` adds the kick.
                    let me = PlaceView {
                        feet: at.feet,
                        yaw: at.yaw,
                        pitch: at.pitch,
                        velocity: at.velocity,
                        ..me.clone()
                    };
                    drive_tick(bot, &me, view, tactics, &tuning, time, searches)
                }
                _ => bot.motor.rest(),
            };
            let (input, turn) = bot.encoder.encode(time, decision);
            let keys = bot.encoder.keys();
            // [recoil] The part of the pitch turn the kick takes, and the
            // tick's shots: the base pitch moves by the rest.
            let taken = bot.reflex.sent(time, keys, turn.1);
            self.tracker.input(
                bot.place,
                time,
                keys,
                (turn.0, turn.1 + taken),
                bot.reflex.kick(),
                &tuning.kin,
                world.as_deref(),
            );
            if let Some(input) = input {
                made.push(Made {
                    place: bot.place,
                    time,
                    input,
                });
            }
        }
    }
}

/// How far `me` is from any fight, metres: the nearest enemy in the match
/// and alive, shot heard (the view keeps the last few seconds of them) or
/// death of the last [`FIGHT_DEATH_MS`]; infinite with none.
fn from_fight(view: &MatchView, me: &PlaceView) -> f32 {
    let enemies = view.enemies_of(me.team).map(|enemy| enemy.feet);
    let shots = view
        .sounds
        .iter()
        .filter(|sound| sound.kind == SoundKind::Shot)
        .map(|sound| sound.at);
    let deaths = view
        .deaths
        .iter()
        .filter(|death| death.time + FIGHT_DEATH_MS >= view.clock)
        .map(|death| death.at);
    enemies
        .chain(shots)
        .chain(deaths)
        .map(|at| at.distance_xz(me.feet))
        .fold(f32::INFINITY, f32::min)
}

/// One tick of a living bot (`me` with its base pitch): the reflex's look
/// and trigger, the play's legs and stance, the presses.
fn drive_tick(
    bot: &mut Bot,
    me: &PlaceView,
    view: &MatchView,
    tactics: &Tactics,
    tuning: &Tuning,
    time: u32,
    searches: &mut u32,
) -> Decision {
    // [recoil] The weapon's update first, as the client's; the view is the
    // base pitch and the kick the last tick left (the reflex takes back
    // this tick's rise itself, `Override::pull`).
    let kick = bot.reflex.weapon_tick(me, time);
    let me = &PlaceView {
        pitch: (me.pitch + kick).clamp(-1.0, 1.0),
        ..me.clone()
    };
    let intent = bot.intent;
    let pose = Pose {
        feet: me.feet,
        yaw: me.yaw,
        pitch: me.pitch,
        speed: me.speed(),
    };
    // Getting unstuck overrides the rest: the keys for a few ticks, the
    // jump on the first.
    if let Some((keys, ticks)) = bot.unstick {
        bot.unstick = (ticks > 1).then_some((keys & !key::JUMP, ticks - 1));
        bot.motor.turn_time = None;
        let decision = bot.motor.rest();
        return Decision { keys, ..decision };
    }
    // [aimpoint] What of its target shows, before the reflex aims.
    bot.reflex.look(me, view, &tactics.world, time);
    let over = bot.reflex.tick(
        me,
        view,
        &bot.memory,
        time,
        intent.fire,
        &tuning.aim,
        bot.skill,
        &mut bot.rng,
    );
    bot.motor.turn_time = over.turn_time_s;
    let eye = me.eye();
    let aim_at = |p: Vec3| {
        let (dx, dz) = (p.x - eye.x, p.z - eye.z);
        (kin::yaw_of([dx, dz]), (p.y - eye.y).atan2(dx.hypot(dz)))
    };
    let aim = over
        .look_at
        .map(|p| {
            let (yaw, pitch) = aim_at(p);
            (yaw + over.error.0, pitch + over.error.1)
        })
        .or(match intent.look {
            Look::At(p) => Some(aim_at(p + Vec3::new(0.0, 1.0, 0.0))),
            _ => None,
        });
    // The legs: the reflex's when it takes them (a push, breaking sight,
    // a re-peek, the weapon's range, in after a grenade), else the play's.
    let mut intent = intent;
    if let Some(legs) = over.legs {
        intent.go = Go::To {
            at: legs.at,
            within_m: legs.within_m,
        };
        intent.crouch = legs.crouch;
        intent.pace = if legs.run { Pace::Run } else { Pace::Walk };
        intent.objective = false;
    }
    let goal = match intent.go {
        Go::To { at, within_m } if me.feet.distance_xz(at) > within_m => Some(at),
        _ => None,
    };
    // Standing to aim (no way to walk): the reflex's side step may move it.
    let mut aiming_still = false;
    let mut decision = match goal {
        Some(at) => {
            let replan = bot
                .route
                .as_ref()
                .is_none_or(|(_, planned)| planned.distance_xz(at) > REPLAN_MOVED_M);
            // A goal with no way lately: not searched again every tick.
            let no_way = bot.no_way.is_some_and(|(goal, until)| {
                time < until && goal.distance_xz(at) <= REPLAN_MOVED_M
            });
            if replan && no_way {
                bot.route = None;
            } else if replan && *searches == 0 {
                // No search left to this poll: the old route meanwhile.
            } else if replan {
                *searches -= 1;
                let grid = &tactics.world.grid;
                let stuck = &bot.stuck;
                let side = bot.side;
                let objective = intent.objective;
                // Round the spots it got stuck at, if there is a way; else
                // through them (a second search only when there are any).
                let avoiding = stuck.avoids_any(time);
                let ends = grid.locate(me.feet, 1.5).zip(grid.locate(at, 3.0));
                // On a floor it may not walk on (in a fire it stepped
                // into): out to the nearest one it may first; the goal is
                // planned from there.
                let escaping =
                    ends.is_some_and(|(from, _)| !tactics.allowed(from, time, side, objective));
                let route = if escaping {
                    ends.and_then(|(from, _)| tactics.escape(from, time, side, objective))
                } else {
                    ends.and_then(|(from, to)| {
                        tactics.route_nodes(from, to, time, side, objective, |node| {
                            !(avoiding && stuck.near_spot(time, grid.position(node)))
                        })
                    })
                    .or_else(|| {
                        avoiding
                            .then(|| tactics.route_for(me.feet, at, time, side, objective))
                            .flatten()
                    })
                };
                bot.no_way = route.is_none().then_some((at, time + NO_WAY_RETRY_MS));
                // Both ends on floors, from one it may walk on, and no way:
                // proven, for the plays.
                if route.is_none() && ends.is_some() && !escaping {
                    bot.no_way_to.retain(|&(goal, until)| {
                        time < until && goal.distance_xz(at) > REPLAN_MOVED_M
                    });
                    bot.no_way_to.push((at, time + NO_WAY_KEEP_MS));
                }
                bot.route = route.map(|route| (route, at));
            }
            match bot.route.as_mut() {
                Some((route, _)) => {
                    let side = bot.side;
                    let objective = intent.objective;
                    let clear = |p: Vec3| tactics.clear_line_for(me.feet, p, time, side, objective);
                    let (decision, walked) = match aim {
                        Some(aim) => bot.motor.walk_aiming(route, &pose, OFF_ROUTE_M, clear, aim),
                        None => bot.motor.walk(route, &pose, OFF_ROUTE_M, clear),
                    };
                    if walked != Walked::OnTheWay {
                        bot.route = None;
                    }
                    decision
                }
                None => match aim {
                    Some(aim) => {
                        aiming_still = true;
                        bot.motor.aim_still(&pose, aim)
                    }
                    None => bot.motor.stand(&pose, &mut bot.rng),
                },
            }
        }
        None => {
            bot.route = None;
            match aim {
                Some(aim) => {
                    aiming_still = true;
                    bot.motor.aim_still(&pose, aim)
                }
                None => bot.motor.stand(&pose, &mut bot.rng),
            }
        }
    };
    // The side step, where the ground that side is clear for a stride.
    if aiming_still && over.strafe != 0 {
        let left = over.strafe == key::LEFT;
        let [sx, sz] = kin::forward(me.yaw + std::f32::consts::FRAC_PI_2);
        let sign = if left { 1.0 } else { -1.0 };
        let to = me.feet + Vec3::new(sign * sx * STRAFE_CLEAR_M, 0.0, sign * sz * STRAFE_CLEAR_M);
        if tactics.clear_line_for(me.feet, to, time, bot.side, intent.objective) {
            decision.keys |= over.strafe;
        }
    }
    // The stance and pace.
    if intent.crouch || intent.pace == Pace::Sneak {
        decision.keys |= key::CROUCH;
        decision.keys &= !key::SPRINT;
    }
    if intent.pace != Pace::Run {
        decision.keys &= !key::SPRINT;
    }
    decision.turn.1 += over.pull;
    if over.fire {
        decision.keys |= key::FIRE;
        decision.keys &= !key::SPRINT;
    }
    decision.keys |= over.keys;
    if over.keys & (key::GRENADE | key::RELOAD | key::AIM | key::CROUCH) != 0 {
        decision.keys &= !key::SPRINT;
    }
    decision.keys |= intent.hold | std::mem::take(&mut bot.press);
    if decision.keys & key::QUICK[0] != 0 {
        bot.medkits_used = bot.medkits_used.saturating_add(1);
    }
    decision
}

#[cfg(test)]
mod tests;
