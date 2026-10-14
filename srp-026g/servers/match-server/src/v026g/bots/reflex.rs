//! The combat reflex: a bot's second track (`player_bots_research.md` B1
//! point 2, Left 4 Dead's "Main" and "Legs"). The play decides where to be
//! and why; the reflex, at every think, picks a target, aims and fires,
//! turns to a sound, and may take the legs for a moment to survive or to
//! punish (research B1: it "may override movement to survive").
//!
//! The fairness rules hold by type: the reflex aims and fires only at a
//! `know::SeenTrack`, so never at what no sense gave, and never before the
//! noticing delay; the first shot waits a reaction delay more
//! (`AimTuning::reaction_ms`). Its aim is a human's: it starts off by
//! `AimTuning::error_deg` × 3 and settles toward its error over `settle_s`;
//! the error never goes to 0 (a floor drawn every tick: no perfect recoil
//! or spread). Play 23 changes only the numbers that are aim or reaction
//! (`error_deg`, `settle_s`, `notice`, `reaction_ms`), through `skill`;
//! every number of [`CombatTuning`] is the same at every skill.
//!
//! What it does besides aiming and firing (`player_bots_behaviour.md` §3):
//! - **play 5, punish a reload:** an enemy seen within `punish_m` or heard
//!   within `punish_heard_m` reloading or switching weapons during a duel:
//!   the legs go at him for `punish_ms`;
//! - **play 6, turn to a sound:** a shot heard, nothing seen: look there;
//! - **play 7, break sight and re-peek:** hit while in a duel: to a spot
//!   near by hidden from the enemy, wait 1-2 s, come back at a spot that
//!   sees him from another angle, never the one it last fought from;
//! - **play 10's other half:** between seen targets, a wounded one counts
//!   nearer (`wounded_bonus_m`);
//! - **play 11, weapon matchup:** the distance band its weapon wants
//!   against the enemy's ([`preferred_band`]), stepped into while it duels;
//! - **play 19, spot:** a newly noticed enemy is spotted (bit 28, key down
//!   once, the game's 3 s cooldown kept) once it is in the crosshair;
//! - **play 20, grenade before the peek:** rarely, at an enemy last seen
//!   holding an angle or at a place an enemy was sensed staying at, with
//!   grenades left (two a life), the grenade key held until the throw;
//! - **play 21, trade (its reflex half):** the partner died within
//!   `trade_ms`: look at that spot, aiming down the sights.

use mapdata::Vec3;
use serde::{Deserialize, Serialize};

use super::kin::{wrap, yaw_of};
use super::know::{Memory, SeenTrack, Source, TeamKnowledge};
use super::rng::Rng;
use super::signals::mix;
use super::tactics::{SpotQuery, Tactics};
use super::tuning::AimTuning;
use super::view::{MatchView, Place, PlaceView, SoundKind};
use crate::v026g::drive::key;

/// A weapon's kind as the matchup (play 11) reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WeaponKind {
    Pistol,
    Smg,
    Shotgun,
    Rifle,
    Sniper,
}

impl WeaponKind {
    /// A server bot's weapon 1 by its role (`survarium::v026g::loadouts::
    /// BotRole::name`): the Uzi, the AK-74U, the TOZ-34, the Mosin.
    pub fn of_role(role: &str) -> Option<Self> {
        Some(match role {
            "flanker" => Self::Smg,
            "support" => Self::Rifle,
            "rusher" => Self::Shotgun,
            "camper" => Self::Sniper,
            _ => return None,
        })
    }

    /// A weapon item's kind from its dictionary category and its
    /// `item_params` kind (`wpn_*`): category 13 is the sniper category
    /// and 14 the shotguns' (`loadouts.rs` tests); else by the kind (an
    /// SVD is `wpn_aslt` in the sniper category, so the category goes
    /// first).
    pub fn of_item(kind: &str, category: u8) -> Option<Self> {
        Some(match (category, kind) {
            (13, _) => Self::Sniper,
            (14, _) => Self::Shotgun,
            (_, "wpn_dbrl" | "wpn_stgn" | "wpn_asgn") => Self::Shotgun,
            (_, "wpn_bolt") => Self::Sniper,
            (_, "wpn_smg") => Self::Smg,
            (_, "wpn_pstl" | "wpn_rvlr" | "wpn_apst") => Self::Pistol,
            (_, "wpn_aslt") => Self::Rifle,
            _ => return None,
        })
    }
}

/// The reflex's numbers besides the aim (`AimTuning::combat`). None of
/// them is mixed by skill: play 23 changes only aim and reaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CombatTuning {
    /// Play 10: between seen targets, one at no health counts this much
    /// nearer (half health, half of it), metres.
    pub wounded_bonus_m: f32,
    /// A duel lasts this long after the last target or hit, ms.
    pub duel_ms: u32,
    /// Play 5: a reload or switch seen within `punish_m`, or heard within
    /// `punish_heard_m` (the sense's reload range × the hearing cheat),
    /// at most `punish_window_ms` old, sends the legs at the enemy for
    /// `punish_ms`, with this chance.
    pub punish_m: f32,
    pub punish_heard_m: f32,
    pub punish_window_ms: u32,
    pub punish_ms: u32,
    pub punish_chance: f32,
    /// Play 7: a health drop of at least this since the last think is a
    /// hit.
    pub hit_drop: f32,
    /// A hiding spot this far from the bot, metres.
    pub hide_m: (f32, f32),
    /// Waited there, ms (drawn).
    pub hide_wait_ms: (u32, u32),
    /// Not there by this, ms: given up.
    pub hide_give_up_ms: u32,
    /// The re-peek spot this far from the hiding spot, metres.
    pub peek_m: (f32, f32),
    /// Another angle than the last peek's: its bearing from the enemy
    /// differs by `repeek_min_deg`, or it lies `repeek_min_m` from it.
    pub repeek_min_deg: f32,
    pub repeek_min_m: f32,
    /// Going to it at most this long, ms.
    pub peek_ms: u32,
    /// Play 11: the distance band each weapon wants, metres.
    pub band_pistol: (f32, f32),
    pub band_smg: (f32, f32),
    pub band_shotgun: (f32, f32),
    pub band_rifle: (f32, f32),
    pub band_sniper: (f32, f32),
    /// Keep an enemy shotgun at least this far; close on an enemy sniper
    /// to at most this, metres.
    pub keep_from_shotgun_m: f32,
    pub close_on_sniper_m: f32,
    /// Out of the band by more than this, it steps `band_step_m` back in
    /// (only with `band_legs`).
    pub band_slack_m: f32,
    pub band_step_m: f32,
    pub band_legs: bool,
    /// Play 19: the game's spot cooldown, ms; the same enemy is spotted
    /// again only after `spot_again_ms`; spotted once the view is within
    /// `spot_within_deg` of him; with this chance per new enemy.
    pub spot_cooldown_ms: u32,
    pub spot_again_ms: u32,
    pub spot_within_deg: f32,
    pub spot_chance: f32,
    /// Play 20: grenades in the kit per life (`loadouts::BOT_GRENADE`).
    pub grenades: u8,
    /// A throw is weighed this often while nothing is seen, and made with
    /// this chance.
    pub grenade_roll_ms: u32,
    pub grenade_chance: f32,
    /// At a place this far, metres.
    pub grenade_m: (f32, f32),
    /// The key held this long once the view is on the throw, ms (drawn).
    pub grenade_hold_ms: (u32, u32),
    pub grenade_within_deg: f32,
    /// Not with a teammate this near the place, metres.
    pub grenade_friend_m: f32,
    /// The view not on the throw by this, ms: given up.
    pub grenade_give_up_ms: u32,
    /// After a throw, the legs go at the place from/until this, ms.
    pub grenade_push_ms: (u32, u32),
    /// An enemy seen crouched, aiming and slower than this (m/s) holds an
    /// angle; remembered this long, ms.
    pub held_speed: f32,
    pub held_memory_ms: u32,
    /// An enemy sensed this long within `camp_m` (plus the sense's error)
    /// of one place camps there, ms.
    pub camp_ms: u32,
    pub camp_m: f32,
    /// Play 21: the partner's death spot is looked at this long, ms.
    pub trade_ms: u32,
    /// Reloading: after this much fire held since the last reload (about
    /// a clip; drawn per clip), ms, at the first moment with no seen
    /// target or out of sight (breaking sight first when a target is
    /// seen).
    pub reload_fire_ms: (u32, u32),
    /// A tactical reload: no enemy seen for `reload_idle_ms` and at least
    /// `reload_some_fire_ms` of fire since the last reload.
    pub reload_idle_ms: u32,
    pub reload_some_fire_ms: u32,
    /// No shot this long after the reload key, ms (the reload's time).
    pub reload_ms: u32,
}

impl Default for CombatTuning {
    fn default() -> Self {
        Self {
            wounded_bonus_m: 20.0,
            duel_ms: 3_000,
            punish_m: 25.0,
            punish_heard_m: 12.5,
            punish_window_ms: 1_500,
            punish_ms: 1_500,
            punish_chance: 0.8,
            hit_drop: 0.03,
            hide_m: (2.0, 10.0),
            hide_wait_ms: (1_000, 2_000),
            hide_give_up_ms: 4_000,
            peek_m: (2.0, 8.0),
            repeek_min_deg: 15.0,
            repeek_min_m: 4.0,
            peek_ms: 4_000,
            band_pistol: (0.0, 20.0),
            band_smg: (5.0, 25.0),
            band_shotgun: (0.0, 12.0),
            band_rifle: (10.0, 45.0),
            band_sniper: (30.0, 80.0),
            keep_from_shotgun_m: 15.0,
            close_on_sniper_m: 30.0,
            band_slack_m: 3.0,
            band_step_m: 6.0,
            band_legs: true,
            spot_cooldown_ms: 3_000,
            spot_again_ms: 15_000,
            spot_within_deg: 8.0,
            spot_chance: 1.0,
            grenades: 2,
            grenade_roll_ms: 4_000,
            grenade_chance: 0.15,
            grenade_m: (8.0, 30.0),
            grenade_hold_ms: (500, 800),
            grenade_within_deg: 6.0,
            grenade_friend_m: 6.0,
            grenade_give_up_ms: 1_500,
            grenade_push_ms: (1_500, 3_500),
            held_speed: 0.3,
            held_memory_ms: 5_000,
            camp_ms: 8_000,
            camp_m: 3.0,
            trade_ms: 3_000,
            reload_fire_ms: (2_500, 4_000),
            reload_idle_ms: 3_000,
            reload_some_fire_ms: 1_000,
            reload_ms: 2_000,
        }
    }
}

impl CombatTuning {
    pub fn band(&self, kind: WeaponKind) -> (f32, f32) {
        match kind {
            WeaponKind::Pistol => self.band_pistol,
            WeaponKind::Smg => self.band_smg,
            WeaponKind::Shotgun => self.band_shotgun,
            WeaponKind::Rifle => self.band_rifle,
            WeaponKind::Sniper => self.band_sniper,
        }
    }
}

/// Play 11: the distance band to an enemy, metres, by the bot's weapon
/// (`own`) and his (`enemy`): its own weapon's band, then an enemy shotgun
/// kept at `keep_from_shotgun_m` and an enemy sniper closed on to
/// `close_on_sniper_m`, unless that leaves less than 5 m of band (its own
/// weapon wins). No band without its own weapon's kind.
pub fn preferred_band(
    own: Option<WeaponKind>,
    enemy: Option<WeaponKind>,
    tuning: &CombatTuning,
) -> Option<(f32, f32)> {
    let band = tuning.band(own?);
    let adjusted = match enemy {
        Some(WeaponKind::Shotgun) => (band.0.max(tuning.keep_from_shotgun_m), band.1),
        Some(WeaponKind::Sniper) => (band.0, band.1.min(tuning.close_on_sniper_m)),
        _ => band,
    };
    Some(if adjusted.1 - adjusted.0 >= 5.0 {
        adjusted
    } else {
        band
    })
}

/// What the reflex reads at a think besides the bot and its memory.
#[derive(Clone, Copy)]
pub struct Situation<'a> {
    pub view: &'a MatchView,
    /// The map's spots (play 7); none before the map is ready.
    pub tactics: Option<&'a Tactics>,
    /// The team's danger spots (play 21).
    pub team: Option<&'a TeamKnowledge>,
    /// The bot's map side (spots off their spawn area).
    pub side: Option<i32>,
    /// Its trade partner (play 21).
    pub partner: Option<Place>,
    /// Its own weapon 1's kind (play 11).
    pub weapon: Option<WeaponKind>,
    /// Every place's weapon 1 kind, by place, where known (play 11).
    pub weapons: &'a [Option<WeaponKind>],
}

impl<'a> Situation<'a> {
    /// The view alone: no map, no team, no weapons.
    pub fn of(view: &'a MatchView) -> Self {
        Self {
            view,
            tactics: None,
            team: None,
            side: None,
            partner: None,
            weapon: None,
            weapons: &[],
        }
    }
}

/// Why the reflex takes the legs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegsWhy {
    /// Play 5: at a reloading enemy.
    Punish,
    /// Play 7: out of his sight.
    BreakSight,
    /// Play 7: back from another angle.
    RePeek,
    /// Play 11: into the weapon's band.
    Range,
    /// Play 20: in after the grenade.
    AfterGrenade,
}

/// The legs as the reflex wants them this tick, instead of the play's go,
/// pace and stance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Legs {
    pub at: Vec3,
    pub within_m: f32,
    /// Sprint where the way allows (firing still stops the sprint).
    pub run: bool,
    pub crouch: bool,
    pub why: LegsWhy,
}

/// What the reflex wants of the motor this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Override {
    /// Look at this point (the aim, with its error), instead of the play's
    /// look.
    pub look_at: Option<Vec3>,
    /// The yaw and pitch error added to the aim, radians.
    pub error: (f32, f32),
    /// Hold fire this tick.
    pub fire: bool,
    /// The turn's time constant while aiming, seconds.
    pub turn_time_s: Option<f32>,
    /// Keys the reflex presses or holds this tick, OR'd into the decision:
    /// `SPOT` for one tick, `GRENADE` each tick until the throw starts,
    /// `AIM` while it watches a partner's death spot.
    pub keys: u32,
    /// The legs instead of the play's (`None`: the play's).
    pub legs: Option<Legs>,
    /// Play 11: the distance band to the target it prefers, metres (a hint
    /// for the plays; the reflex steps into it itself with `legs`).
    pub band_m: Option<(f32, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Push {
    toward: Vec3,
    from: u32,
    until: u32,
    why: LegsWhy,
}

/// Play 7's steps.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Cover {
    #[default]
    Open,
    Hiding {
        at: Vec3,
        enemy: Vec3,
        started: u32,
        arrived: Option<u32>,
        wait_ms: u32,
    },
    Peeking {
        at: Vec3,
        until: u32,
    },
}

/// A grenade being thrown (play 20).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Throw {
    at: Vec3,
    started: u32,
    hold_ms: u32,
    pressed: Option<u32>,
}

/// An enemy sensed staying at one place.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Camp {
    who: Place,
    at: Vec3,
    since: u32,
    last: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Reflex {
    target: Option<Place>,
    /// When it took the target.
    since: u32,
    /// The error drawn for this target, radians (yaw, pitch).
    error: (f32, f32),
    burst_until: u32,
    pause_until: u32,
    /// A heard shot to turn to, and until when.
    sound: Option<(Vec3, u32)>,
    /// No shot at the target before this: noticed + the reaction delay.
    first_shot_at: u32,
    /// The life it counts for (`PlaceView::spawned`).
    life: Option<u32>,
    /// Its health at the last think.
    health: Option<f32>,
    duel_until: u32,
    /// Reload and switch sounds up to this time were weighed.
    punished_until: u32,
    push: Option<Push>,
    cover: Cover,
    /// The spot it last fought from (play 7: not again next).
    last_peek: Option<Vec3>,
    band: Option<(f32, f32)>,
    /// A step into the band.
    range_step: Option<Vec3>,
    spot_pending: Option<Place>,
    last_spot: Option<u32>,
    spotted: Vec<(Place, u32)>,
    throw: Option<Throw>,
    thrown: u8,
    next_grenade_roll: u32,
    /// Enemies seen holding an angle: who, where, when.
    held: Vec<(Place, Vec3, u32)>,
    camps: Vec<Camp>,
    /// The partner's death spot to watch, until when.
    trade: Option<(Vec3, u32)>,
    /// Fire held since the last reload, ms, and the clip drawn (0: not
    /// drawn yet).
    fired_ms: u32,
    clip_ms: u32,
    /// The last tick and the time since the one before, ms (the fire's
    /// time).
    last_tick: Option<u32>,
    tick_ms: u32,
    /// An enemy last seen (a seen track) at.
    last_seen: u32,
    /// Empty with a target seen and no cover found: reload where it is.
    reload_here: bool,
    /// No shot before this (a reload under way).
    reloaded_until: u32,
}

/// The bearing of `to` from `from`, radians.
fn bearing(from: Vec3, to: Vec3) -> f32 {
    yaw_of([to.x - from.x, to.z - from.z])
}

/// How far the view of `me` is off looking at `point` (plus `error`),
/// radians.
fn off_by(me: &PlaceView, point: Vec3, error: (f32, f32)) -> f32 {
    let eye = me.eye();
    let (dx, dz) = (point.x - eye.x, point.z - eye.z);
    let want_yaw = yaw_of([dx, dz]) + error.0;
    let want_pitch = (point.y - eye.y).atan2(dx.hypot(dz)) + error.1;
    wrap(want_yaw - me.yaw)
        .abs()
        .max((want_pitch - me.pitch).abs())
}

/// Play 7: whether a peek from `spot` is another angle on `enemy` than the
/// last one, from `last`.
fn another_angle(enemy: Vec3, last: Vec3, spot: Vec3, c: &CombatTuning) -> bool {
    wrap(bearing(enemy, spot) - bearing(enemy, last)).abs() >= c.repeek_min_deg.to_radians()
        || spot.distance_xz(last) >= c.repeek_min_m
}

/// `from` moved `m` metres on the ground toward `to` (away if negative).
fn step_toward(from: Vec3, to: Vec3, m: f32) -> Vec3 {
    let (dx, dz) = (to.x - from.x, to.z - from.z);
    let d = dx.hypot(dz).max(1e-3);
    Vec3::new(from.x + dx / d * m, from.y, from.z + dz / d * m)
}

impl Reflex {
    pub fn target(&self) -> Option<Place> {
        self.target
    }

    /// Grenades thrown this life.
    pub fn grenades_thrown(&self) -> u8 {
        self.thrown
    }

    /// At a think, with the view alone: the target and the turn to a sound
    /// (the first form; [`Self::think_with`] does everything).
    pub fn think(
        &mut self,
        me: &PlaceView,
        memory: &Memory,
        now: u32,
        aim: &AimTuning,
        skill: f32,
        rng: &mut Rng,
    ) {
        self.new_life(me);
        self.choose(me, None, memory, now, aim, skill, rng);
    }

    /// At a think: the target, and plays 5, 7, 11, 19, 20 and 21.
    #[allow(clippy::too_many_arguments)]
    pub fn think_with(
        &mut self,
        me: &PlaceView,
        sit: &Situation,
        memory: &Memory,
        now: u32,
        aim: &AimTuning,
        skill: f32,
        rng: &mut Rng,
    ) {
        let c = &aim.combat;
        self.new_life(me);
        let hit = self
            .health
            .is_some_and(|before| me.health <= before - c.hit_drop);
        self.health = Some(me.health);
        self.choose(me, Some(sit.view), memory, now, aim, skill, rng);
        if self.target.is_some() || hit {
            self.duel_until = now + c.duel_ms;
        }
        self.trade_watch(sit, now, c);
        self.watch_enemies(sit.view, memory, now, c);
        self.punish(me, sit.view, memory, now, c, rng);
        if !memory.all_seen(now).is_empty() {
            self.last_seen = now;
        }
        if hit {
            self.break_sight(me, sit, memory, now, c, rng);
        }
        // About a clip fired with a target seen: out of sight to reload,
        // else reload where it stands.
        let empty = self.clip_ms > 0 && self.fired_ms >= self.clip_ms;
        if empty && self.target.is_some() && !self.reload_here {
            self.break_sight(me, sit, memory, now, c, rng);
            self.reload_here = !matches!(self.cover, Cover::Hiding { .. });
        }
        self.cover_steps(me, sit, now, c, rng);
        self.matchup(me, sit, memory, now, c);
        self.grenade_roll(me, sit.view, now, c, rng);
    }

    /// A new life: the per-life counts start again.
    fn new_life(&mut self, me: &PlaceView) {
        if self.life != Some(me.spawned) {
            if self.life.is_some() {
                *self = Self::default();
            }
            self.life = Some(me.spawned);
        }
    }

    /// The target among the seen enemies: the present one if still seen,
    /// else the nearest in range, a wounded one counting nearer (with the
    /// view); or a sound to turn to.
    #[allow(clippy::too_many_arguments)]
    fn choose(
        &mut self,
        me: &PlaceView,
        view: Option<&MatchView>,
        memory: &Memory,
        now: u32,
        aim: &AimTuning,
        skill: f32,
        rng: &mut Rng,
    ) {
        let seen = memory.all_seen(now);
        let in_range = |t: &&SeenTrack| t.at().distance_xz(me.feet) <= aim.range_m;
        let keep = self
            .target
            .and_then(|target| seen.iter().filter(in_range).find(|t| t.who() == target));
        let bonus = view.map_or(0.0, |_| aim.combat.wounded_bonus_m);
        let score = |t: &SeenTrack| {
            let health = view
                .and_then(|v| v.place(t.who()))
                .map_or(1.0, |p| p.health.clamp(0.0, 1.0));
            t.at().distance_xz(me.feet) - bonus * (1.0 - health)
        };
        let choice = keep.or_else(|| {
            seen.iter()
                .filter(in_range)
                .min_by(|a, b| score(a).total_cmp(&score(b)))
        });
        match choice {
            Some(track) if Some(track.who()) != self.target => {
                self.target = Some(track.who());
                self.since = now;
                // A fresh target: the aim starts well off and settles.
                let error = mix(aim.error_deg, skill).to_radians() * 3.0;
                self.error = (rng.range(-error, error), rng.range(-error, error) * 0.5);
                // The first shot a reaction after noticing.
                let t = track.track();
                let noticed = t.sight_since.unwrap_or(now) + t.notice_ms;
                let reaction = mix(aim.reaction_ms, skill).max(0.0) * rng.range(0.85, 1.15);
                self.first_shot_at = noticed.max(self.since.min(noticed)) + reaction as u32;
                self.sound = None;
                // Play 19: a new enemy is spotted once in the crosshair.
                let c = &aim.combat;
                let lately = self
                    .spotted
                    .iter()
                    .any(|(who, at)| *who == track.who() && *at + c.spot_again_ms > now);
                self.spot_pending = (!lately && rng.unit() < c.spot_chance).then_some(track.who());
            }
            Some(_) => {}
            None => {
                self.target = None;
                // Play 6: a shot heard lately, nothing seen: turn to it.
                let heard = memory
                    .tracks()
                    .iter()
                    .filter(|t| t.source == Source::Heard && t.time + 1_500 >= now)
                    .min_by(|a, b| {
                        a.at.distance_xz(me.feet)
                            .total_cmp(&b.at.distance_xz(me.feet))
                    });
                if let Some(track) = heard {
                    self.sound = Some((track.at, now + 2_000));
                }
            }
        }
    }

    /// Play 21: the partner died at a spot lately: watch it.
    fn trade_watch(&mut self, sit: &Situation, now: u32, c: &CombatTuning) {
        if self.trade.is_some_and(|(_, until)| now >= until) {
            self.trade = None;
        }
        let (Some(team), Some(partner)) = (sit.team, sit.partner) else {
            return;
        };
        if let Some(spot) = team
            .danger_within(now, c.trade_ms)
            .filter(|spot| spot.victim == partner)
            .max_by_key(|spot| spot.time)
        {
            self.trade = Some((spot.at, spot.time + c.trade_ms));
        }
    }

    /// What the senses showed of the enemies' habits: who holds an angle
    /// (seen crouched, aiming, still) and who stays at one place.
    fn watch_enemies(&mut self, view: &MatchView, memory: &Memory, now: u32, c: &CombatTuning) {
        for track in memory.all_seen(now) {
            let Some(body) = view.place(track.who()) else {
                continue;
            };
            if body.crouched() && body.keys & key::AIM != 0 && body.speed() <= c.held_speed {
                self.held.retain(|(who, _, _)| *who != track.who());
                self.held.push((track.who(), track.at(), now));
            }
        }
        self.held.retain(|(_, _, at)| *at + c.held_memory_ms >= now);
        for track in memory.tracks() {
            if !matches!(track.source, Source::Seen | Source::Heard) || track.time + 500 < now {
                continue;
            }
            match self.camps.iter_mut().find(|camp| camp.who == track.who) {
                Some(camp) if camp.at.distance_xz(track.at) <= c.camp_m + track.error_m => {
                    camp.last = camp.last.max(track.time);
                }
                Some(camp) => {
                    *camp = Camp {
                        who: track.who,
                        at: track.at,
                        since: track.time,
                        last: track.time,
                    };
                }
                None => self.camps.push(Camp {
                    who: track.who,
                    at: track.at,
                    since: track.time,
                    last: track.time,
                }),
            }
        }
        self.camps
            .retain(|camp| camp.last + c.held_memory_ms >= now);
    }

    /// Play 5: a reload or switch sensed in a duel: at him.
    fn punish(
        &mut self,
        me: &PlaceView,
        view: &MatchView,
        memory: &Memory,
        now: u32,
        c: &CombatTuning,
        rng: &mut Rng,
    ) {
        let weighed = self.punished_until;
        for sound in &view.sounds {
            if !matches!(sound.kind, SoundKind::Reload | SoundKind::Switch) || sound.time <= weighed
            {
                continue;
            }
            self.punished_until = self.punished_until.max(sound.time);
            if sound.time + c.punish_window_ms < now || now >= self.duel_until {
                continue;
            }
            let Some(enemy) = view.place(sound.from) else {
                continue;
            };
            if enemy.team == me.team || !enemy.alive {
                continue;
            }
            let Some(track) = memory.tracks().iter().find(|t| t.who == sound.from) else {
                continue;
            };
            let d = sound.at.distance_xz(me.feet);
            let seen = memory.seen(sound.from, now).is_some() && d <= c.punish_m;
            let heard = track.time + 300 >= sound.time && d <= c.punish_heard_m;
            if (seen || heard) && rng.unit() < c.punish_chance {
                self.push = Some(Push {
                    toward: track.at,
                    from: now,
                    until: now + c.punish_ms,
                    why: LegsWhy::Punish,
                });
                // Out of cover at once: the window is now.
                self.cover = Cover::Open;
            }
        }
    }

    /// Play 7: hit in a duel: to a spot near by hidden from the enemy.
    fn break_sight(
        &mut self,
        me: &PlaceView,
        sit: &Situation,
        memory: &Memory,
        now: u32,
        c: &CombatTuning,
        rng: &mut Rng,
    ) {
        if matches!(self.cover, Cover::Hiding { .. })
            || self.push.is_some_and(|p| now < p.until)
            || now >= self.duel_until
        {
            return;
        }
        let Some(tactics) = sit.tactics else {
            return;
        };
        // The enemy it fights: the target, else the last one seen.
        let enemy = self
            .target
            .and_then(|target| memory.seen(target, now))
            .map(|t| t.at())
            .or_else(|| {
                memory
                    .tracks()
                    .iter()
                    .filter(|t| t.source == Source::Seen)
                    .max_by_key(|t| t.time)
                    .map(|t| t.at)
            });
        let Some(enemy) = enemy else {
            return;
        };
        let query = SpotQuery {
            hidden_from: Some(enemy),
            side: sit.side,
            samples: 48,
            ..SpotQuery::ring(me.feet, c.hide_m.0, c.hide_m.1)
        };
        if let Some(spot) = tactics.spots(&query, now, 1, rng).first() {
            self.last_peek = Some(me.feet);
            self.cover = Cover::Hiding {
                at: spot.at,
                enemy,
                started: now,
                arrived: None,
                wait_ms: rng.range(c.hide_wait_ms.0 as f32, c.hide_wait_ms.1 as f32) as u32,
            };
        }
    }

    /// Play 7 on: there, waited, then back at another angle.
    fn cover_steps(
        &mut self,
        me: &PlaceView,
        sit: &Situation,
        now: u32,
        c: &CombatTuning,
        rng: &mut Rng,
    ) {
        match self.cover {
            Cover::Open => {}
            Cover::Hiding {
                at,
                enemy,
                started,
                arrived,
                wait_ms,
            } => match arrived {
                None if me.feet.distance_xz(at) <= 1.2 => {
                    self.cover = Cover::Hiding {
                        at,
                        enemy,
                        started,
                        arrived: Some(now),
                        wait_ms,
                    };
                }
                None if now >= started + c.hide_give_up_ms => self.cover = Cover::Open,
                None => {}
                Some(arrived) if now >= arrived + wait_ms => {
                    self.cover = Cover::Open;
                    let Some(tactics) = sit.tactics else {
                        return;
                    };
                    let query = SpotQuery {
                        sight_to: Some(enemy),
                        side: sit.side,
                        samples: 48,
                        ..SpotQuery::ring(at, c.peek_m.0, c.peek_m.1)
                    };
                    let last = self.last_peek;
                    let spot = tactics.spots(&query, now, 8, rng).into_iter().find(|spot| {
                        last.is_none_or(|last| another_angle(enemy, last, spot.at, c))
                    });
                    if let Some(spot) = spot {
                        self.last_peek = Some(spot.at);
                        self.cover = Cover::Peeking {
                            at: spot.at,
                            until: now + c.peek_ms,
                        };
                    }
                }
                Some(_) => {}
            },
            Cover::Peeking { at, until } => {
                if now >= until || me.feet.distance_xz(at) <= 1.0 {
                    self.cover = Cover::Open;
                }
            }
        }
    }

    /// Play 11: the band against the target, and a step into it.
    fn matchup(
        &mut self,
        me: &PlaceView,
        sit: &Situation,
        memory: &Memory,
        now: u32,
        c: &CombatTuning,
    ) {
        self.band = None;
        self.range_step = None;
        let Some(track) = self.target.and_then(|target| memory.seen(target, now)) else {
            return;
        };
        let enemy = sit.weapons.get(track.who()).copied().flatten();
        let Some(band) = preferred_band(sit.weapon, enemy, c) else {
            return;
        };
        self.band = Some(band);
        if !c.band_legs {
            return;
        }
        let d = track.at().distance_xz(me.feet);
        let step = if d < band.0 - c.band_slack_m {
            Some(step_toward(me.feet, track.at(), -c.band_step_m))
        } else if d > band.1 + c.band_slack_m {
            Some(step_toward(me.feet, track.at(), c.band_step_m))
        } else {
            None
        };
        self.range_step = step.filter(|at| {
            sit.tactics
                .is_some_and(|tactics| tactics.fine_goal(*at, now, sit.side))
        });
    }

    /// Play 20: now and then, with nothing seen, a grenade at a place an
    /// enemy was seen holding an angle or sensed staying at.
    fn grenade_roll(
        &mut self,
        me: &PlaceView,
        view: &MatchView,
        now: u32,
        c: &CombatTuning,
        rng: &mut Rng,
    ) {
        if self.throw.is_some()
            || self.target.is_some()
            || self.thrown >= c.grenades
            || now < self.next_grenade_roll
        {
            return;
        }
        let held = self.held.iter().map(|(_, at, _)| *at);
        let camps = self
            .camps
            .iter()
            .filter(|camp| camp.last >= camp.since + c.camp_ms)
            .map(|camp| camp.at);
        let friend_near = |at: Vec3| {
            view.team(me.team)
                .any(|p| p.place != me.place && p.feet.distance_xz(at) < c.grenade_friend_m)
        };
        let at = held
            .chain(camps)
            .filter(|at| {
                let d = at.distance_xz(me.feet);
                (c.grenade_m.0..=c.grenade_m.1).contains(&d) && !friend_near(*at)
            })
            .min_by(|a, b| a.distance_xz(me.feet).total_cmp(&b.distance_xz(me.feet)));
        let Some(at) = at else {
            return;
        };
        self.next_grenade_roll = now + c.grenade_roll_ms;
        if rng.unit() < c.grenade_chance {
            self.throw = Some(Throw {
                at,
                started: now,
                hold_ms: rng.range(c.grenade_hold_ms.0 as f32, c.grenade_hold_ms.1 as f32) as u32,
                pressed: None,
            });
        }
    }

    /// The reload habit: `key::RELOAD` for this tick, or 0. After about a
    /// clip of fire, at the first moment with no target seen or out of
    /// sight (or where it stands when no cover was found); tactically
    /// after `reload_idle_ms` with nothing seen and some fire.
    fn reload(&mut self, seen: bool, now: u32, c: &CombatTuning, rng: &mut Rng) -> u32 {
        if self.clip_ms == 0 {
            self.clip_ms = rng.range(c.reload_fire_ms.0 as f32, c.reload_fire_ms.1 as f32) as u32;
        }
        if now < self.reloaded_until || self.fired_ms == 0 {
            return 0;
        }
        let empty = self.fired_ms >= self.clip_ms;
        let hiding = matches!(self.cover, Cover::Hiding { .. });
        let tactical = !seen
            && now >= self.last_seen + c.reload_idle_ms
            && self.fired_ms >= c.reload_some_fire_ms;
        if !(empty && (!seen || hiding || self.reload_here) || tactical) {
            return 0;
        }
        self.fired_ms = 0;
        self.clip_ms = rng.range(c.reload_fire_ms.0 as f32, c.reload_fire_ms.1 as f32) as u32;
        self.reloaded_until = now + c.reload_ms;
        self.reload_here = false;
        key::RELOAD
    }

    /// The legs the reflex takes this tick, if any.
    fn legs(&mut self, now: u32) -> Option<Legs> {
        if self.push.is_some_and(|p| now >= p.until) {
            self.push = None;
        }
        match self.cover {
            Cover::Hiding { at, .. } => {
                return Some(Legs {
                    at,
                    within_m: 0.8,
                    run: true,
                    crouch: false,
                    why: LegsWhy::BreakSight,
                });
            }
            Cover::Peeking { at, .. } => {
                return Some(Legs {
                    at,
                    within_m: 0.8,
                    run: false,
                    crouch: false,
                    why: LegsWhy::RePeek,
                });
            }
            Cover::Open => {}
        }
        if let Some(push) = self.push.filter(|p| now >= p.from) {
            return Some(Legs {
                at: push.toward,
                within_m: 3.0,
                run: false,
                crouch: false,
                why: push.why,
            });
        }
        let at = self.range_step.filter(|_| self.target.is_some())?;
        Some(Legs {
            at,
            within_m: 1.0,
            run: false,
            crouch: false,
            why: LegsWhy::Range,
        })
    }

    /// One tick: where to look, whether to fire, the keys and the legs.
    /// `fire_allowed`: the play's word (a carrier cannot shoot).
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        me: &PlaceView,
        view: &MatchView,
        memory: &Memory,
        now: u32,
        fire_allowed: bool,
        aim: &AimTuning,
        skill: f32,
        rng: &mut Rng,
    ) -> Override {
        let c = &aim.combat;
        self.tick_ms = self
            .last_tick
            .map_or(50, |last| now.saturating_sub(last).clamp(1, 100));
        self.last_tick = Some(now);
        let legs = self.legs(now);
        let settle_s = mix(aim.settle_s, skill).max(0.05);
        let floor = mix(aim.error_deg, skill).to_radians();
        // Play 20: a throw under way has the look and the hands.
        if let Some(mut throw) = self.throw {
            let d = throw.at.distance_xz(me.feet);
            let point = throw.at + Vec3::new(0.0, 1.0 + 0.12 * d, 0.0);
            let error = (floor * (rng.unit() - 0.5), floor * 0.5 * (rng.unit() - 0.5));
            let mut keys = 0;
            match throw.pressed {
                None if off_by(me, point, error) <= c.grenade_within_deg.to_radians() => {
                    throw.pressed = Some(now);
                    keys = key::GRENADE;
                    self.throw = Some(throw);
                }
                None if now >= throw.started + c.grenade_give_up_ms => self.throw = None,
                None => {}
                Some(pressed) if now < pressed + throw.hold_ms => keys = key::GRENADE,
                Some(_) => {
                    // Let go: thrown. Then in, after it goes off.
                    self.throw = None;
                    self.thrown += 1;
                    self.push = Some(Push {
                        toward: throw.at,
                        from: now + c.grenade_push_ms.0,
                        until: now + c.grenade_push_ms.1,
                        why: LegsWhy::AfterGrenade,
                    });
                }
            }
            return Override {
                look_at: Some(point),
                error,
                fire: false,
                turn_time_s: Some(settle_s * 0.5),
                keys,
                legs: None,
                band_m: self.band,
            };
        }
        let seen = self.target.and_then(|target| memory.seen(target, now));
        if seen.is_some() {
            self.last_seen = now;
        }
        let reload = self.reload(seen.is_some(), now, c, rng);
        let Some(track) = seen else {
            self.target = None;
            // Play 21: the partner's death spot, weapon up; else play 6.
            if let Some((at, _)) = self.trade.filter(|(_, until)| now < *until) {
                return Override {
                    look_at: Some(at + Vec3::new(0.0, 1.3, 0.0)),
                    keys: key::AIM | reload,
                    legs,
                    ..Override::default()
                };
            }
            return match self.sound {
                Some((at, until)) if now < until => Override {
                    look_at: Some(at + Vec3::new(0.0, 1.4, 0.0)),
                    keys: reload,
                    legs,
                    ..Override::default()
                },
                _ => Override {
                    keys: reload,
                    legs,
                    ..Override::default()
                },
            };
        };
        // The seen body where it is now (a seen enemy is tracked by eye).
        let body = view.place(track.who()).filter(|p| p.alive);
        let point = body.map_or(track.at() + Vec3::new(0.0, 1.3, 0.0), PlaceView::chest);
        // The error settles from its start toward the skill's, never to 0.
        let t = now.saturating_sub(self.since) as f32 / 1000.0;
        let fade = (-t / settle_s).exp();
        let error = (
            self.error.0 * fade + floor * (rng.unit() - 0.5),
            self.error.1 * fade + floor * 0.5 * (rng.unit() - 0.5),
        );
        let off = off_by(me, point, error);
        // Play 19: spot him once he is in the crosshair, the cooldown kept.
        let mut keys = reload;
        if self.spot_pending == Some(track.who())
            && off <= c.spot_within_deg.to_radians()
            && self
                .last_spot
                .is_none_or(|last| now >= last + c.spot_cooldown_ms)
        {
            keys |= key::SPOT;
            self.last_spot = Some(now);
            self.spotted.retain(|(who, _)| *who != track.who());
            self.spotted.push((track.who(), now));
            self.spot_pending = None;
        }
        // Fire in bursts when the view is near the aim, a reaction after
        // noticing.
        let within = off <= aim.fire_within_deg.to_radians();
        let mut fire = false;
        if fire_allowed
            && within
            && now >= self.first_shot_at
            && now >= self.reloaded_until
            && point.distance_xz(me.feet) <= aim.range_m
        {
            if now < self.burst_until {
                fire = true;
            } else if now >= self.pause_until {
                self.burst_until =
                    now + rng.range(aim.burst_ms.0 as f32, aim.burst_ms.1 as f32) as u32;
                self.pause_until = self.burst_until
                    + rng.range(aim.pause_ms.0 as f32, aim.pause_ms.1 as f32) as u32;
                fire = true;
            }
        }
        if fire {
            self.fired_ms += self.tick_ms;
        }
        Override {
            look_at: Some(point),
            error,
            fire,
            turn_time_s: Some(settle_s * 0.5),
            keys,
            legs,
            band_m: self.band,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v026g::bots::know::{DangerSpot, Percept};
    use crate::v026g::bots::view::{Mode, Sound};

    fn body(place: Place, team: u8, feet: Vec3, yaw: f32) -> PlaceView {
        PlaceView {
            place,
            team,
            bot: true,
            present: true,
            alive: true,
            feet,
            yaw,
            pitch: 0.0,
            velocity: [0.0, 0.0],
            keys: 0,
            health: 1.0,
            spawned: 0,
            kills: 0,
            deaths: 0,
            streak: 0,
            carrying: None,
            seen_at: 0,
        }
    }

    fn view_of(places: Vec<PlaceView>) -> MatchView {
        MatchView {
            clock: 0,
            mode: Mode::TeamDeathmatch,
            time_left_s: 600.0,
            total_s: 900.0,
            points: [0, 0],
            places,
            batteries: vec![],
            containers: vec![],
            sounds: vec![],
            deaths: vec![],
            world_time: 0,
            can_respawn: [true, true],
        }
    }

    fn percept(who: Place, at: Vec3, time: u32, source: Source) -> Percept {
        Percept {
            who,
            at,
            error_m: 0.0,
            time,
            source,
            busy: false,
        }
    }

    fn sense(memory: &mut Memory, percepts: &[Percept], now: u32) {
        memory.update(percepts, now, |_| 300, 3_000, 20_000);
    }

    /// The pitch and yaw that look at `point` from `me`, so the view is on
    /// it.
    fn facing(me: &mut PlaceView, point: Vec3) {
        let eye = me.eye();
        let (dx, dz) = (point.x - eye.x, point.z - eye.z);
        me.yaw = yaw_of([dx, dz]);
        me.pitch = (point.y - eye.y).atan2(dx.hypot(dz));
    }

    #[test]
    fn it_aims_only_at_a_noticed_enemy_and_fires_once_on_target() {
        let me = body(0, 0, Vec3::ZERO, 0.0);
        let enemy = body(1, 1, Vec3::new(0.0, 0.0, 20.0), 0.0);
        let view = view_of(vec![me.clone(), enemy]);
        let aim = AimTuning::default();
        let mut memory = Memory::default();
        let mut reflex = Reflex::default();
        let mut rng = Rng::new(9);
        let sight = |time| percept(1, Vec3::new(0.0, 0.0, 20.0), time, Source::Seen);
        sense(&mut memory, &[sight(1_000)], 1_000);
        reflex.think(&me, &memory, 1_000, &aim, 1.0, &mut rng);
        assert_eq!(reflex.target(), None, "not noticed yet");
        // Noticed: it aims; facing the enemy squarely, it fires once the
        // error has settled.
        sense(&mut memory, &[sight(1_300)], 1_300);
        reflex.think(&me, &memory, 1_300, &aim, 1.0, &mut rng);
        assert_eq!(reflex.target(), Some(1));
        let mut fired = false;
        for k in 0..20 {
            let now = 1_300 + k * 50;
            sense(&mut memory, &[sight(now)], now);
            let o = reflex.tick(&me, &view, &memory, now, true, &aim, 1.0, &mut rng);
            assert!(o.look_at.is_some());
            fired |= o.fire;
        }
        assert!(fired);
        // The play holds fire: no shot.
        let o = reflex.tick(&me, &view, &memory, 2_400, false, &aim, 1.0, &mut rng);
        assert!(!o.fire);
    }

    #[test]
    fn no_aim_and_no_shot_without_a_seen_track() {
        // Heard, told and cheated enemies all about, never seen: never a
        // shot, never an aim with an error, never a spot or a grenade.
        let me = body(0, 0, Vec3::ZERO, 0.0);
        let enemies: Vec<PlaceView> = (1..4)
            .map(|p| body(p, 1, Vec3::new(p as f32 * 4.0, 0.0, 15.0), 0.0))
            .collect();
        let mut places = vec![me.clone()];
        places.extend(enemies.iter().cloned());
        let view = view_of(places);
        let aim = AimTuning {
            combat: CombatTuning {
                grenade_chance: 1.0,
                ..CombatTuning::default()
            },
            ..AimTuning::default()
        };
        let mut memory = Memory::default();
        let mut reflex = Reflex::default();
        let mut rng = Rng::new(3);
        for k in 0..200u32 {
            let now = 1_000 + k * 50;
            let percepts = [
                percept(1, enemies[0].feet, now, Source::Heard),
                percept(2, enemies[1].feet, now, Source::Told { by: 5 }),
                percept(
                    3,
                    enemies[2].feet,
                    now,
                    Source::Cheat(crate::v026g::bots::know::CheatKind::Radar),
                ),
            ];
            sense(&mut memory, &percepts, now);
            if k % 6 == 0 {
                reflex.think_with(
                    &me,
                    &Situation::of(&view),
                    &memory,
                    now,
                    &aim,
                    0.5,
                    &mut rng,
                );
            }
            let o = reflex.tick(&me, &view, &memory, now, true, &aim, 0.5, &mut rng);
            assert!(!o.fire, "{o:?}");
            assert_eq!(o.keys & key::SPOT, 0);
            assert!(reflex.target().is_none());
            // An aim with an error is only a grenade's, and only at the
            // place the heard enemy was sensed staying at: never at the
            // told or the cheated one.
            if o.error != (0.0, 0.0) || o.keys & key::GRENADE != 0 {
                let at = o.look_at.unwrap();
                assert!(at.distance_xz(enemies[0].feet) < 0.5, "{o:?}");
            }
        }
    }

    #[test]
    fn no_first_shot_before_noticing_and_the_reaction() {
        let mut me = body(0, 0, Vec3::ZERO, 0.0);
        let at = Vec3::new(0.0, 0.0, 20.0);
        let enemy = body(1, 1, at, 0.0);
        facing(&mut me, enemy.chest());
        let view = view_of(vec![me.clone(), enemy.clone()]);
        for (skill, seed) in [(0.0, 1), (0.5, 2), (1.0, 3)] {
            let aim = AimTuning {
                fire_within_deg: 180.0,
                ..AimTuning::default()
            };
            let reaction = mix(aim.reaction_ms, skill) * 0.85;
            let mut memory = Memory::default();
            let mut reflex = Reflex::default();
            let mut rng = Rng::new(seed);
            let mut first = None;
            for k in 0..60u32 {
                let now = 1_000 + k * 50;
                sense(&mut memory, &[percept(1, at, now, Source::Seen)], now);
                if k % 5 == 0 {
                    reflex.think_with(
                        &me,
                        &Situation::of(&view),
                        &memory,
                        now,
                        &aim,
                        skill,
                        &mut rng,
                    );
                }
                let o = reflex.tick(&me, &view, &memory, now, true, &aim, skill, &mut rng);
                if o.fire && first.is_none() {
                    first = Some(now);
                }
            }
            // Seen at 1 000, noticed 300 ms on, then the reaction.
            let first = first.expect("it fires in the end");
            assert!(
                first as f32 >= 1_300.0 + reaction,
                "skill {skill}: {first} before {}",
                1_300.0 + reaction
            );
        }
        // A weaker bot reacts slower.
        let aim = AimTuning::default();
        assert!(mix(aim.reaction_ms, 0.0) > mix(aim.reaction_ms, 1.0));
    }

    #[test]
    fn the_aim_error_never_settles_to_zero() {
        let mut me = body(0, 0, Vec3::ZERO, 0.0);
        let at = Vec3::new(0.0, 0.0, 20.0);
        let enemy = body(1, 1, at, 0.0);
        facing(&mut me, enemy.chest());
        let view = view_of(vec![me.clone(), enemy]);
        let aim = AimTuning::default();
        let mut memory = Memory::default();
        let mut reflex = Reflex::default();
        let mut rng = Rng::new(4);
        let mut largest: f32 = 0.0;
        for k in 0..400u32 {
            let now = 1_000 + k * 50;
            sense(&mut memory, &[percept(1, at, now, Source::Seen)], now);
            if k % 5 == 0 {
                reflex.think(&me, &memory, now, &aim, 1.0, &mut rng);
            }
            let o = reflex.tick(&me, &view, &memory, now, true, &aim, 1.0, &mut rng);
            if k > 200 {
                largest = largest.max(o.error.0.abs());
            }
        }
        assert!(largest > 0.1_f32.to_radians(), "{largest}");
    }

    #[test]
    fn a_wounded_enemy_is_preferred_among_the_seen() {
        let me = body(0, 0, Vec3::ZERO, 0.0);
        let near = body(1, 1, Vec3::new(0.0, 0.0, 15.0), 0.0);
        let mut far = body(2, 1, Vec3::new(3.0, 0.0, 25.0), 0.0);
        far.health = 0.2;
        let view = view_of(vec![me.clone(), near.clone(), far.clone()]);
        let aim = AimTuning::default();
        let mut memory = Memory::default();
        let mut rng = Rng::new(5);
        for now in [1_000, 1_400] {
            let seen = [
                percept(1, near.feet, now, Source::Seen),
                percept(2, far.feet, now, Source::Seen),
            ];
            sense(&mut memory, &seen, now);
        }
        let mut reflex = Reflex::default();
        reflex.think_with(
            &me,
            &Situation::of(&view),
            &memory,
            1_400,
            &aim,
            0.5,
            &mut rng,
        );
        assert_eq!(reflex.target(), Some(2));
        // Without the view's health (the first form), the nearest.
        let mut reflex = Reflex::default();
        reflex.think(&me, &memory, 1_400, &aim, 0.5, &mut rng);
        assert_eq!(reflex.target(), Some(1));
    }

    #[test]
    fn spot_is_pressed_once_per_new_enemy_and_keeps_the_cooldown() {
        let mut me = body(0, 0, Vec3::ZERO, 0.0);
        let a = body(1, 1, Vec3::new(0.0, 0.0, 20.0), 0.0);
        let b = body(2, 1, Vec3::new(1.0, 0.0, 20.0), 0.0);
        facing(&mut me, a.chest());
        let mut view = view_of(vec![me.clone(), a.clone(), b.clone()]);
        let aim = AimTuning::default();
        let mut memory = Memory::default();
        let mut reflex = Reflex::default();
        let mut rng = Rng::new(6);
        let mut spots = Vec::new();
        for k in 0..200u32 {
            let now = 1_000 + k * 50;
            // A at first; from 1.5 s only B is in sight (A gone behind
            // something), so B is a new enemy within A's cooldown; A back
            // at 6 s.
            let mut seen = Vec::new();
            if !(1_500..6_000).contains(&now) {
                seen.push(percept(1, a.feet, now, Source::Seen));
            }
            if now >= 1_500 {
                seen.push(percept(2, b.feet, now, Source::Seen));
            }
            sense(&mut memory, &seen, now);
            view.clock = now;
            if k % 5 == 0 {
                reflex.think_with(
                    &me,
                    &Situation::of(&view),
                    &memory,
                    now,
                    &aim,
                    0.5,
                    &mut rng,
                );
            }
            let target = reflex.target();
            if let Some(t) = target {
                let chest = view.place(t).unwrap().chest();
                facing(&mut me, chest);
            }
            let o = reflex.tick(&me, &view, &memory, now, true, &aim, 0.5, &mut rng);
            if o.keys & key::SPOT != 0 {
                spots.push((now, target));
            }
        }
        // A once, B once (after A's cooldown), A not again within
        // `spot_again_ms`.
        assert_eq!(spots.len(), 2, "{spots:?}");
        assert_eq!(spots[0].1, Some(1));
        assert_eq!(spots[1].1, Some(2));
        assert!(spots[1].0 >= spots[0].0 + aim.combat.spot_cooldown_ms);
    }

    #[test]
    fn a_reload_heard_in_a_duel_sends_the_legs_at_him() {
        let me = body(0, 0, Vec3::ZERO, 0.0);
        let enemy_at = Vec3::new(0.0, 0.0, 15.0);
        let enemy = body(1, 1, enemy_at, 0.0);
        let mut view = view_of(vec![me.clone(), enemy]);
        let aim = AimTuning {
            combat: CombatTuning {
                punish_chance: 1.0,
                ..CombatTuning::default()
            },
            ..AimTuning::default()
        };
        let mut memory = Memory::default();
        let mut reflex = Reflex::default();
        let mut rng = Rng::new(7);
        for now in [1_000, 1_300] {
            sense(&mut memory, &[percept(1, enemy_at, now, Source::Seen)], now);
            reflex.think_with(
                &me,
                &Situation::of(&view),
                &memory,
                now,
                &aim,
                0.5,
                &mut rng,
            );
        }
        assert_eq!(reflex.target(), Some(1));
        let o = reflex.tick(&me, &view, &memory, 1_350, true, &aim, 0.5, &mut rng);
        assert!(o.legs.is_none());
        // He reloads: within the window the legs go at him.
        view.sounds.push(Sound {
            from: 1,
            kind: SoundKind::Reload,
            at: enemy_at,
            time: 1_400,
        });
        sense(
            &mut memory,
            &[percept(1, enemy_at, 1_500, Source::Seen)],
            1_500,
        );
        reflex.think_with(
            &me,
            &Situation::of(&view),
            &memory,
            1_500,
            &aim,
            0.5,
            &mut rng,
        );
        let o = reflex.tick(&me, &view, &memory, 1_550, true, &aim, 0.5, &mut rng);
        let legs = o.legs.expect("pushes");
        assert_eq!(legs.why, LegsWhy::Punish);
        assert!(legs.at.distance_xz(enemy_at) < 0.1);
        // Over after `punish_ms`, and the same sound is not weighed again.
        reflex.think_with(
            &me,
            &Situation::of(&view),
            &memory,
            1_700,
            &aim,
            0.5,
            &mut rng,
        );
        let o = reflex.tick(&me, &view, &memory, 3_100, true, &aim, 0.5, &mut rng);
        assert!(o.legs.is_none(), "{o:?}");
        // A reload out of the duel and out of hearing: nothing.
        let mut reflex = Reflex::default();
        let far = Vec3::new(0.0, 0.0, 40.0);
        let mut memory = Memory::default();
        sense(&mut memory, &[percept(1, far, 9_000, Source::Heard)], 9_000);
        view.sounds = vec![Sound {
            from: 1,
            kind: SoundKind::Reload,
            at: far,
            time: 9_000,
        }];
        reflex.think_with(
            &me,
            &Situation::of(&view),
            &memory,
            9_100,
            &aim,
            0.5,
            &mut rng,
        );
        assert!(
            reflex
                .tick(&me, &view, &memory, 9_150, true, &aim, 0.5, &mut rng)
                .legs
                .is_none()
        );
    }

    #[test]
    fn hit_in_a_duel_it_breaks_sight_waits_and_repeeks_from_another_angle() {
        let tactics = Tactics::new(crate::v026g::bots::tactics::tests::field(), 1.0);
        // The field: a wall across x = 40 from z = 0 to 25. The enemy east
        // of it, the bot west, past the wall's end.
        let enemy_at = Vec3::new(55.0, 0.0, 30.0);
        let mut me = body(0, 0, Vec3::new(32.0, 0.0, 28.0), 0.0);
        let enemy = body(1, 1, enemy_at, 0.0);
        let view = view_of(vec![me.clone(), enemy]);
        let aim = AimTuning::default();
        let mut memory = Memory::default();
        let mut reflex = Reflex::default();
        let mut rng = Rng::new(8);
        let sit = Situation {
            tactics: Some(&tactics),
            side: Some(0),
            ..Situation::of(&view)
        };
        for now in [1_000, 1_300] {
            sense(&mut memory, &[percept(1, enemy_at, now, Source::Seen)], now);
            reflex.think_with(&me, &sit, &memory, now, &aim, 0.5, &mut rng);
        }
        assert_eq!(reflex.target(), Some(1));
        // Hit.
        me.health = 0.7;
        sense(
            &mut memory,
            &[percept(1, enemy_at, 1_600, Source::Seen)],
            1_600,
        );
        reflex.think_with(&me, &sit, &memory, 1_600, &aim, 0.5, &mut rng);
        let o = reflex.tick(&me, &view, &memory, 1_650, true, &aim, 0.5, &mut rng);
        let legs = o.legs.expect("breaks sight");
        assert_eq!(legs.why, LegsWhy::BreakSight);
        let hide = legs.at;
        let eye = |p: Vec3| p + Vec3::new(0.0, 1.6, 0.0);
        assert!(!tactics.sees(eye(enemy_at), eye(hide)), "{hide:?}");
        let d = hide.distance_xz(Vec3::new(32.0, 0.0, 28.0));
        assert!((2.0..=10.0).contains(&d), "{d}");
        // There: it waits 1-2 s, then comes back at another angle.
        me.feet = hide;
        let mut now = 2_000;
        reflex.think_with(&me, &sit, &memory, now, &aim, 0.5, &mut rng);
        let o = reflex.tick(&me, &view, &memory, now, true, &aim, 0.5, &mut rng);
        assert_eq!(o.legs.map(|l| l.why), Some(LegsWhy::BreakSight), "waits");
        let mut peek = None;
        while now < 5_000 {
            now += 250;
            reflex.think_with(&me, &sit, &memory, now, &aim, 0.5, &mut rng);
            let o = reflex.tick(&me, &view, &memory, now, true, &aim, 0.5, &mut rng);
            if let Some(legs) = o.legs.filter(|l| l.why == LegsWhy::RePeek) {
                peek = Some((now, legs.at));
                break;
            }
        }
        let (when, at) = peek.expect("re-peeks");
        assert!((3_000..=4_250).contains(&when), "{when}");
        assert!(another_angle(
            enemy_at,
            Vec3::new(32.0, 0.0, 28.0),
            at,
            &aim.combat
        ));
        assert!(tactics.sees(
            at + Vec3::new(0.0, 0.95, 0.0),
            enemy_at + Vec3::new(0.0, 1.0, 0.0)
        ));
    }

    #[test]
    fn a_grenade_goes_only_at_a_sensed_place_and_two_a_life() {
        let mut me = body(0, 0, Vec3::ZERO, 0.0);
        let camper_at = Vec3::new(0.0, 0.0, 18.0);
        let mut camper = body(1, 1, camper_at, 0.0);
        camper.keys = key::CROUCH | key::AIM;
        // Another enemy holding an angle the bot never sensed.
        let mut unseen = body(2, 1, Vec3::new(-12.0, 0.0, 12.0), 0.0);
        unseen.keys = key::CROUCH | key::AIM;
        let view = view_of(vec![me.clone(), camper, unseen]);
        let aim = AimTuning {
            combat: CombatTuning {
                grenade_chance: 1.0,
                grenade_roll_ms: 500,
                ..CombatTuning::default()
            },
            ..AimTuning::default()
        };
        let mut memory = Memory::default();
        let mut reflex = Reflex::default();
        let mut rng = Rng::new(10);
        // Seen holding an angle, then gone from sight (no target).
        for now in [1_000, 1_300] {
            sense(
                &mut memory,
                &[percept(1, camper_at, now, Source::Seen)],
                now,
            );
            reflex.think_with(
                &me,
                &Situation::of(&view),
                &memory,
                now,
                &aim,
                0.5,
                &mut rng,
            );
        }
        let mut held_ticks = 0;
        let mut throws = Vec::new();
        let mut now = 1_800;
        while now < 12_000 {
            sense(&mut memory, &[], now);
            if now % 250 == 0 {
                reflex.think_with(
                    &me,
                    &Situation::of(&view),
                    &memory,
                    now,
                    &aim,
                    0.5,
                    &mut rng,
                );
            }
            let before = reflex.grenades_thrown();
            let o = reflex.tick(&me, &view, &memory, now, true, &aim, 0.5, &mut rng);
            if let Some(look) = o
                .look_at
                .filter(|_| o.keys & key::GRENADE != 0 || o.error != (0.0, 0.0))
            {
                // Only ever at the place it saw him.
                assert!(look.distance_xz(camper_at) < 0.5, "{look:?}");
                assert!(!o.fire);
                facing(&mut me, look);
            }
            if o.keys & key::GRENADE != 0 {
                held_ticks += 1;
            }
            if reflex.grenades_thrown() > before {
                throws.push(now);
            }
            now += 50;
        }
        // Two throws (the kit's two), each with the key held 0.5-0.8 s.
        assert_eq!(throws.len(), 2, "{throws:?}");
        assert!((20..=34).contains(&held_ticks), "{held_ticks}");
        // A new life: two again.
        me.spawned = 50_000;
        reflex.think_with(
            &me,
            &Situation::of(&view),
            &memory,
            now,
            &aim,
            0.5,
            &mut rng,
        );
        assert_eq!(reflex.grenades_thrown(), 0);
    }

    #[test]
    fn a_partners_death_spot_is_watched_weapon_up_then_left() {
        let me = body(0, 0, Vec3::ZERO, 0.0);
        let partner = body(3, 0, Vec3::new(8.0, 0.0, 8.0), 0.0);
        let view = view_of(vec![
            me.clone(),
            body(1, 1, Vec3::ZERO, 0.0),
            body(2, 0, Vec3::ZERO, 0.0),
            partner,
        ]);
        let mut team = TeamKnowledge::default();
        team.danger.push(DangerSpot {
            at: Vec3::new(8.0, 0.0, 8.0),
            time: 1_000,
            victim: 3,
        });
        let aim = AimTuning::default();
        let memory = Memory::default();
        let mut reflex = Reflex::default();
        let mut rng = Rng::new(12);
        let sit = Situation {
            team: Some(&team),
            partner: Some(3),
            ..Situation::of(&view)
        };
        reflex.think_with(&me, &sit, &memory, 1_200, &aim, 0.5, &mut rng);
        let o = reflex.tick(&me, &view, &memory, 1_250, true, &aim, 0.5, &mut rng);
        assert_eq!(
            o.look_at
                .map(|p| p.distance_xz(Vec3::new(8.0, 0.0, 8.0)) < 0.1),
            Some(true)
        );
        assert_ne!(o.keys & key::AIM, 0);
        assert!(!o.fire);
        // Another teammate's death is not a trade; after `trade_ms` it is
        // over.
        let o = reflex.tick(&me, &view, &memory, 4_100, true, &aim, 0.5, &mut rng);
        assert!(o.look_at.is_none() && o.keys == 0, "{o:?}");
        let sit = Situation {
            partner: Some(2),
            ..sit
        };
        let mut reflex = Reflex::default();
        reflex.think_with(&me, &sit, &memory, 1_200, &aim, 0.5, &mut rng);
        assert!(
            reflex
                .tick(&me, &view, &memory, 1_250, true, &aim, 0.5, &mut rng)
                .look_at
                .is_none()
        );
    }

    /// Ticks a duel with the enemy always seen and the view on him; returns
    /// per tick (time, fire, reload pressed, legs' why).
    fn duel(
        sit: &Situation,
        me: &mut PlaceView,
        enemy_at: Vec3,
        aim: &AimTuning,
        from: u32,
        until: u32,
        seen_until: u32,
    ) -> Vec<(u32, bool, bool, Option<LegsWhy>)> {
        let mut memory = Memory::default();
        let mut reflex = Reflex::default();
        let mut rng = Rng::new(21);
        let mut out = Vec::new();
        let chest = enemy_at + Vec3::new(0.0, 1.3, 0.0);
        let mut now = from;
        while now < until {
            let seen = if now < seen_until {
                vec![percept(1, enemy_at, now, Source::Seen)]
            } else {
                vec![]
            };
            sense(&mut memory, &seen, now);
            if (now - from).is_multiple_of(250) {
                reflex.think_with(me, sit, &memory, now, aim, 0.5, &mut rng);
            }
            facing(me, chest);
            let o = reflex.tick(me, sit.view, &memory, now, true, aim, 0.5, &mut rng);
            out.push((
                now,
                o.fire,
                o.keys & key::RELOAD != 0,
                o.legs.map(|l| l.why),
            ));
            now += 50;
        }
        out
    }

    #[test]
    fn it_reloads_after_about_a_clip_and_holds_fire_while_reloading() {
        let enemy_at = Vec3::new(0.0, 0.0, 20.0);
        let mut me = body(0, 0, Vec3::ZERO, 0.0);
        let view = view_of(vec![me.clone(), body(1, 1, enemy_at, 0.0)]);
        let aim = AimTuning {
            fire_within_deg: 180.0,
            ..AimTuning::default()
        };
        let c = &aim.combat;
        // No map: no cover to find, so it reloads where it stands.
        let ticks = duel(
            &Situation::of(&view),
            &mut me,
            enemy_at,
            &aim,
            1_000,
            30_000,
            30_000,
        );
        let mut fired = 0;
        let mut reloads = Vec::new();
        for &(now, fire, reload, _) in &ticks {
            if reload {
                assert!(fired >= c.reload_fire_ms.0, "{fired} ms of fire at {now}");
                reloads.push(now);
                fired = 0;
            }
            if fire {
                fired += 50;
                assert!(
                    fired <= c.reload_fire_ms.1 + 300,
                    "{fired} ms without a reload"
                );
            }
        }
        assert!(reloads.len() >= 2, "{reloads:?}");
        // No shot for `reload_ms` after each press.
        for at in &reloads {
            assert!(
                ticks
                    .iter()
                    .all(|&(now, fire, _, _)| !(fire && now > *at && now < at + c.reload_ms)),
                "fired while reloading after {at}"
            );
        }
    }

    #[test]
    fn it_breaks_sight_before_reloading_and_reloads_tactically_when_quiet() {
        let tactics = Tactics::new(crate::v026g::bots::tactics::tests::field(), 1.0);
        let enemy_at = Vec3::new(55.0, 0.0, 30.0);
        let mut me = body(0, 0, Vec3::new(32.0, 0.0, 28.0), 0.0);
        let view = view_of(vec![me.clone(), body(1, 1, enemy_at, 0.0)]);
        let aim = AimTuning {
            fire_within_deg: 180.0,
            ..AimTuning::default()
        };
        let sit = Situation {
            tactics: Some(&tactics),
            side: Some(0),
            ..Situation::of(&view)
        };
        // Empty with the target in sight: it heads for cover, and the
        // reload comes once it is breaking sight.
        let ticks = duel(&sit, &mut me, enemy_at, &aim, 1_000, 12_000, 12_000);
        let first = ticks.iter().find(|t| t.2).expect("reloads");
        assert_eq!(first.3, Some(LegsWhy::BreakSight), "{first:?}");
        // A little fire, then nothing seen: a tactical reload after
        // `reload_idle_ms`, once.
        let mut me = body(0, 0, Vec3::ZERO, 0.0);
        let enemy_at = Vec3::new(0.0, 0.0, 20.0);
        let view = view_of(vec![me.clone(), body(1, 1, enemy_at, 0.0)]);
        let ticks = duel(
            &Situation::of(&view),
            &mut me,
            enemy_at,
            &aim,
            1_000,
            12_000,
            4_500,
        );
        let fired = ticks.iter().filter(|t| t.1).count() as u32 * 50;
        assert!(
            fired >= aim.combat.reload_some_fire_ms && fired < aim.combat.reload_fire_ms.0,
            "{fired}"
        );
        let reloads: Vec<u32> = ticks.iter().filter(|t| t.2).map(|t| t.0).collect();
        assert_eq!(reloads.len(), 1, "{reloads:?}");
        assert!(
            reloads[0] >= 4_500 + aim.combat.reload_idle_ms - 500,
            "{reloads:?}"
        );
    }

    #[test]
    fn the_matchup_keeps_each_weapon_at_its_range() {
        let c = CombatTuning::default();
        assert_eq!(preferred_band(None, Some(WeaponKind::Rifle), &c), None);
        assert_eq!(
            preferred_band(Some(WeaponKind::Shotgun), None, &c),
            Some(c.band_shotgun)
        );
        assert_eq!(
            preferred_band(Some(WeaponKind::Rifle), Some(WeaponKind::Shotgun), &c),
            Some((c.keep_from_shotgun_m, c.band_rifle.1))
        );
        assert_eq!(
            preferred_band(Some(WeaponKind::Rifle), Some(WeaponKind::Sniper), &c),
            Some((c.band_rifle.0, c.close_on_sniper_m))
        );
        // Its own weapon wins when the enemy's leaves no band.
        assert_eq!(
            preferred_band(Some(WeaponKind::Shotgun), Some(WeaponKind::Shotgun), &c),
            Some(c.band_shotgun)
        );
        assert_eq!(WeaponKind::of_role("rusher"), Some(WeaponKind::Shotgun));
        assert_eq!(WeaponKind::of_role("camper"), Some(WeaponKind::Sniper));
        assert_eq!(
            WeaponKind::of_item("wpn_aslt", 13),
            Some(WeaponKind::Sniper)
        );
        assert_eq!(
            WeaponKind::of_item("wpn_dbrl", 14),
            Some(WeaponKind::Shotgun)
        );
        assert_eq!(WeaponKind::of_item("wpn_smg", 9), Some(WeaponKind::Smg));
        assert_eq!(WeaponKind::of_item("wpn_aslt", 8), Some(WeaponKind::Rifle));

        // A shotgun bot facing a rifle at 30 m steps in; a sniper bot at
        // 10 m steps back.
        let tactics = Tactics::new(crate::v026g::bots::tactics::tests::field(), 1.0);
        let aim = AimTuning::default();
        for (kind, enemy_x, closer) in [
            (WeaponKind::Shotgun, 30.0, true),
            (WeaponKind::Sniper, 10.0, false),
        ] {
            let me = body(0, 0, Vec3::new(20.0, 0.0, 32.0), 0.0);
            let enemy_at = Vec3::new(20.0 + enemy_x, 0.0, 32.0);
            let view = view_of(vec![me.clone(), body(1, 1, enemy_at, 0.0)]);
            let weapons = [Some(kind), Some(WeaponKind::Rifle)];
            let sit = Situation {
                tactics: Some(&tactics),
                side: Some(0),
                weapon: Some(kind),
                weapons: &weapons,
                ..Situation::of(&view)
            };
            let mut memory = Memory::default();
            let mut reflex = Reflex::default();
            let mut rng = Rng::new(13);
            for now in [1_000, 1_300] {
                sense(&mut memory, &[percept(1, enemy_at, now, Source::Seen)], now);
                reflex.think_with(&me, &sit, &memory, now, &aim, 0.5, &mut rng);
            }
            let o = reflex.tick(&me, &view, &memory, 1_350, true, &aim, 0.5, &mut rng);
            assert_eq!(o.band_m, Some(c.band(kind)));
            let legs = o.legs.expect("steps");
            assert_eq!(legs.why, LegsWhy::Range);
            let d = legs.at.distance_xz(enemy_at);
            assert_eq!(d < enemy_x, closer, "{kind:?}: {d}");
        }
    }
}
