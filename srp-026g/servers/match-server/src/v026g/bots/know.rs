//! What a bot and its team know (`docs/notes/player_bots_research.md` B1,
//! "Knowledge (Brewer)"): every enemy fact carries the sense it came by
//! ([`Source`]), so the fairness rules of `player_bots_behaviour.md` §2 are
//! rules on types:
//! - the motor aims only at a [`SeenTrack`], which only [`Memory`] makes,
//!   and only from a sight older than the noticing delay: "never aim at
//!   what no sense gave" and "never react faster than noticing" cannot be
//!   broken by a play;
//! - a teammate's word ([`Source::Told`]) and the cheats
//!   ([`Source::Cheat`]) steer routes and where to look, never a shot;
//! - an enemy not sensed for a while becomes a last known position that
//!   fades, then is dropped.

use mapdata::Vec3;

use super::view::{BatteryState, MatchView, Place};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CheatKind {
    Thief,
    Radar,
}

/// How a fact came to the bot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Seen,
    Heard,
    /// A bot teammate's callout.
    Told {
        by: Place,
    },
    /// What a human's HUD shows (the kill feed, markers).
    Hud,
    Cheat(CheatKind),
}

impl Source {
    /// How much the bot trusts a report of this kind: its own sight over
    /// its own hearing over a teammate's word (and the HUD) over a cheat.
    pub fn strength(self) -> u8 {
        match self {
            Self::Seen => 3,
            Self::Heard => 2,
            Self::Told { .. } | Self::Hud => 1,
            Self::Cheat(_) => 0,
        }
    }

    /// The team's own senses: a bot's sight or hearing, a teammate's word.
    pub fn own(self) -> bool {
        matches!(self, Self::Seen | Self::Heard | Self::Told { .. })
    }

    /// A track of this kind sensed within this, ms, is fresh: a weaker
    /// report does not replace it. A sight for a moment (as before: a heard
    /// shot from where he went is better than a sight half a second old),
    /// the rest for [`FRESH_MS`].
    pub fn fresh_ms(self) -> u32 {
        match self {
            Self::Seen => 500,
            _ => FRESH_MS,
        }
    }
}

/// A heard or told track sensed within this, ms, is fresh: a weaker report
/// does not replace it (a teammate's callout does not wipe what the bot
/// heard itself a moment ago; play 6's heard shot is as long).
pub const FRESH_MS: u32 = 1_500;

/// A sense's report: who, where (with its error), when.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Percept {
    pub who: Place,
    pub at: Vec3,
    pub error_m: f32,
    pub time: u32,
    pub source: Source,
    /// Seen moving or firing: noticed sooner.
    pub busy: bool,
}

/// What a bot knows of one enemy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Track {
    pub who: Place,
    pub at: Vec3,
    pub error_m: f32,
    /// When it was last sensed.
    pub time: u32,
    pub source: Source,
    /// Since when it has been in sight without a break (`Seen` only).
    pub sight_since: Option<u32>,
    /// The noticing delay for this sight, ms.
    pub notice_ms: u32,
    /// The last place the bot's or its team's own senses gave (seen,
    /// heard, told) and when: a cheat's place does not count.
    pub own: Option<(Vec3, u32)>,
}

/// An enemy seen, and noticed: the only thing the motor may aim at. Made
/// only by [`Memory::seen`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeenTrack(Track);

impl SeenTrack {
    pub fn track(&self) -> &Track {
        &self.0
    }

    pub fn who(&self) -> Place {
        self.0.who
    }

    pub fn at(&self) -> Vec3 {
        self.0.at
    }
}

/// One bot's memory of the enemies.
#[derive(Clone, Debug, Default)]
pub struct Memory {
    tracks: Vec<Track>,
}

impl Memory {
    /// Takes in a think's percepts. `notice_ms(percept)` is the noticing
    /// delay for a new sight; `forget_ms` and `drop_ms` the fading.
    pub fn update(
        &mut self,
        percepts: &[Percept],
        now: u32,
        notice_ms: impl Fn(&Percept) -> u32,
        forget_ms: u32,
        drop_ms: u32,
    ) {
        for percept in percepts {
            let notice = notice_ms(percept);
            match self.tracks.iter_mut().find(|t| t.who == percept.who) {
                Some(track) => {
                    let seen = percept.source == Source::Seen;
                    // A report replaces the place only if it is no older
                    // than the track and not weaker than a fresh one: a
                    // teammate's callout (sensed a second or more ago) does
                    // not wipe the bot's own hearing, a cheat not a
                    // callout, and nothing a sight.
                    let fresh = track.time + track.source.fresh_ms() >= now;
                    let weaker = percept.source.strength() < track.source.strength();
                    let replaces = percept.time >= track.time && !(fresh && weaker);
                    if replaces {
                        track.at = percept.at;
                        track.error_m = percept.error_m;
                        track.source = percept.source;
                    }
                    if percept.source.own()
                        && track.own.is_none_or(|(_, time)| percept.time >= time)
                    {
                        track.own = Some((percept.at, percept.time));
                    }
                    if seen {
                        // A sight goes on if the last one was a moment ago.
                        let continuing =
                            track.sight_since.is_some() && track.time + 400 >= percept.time;
                        if !continuing {
                            track.sight_since = Some(percept.time);
                            track.notice_ms = notice;
                        }
                    } else if track.time + 400 < percept.time {
                        track.sight_since = None;
                    }
                    // When it was last sensed by a report it took: a weaker
                    // one it did not take keeps no sight alive (a cheat's
                    // place of a thief is no sight of him).
                    if replaces {
                        track.time = percept.time;
                    }
                }
                None => self.tracks.push(Track {
                    who: percept.who,
                    at: percept.at,
                    error_m: percept.error_m,
                    time: percept.time,
                    source: percept.source,
                    sight_since: (percept.source == Source::Seen).then_some(percept.time),
                    notice_ms: notice,
                    own: percept.source.own().then_some((percept.at, percept.time)),
                }),
            }
        }
        for track in &mut self.tracks {
            // Out of sight now.
            if track.sight_since.is_some() && track.time + 400 < now {
                track.sight_since = None;
            }
            // Not sensed for a while: a last known position, fading.
            if track.time + forget_ms < now {
                track.error_m += 0.5;
            }
        }
        self.tracks.retain(|track| track.time + drop_ms >= now);
    }

    /// The enemy at `who` seen now and noticed.
    pub fn seen(&self, who: Place, now: u32) -> Option<SeenTrack> {
        let track = self.tracks.iter().find(|t| t.who == who)?;
        let since = track.sight_since?;
        (track.source == Source::Seen && now >= since + track.notice_ms && track.time + 400 >= now)
            .then_some(SeenTrack(*track))
    }

    /// Every enemy seen now and noticed.
    pub fn all_seen(&self, now: u32) -> Vec<SeenTrack> {
        self.tracks
            .iter()
            .filter_map(|track| self.seen(track.who, now))
            .collect()
    }

    /// Everything known, however it came and however old.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// An enemy died: it is not where it was.
    pub fn forget(&mut self, who: Place) {
        self.tracks.retain(|track| track.who != who);
    }
}

/// A teammate's death at a spot: danger there for a while (and a trade to
/// make).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DangerSpot {
    pub at: Vec3,
    pub time: u32,
    pub victim: Place,
}

/// A battery of ours taken.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theft {
    pub battery: usize,
    /// Where it was last known.
    pub at: Vec3,
    pub time: u32,
    pub thief: Option<Place>,
}

/// Where a team believes a battery lies (the fair battery knowledge,
/// `player_bots_behaviour.md` §2), used with the battery cheat off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnownBattery {
    pub at: Vec3,
    /// When it was last put right.
    pub time: u32,
    /// Shown on the team's HUD: it follows the battery while it lies about
    /// and goes when the game takes it off the HUD. Else seen by a bot of
    /// the team, and left where it was seen until a bot sees that spot
    /// empty.
    pub hud: bool,
}

/// A teammate's death (the kill feed), kept longer than a danger spot for
/// the spawners' search: a spot where the team keeps dying is worth less.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loss {
    pub at: Vec3,
    pub time: u32,
    /// The spawner the bot was going to look at, if it was.
    pub spawner: Option<usize>,
}

/// Losses are kept this long, ms.
pub const LOSS_KEEP_MS: u32 = 120_000;

/// An enemy a bot of the team saw carrying a battery (a carrier holds it in
/// his hands, in sight).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarrierSeen {
    pub who: Place,
    pub battery: usize,
    pub at: Vec3,
    pub time: u32,
}

/// Seen carriers are kept this long, ms.
pub const CARRIER_SEEN_KEEP_MS: u32 = 60_000;

/// A battery an enemy carrier dropped where he died, unseen: the kill feed
/// says he died, the team knew he carried it (a theft of ours, or seen
/// carrying), and the game's HUD shows the dropped battery only to his
/// team. A human walks to where he last saw him; the bots look there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookSpot {
    pub battery: usize,
    /// His last place the team itself sensed (the killer's sight, else
    /// its last own sight, hearing or callout of him).
    pub at: Vec3,
    pub time: u32,
}

/// A look spot nobody has looked at is dropped after this, ms.
pub const LOOK_KEEP_MS: u32 = 120_000;

/// What a team's bots share: callouts (one bot's sense, the others' a
/// moment later), danger spots, thefts, and with the battery cheat off
/// what they know of the batteries and the spawners they have looked at.
#[derive(Clone, Debug, Default)]
pub struct TeamKnowledge {
    /// Callouts: (when the others know it, the percept, the places that
    /// took it already as a bit mask).
    callouts: Vec<(u32, Percept, u64)>,
    pub danger: Vec<DangerSpot>,
    pub thefts: Vec<Theft>,
    /// By battery number.
    pub batteries: Vec<Option<KnownBattery>>,
    /// By battery spawner: when a bot of the team last saw its spot.
    pub spawners_seen: Vec<Option<u32>>,
    /// The team's bots checking spawners now and the spawner each looks
    /// at (the commander's, from the plays it handed out).
    pub searching: Vec<(Place, usize)>,
    /// The team's deaths of the last [`LOSS_KEEP_MS`].
    pub losses: Vec<Loss>,
    /// Enemies seen carrying lately.
    pub carriers_seen: Vec<CarrierSeen>,
    /// Where enemy carriers died with a battery the team cannot see.
    pub looks: Vec<LookSpot>,
}

impl TeamKnowledge {
    /// A bot of the team sensed something: its teammates know it after
    /// `delay_ms`. Only first-hand senses are called out.
    pub fn call_out(&mut self, by: Place, percept: &Percept, delay_ms: u32) {
        if !matches!(percept.source, Source::Seen | Source::Heard) {
            return;
        }
        self.callouts.push((
            percept.time + delay_ms,
            Percept {
                source: Source::Told { by },
                ..*percept
            },
            0,
        ));
    }

    /// The callouts the bot at `me` hears by `now` (not its own), each
    /// once (handed back at every think, a callout came again with its old
    /// time and place over what the bot sensed since), and the old ones
    /// dropped.
    pub fn heard_by(&mut self, me: Place, now: u32) -> Vec<Percept> {
        self.callouts.retain(|(at, _, _)| *at + 3_000 >= now);
        // Places past the mask's 64 take a callout at every think, as
        // before (the memory takes none older than what it knows).
        let bit = (me < 64).then(|| 1u64 << me);
        let mut out = Vec::new();
        for (at, percept, taken) in &mut self.callouts {
            if *at > now
                || percept.source == (Source::Told { by: me })
                || bit.is_some_and(|bit| *taken & bit != 0)
            {
                continue;
            }
            *taken |= bit.unwrap_or(0);
            out.push(*percept);
        }
        out
    }

    /// Danger spots of the last `ms`.
    pub fn danger_within(&self, now: u32, ms: u32) -> impl Iterator<Item = &DangerSpot> {
        self.danger.iter().filter(move |spot| spot.time + ms >= now)
    }

    /// The player at `who` left the match: no callout about it is heard,
    /// and no theft names it. The battery it took is still where it was
    /// last known.
    pub fn forget(&mut self, who: Place) {
        self.forget_callouts(who);
        for theft in &mut self.thefts {
            if theft.thief == Some(who) {
                theft.thief = None;
            }
        }
    }

    /// The enemy at `who` died: no callout about him is heard any more (he
    /// is not where he was; a callout of the moment before his death would
    /// bring him back to the memories that forgot him).
    pub fn forget_callouts(&mut self, who: Place) {
        self.callouts.retain(|(_, percept, _)| percept.who != who);
    }

    /// Where the team believes battery `battery` lies.
    pub fn battery(&self, battery: usize) -> Option<&KnownBattery> {
        self.batteries.get(battery)?.as_ref()
    }

    /// Battery `battery` lies at `at` as far as the team knows (`None`:
    /// it does not know).
    pub fn set_battery(&mut self, battery: usize, known: Option<KnownBattery>) {
        if self.batteries.len() <= battery {
            if known.is_none() {
                return;
            }
            self.batteries.resize(battery + 1, None);
        }
        self.batteries[battery] = known;
    }

    /// The batteries on the HUD of team `team`, from two worlds' states
    /// (`player_bots_behaviour.md` §2). The client (PROVEN in 0.21d's
    /// `gather_victory_items_rule::fill_match_hud_state` 0x1FBDF4 and
    /// `victory_item_core::initialize`/`deactivate`, the same in 0.26g's
    /// 0x3140C0, 0x300BEA, 0x30104E, 0x3010A4) shows a lying battery to a
    /// team whose bit is set in the battery's mask; the mask is empty when
    /// the battery appears, set to the picker's team alone on a pickup, to
    /// the carrier's team alone on a drop (a death too), emptied when it is
    /// put in, and a player's spot adds his team (not modelled here). So:
    /// - a battery dropped by our carrier is on our HUD where it lies;
    /// - one our HUD showed goes from it when anybody picks it up, when it
    ///   moved between the worlds unseen, or when it is put in;
    /// - one our teammate carries is known by him (the HUD shows him);
    /// - one only seen by a bot stays where it was seen.
    pub fn hud_update(&mut self, team: u8, before: &MatchView, after: &MatchView, time: u32) {
        let team_of = |carrier: Option<Place>| carrier.and_then(|p| after.place(p)).map(|p| p.team);
        for (battery, now) in after.batteries.iter().enumerate() {
            let was = before.batteries.get(battery).copied();
            let known = self.battery(battery).copied();
            match *now {
                BatteryState::Free(at) => {
                    let hud = match was {
                        // Lying still: as it was.
                        Some(BatteryState::Free(then)) if then.distance_xz(at) <= 1.0 => {
                            known.is_some_and(|k| k.hud)
                        }
                        Some(BatteryState::Carried(carrier)) => team_of(carrier) == Some(team),
                        // New, moved unseen, or out of a container.
                        _ => false,
                    };
                    if hud {
                        self.set_battery(battery, Some(KnownBattery { at, time, hud }));
                    } else if known.is_some_and(|k| k.hud) {
                        self.set_battery(battery, None);
                    }
                }
                BatteryState::Carried(carrier) => {
                    if known.is_some_and(|k| k.hud) || team_of(carrier) == Some(team) {
                        self.set_battery(battery, None);
                    }
                }
                BatteryState::Home(_) => {
                    if known.is_some_and(|k| k.hud) {
                        self.set_battery(battery, None);
                    }
                }
            }
        }
    }

    /// When a bot of the team last saw spawner `spawner`'s spot.
    pub fn spawner_seen(&self, spawner: usize) -> Option<u32> {
        self.spawners_seen.get(spawner).copied().flatten()
    }

    pub fn saw_spawner(&mut self, spawner: usize, now: u32) {
        if self.spawners_seen.len() <= spawner {
            self.spawners_seen.resize(spawner + 1, None);
        }
        self.spawners_seen[spawner] = Some(now);
    }

    pub fn forget_old(&mut self, now: u32) {
        self.danger.retain(|spot| spot.time + 30_000 >= now);
        self.thefts.retain(|theft| theft.time + 60_000 >= now);
        self.losses.retain(|loss| loss.time + LOSS_KEEP_MS >= now);
        self.carriers_seen
            .retain(|seen| seen.time + CARRIER_SEEN_KEEP_MS >= now);
        self.looks.retain(|look| look.time + LOOK_KEEP_MS >= now);
    }

    /// A bot of the team saw the enemy at `who` carrying `battery`.
    pub fn saw_carrying(&mut self, who: Place, battery: usize, at: Vec3, time: u32) {
        self.carriers_seen.retain(|seen| seen.who != who);
        self.carriers_seen.push(CarrierSeen {
            who,
            battery,
            at,
            time,
        });
    }

    /// Whether the team knows the enemy at `who` carried `battery`: he
    /// stole it from us (the HUD's theft), or a bot saw him carrying it.
    pub fn knew_carrying(&self, who: Place, battery: usize) -> bool {
        self.thefts
            .iter()
            .any(|t| t.battery == battery && t.thief == Some(who))
            || self
                .carriers_seen
                .iter()
                .any(|s| s.who == who && s.battery == battery)
    }

    /// Where the team looks for `battery`, dropped by a carrier it lost.
    pub fn add_look(&mut self, look: LookSpot) {
        self.looks.retain(|l| l.battery != look.battery);
        self.looks.push(look);
    }

    /// The look spots still worth a look for team `team`: not the ones
    /// whose battery the team knows of now (seen lying, on its HUD, home,
    /// or carried by a teammate).
    pub fn settle_looks(&mut self, team: u8, view: &MatchView) {
        if self.looks.is_empty() {
            return;
        }
        let known: Vec<bool> = (0..view.batteries.len())
            .map(|b| {
                self.battery(b).is_some()
                    || match view.batteries[b] {
                        BatteryState::Home(_) => true,
                        BatteryState::Carried(Some(p)) => {
                            view.place(p).is_some_and(|p| p.team == team)
                        }
                        _ => false,
                    }
            })
            .collect();
        self.looks
            .retain(|look| !known.get(look.battery).copied().unwrap_or(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(who: Place, time: u32) -> Percept {
        Percept {
            who,
            at: Vec3::new(10.0, 0.0, 0.0),
            error_m: 0.0,
            time,
            source: Source::Seen,
            busy: false,
        }
    }

    #[test]
    fn a_sight_is_a_seen_track_only_after_the_noticing_delay() {
        let mut memory = Memory::default();
        memory.update(&[seen(3, 1_000)], 1_000, |_| 300, 3_000, 20_000);
        assert!(memory.seen(3, 1_000).is_none());
        assert!(memory.seen(3, 1_200).is_none());
        memory.update(&[seen(3, 1_300)], 1_300, |_| 300, 3_000, 20_000);
        assert_eq!(memory.seen(3, 1_300).map(|t| t.who()), Some(3));
        // Out of sight: no seen track, but it is still known.
        memory.update(&[], 2_000, |_| 300, 3_000, 20_000);
        assert!(memory.seen(3, 2_000).is_none());
        assert_eq!(memory.tracks().len(), 1);
        // Seen again: a new noticing delay.
        memory.update(&[seen(3, 2_500)], 2_500, |_| 300, 3_000, 20_000);
        assert!(memory.seen(3, 2_600).is_none());
        // A heard report does not make a seen track, and old ones go.
        let heard = Percept {
            source: Source::Heard,
            ..seen(4, 3_000)
        };
        memory.update(&[heard], 3_000, |_| 300, 3_000, 20_000);
        assert!(memory.seen(4, 9_000).is_none());
        memory.update(&[], 30_000, |_| 300, 3_000, 20_000);
        assert!(memory.tracks().is_empty());
    }

    fn battery_view(batteries: Vec<BatteryState>) -> MatchView {
        use crate::v026g::bots::view::{Mode, PlaceView};
        let place = |place: Place, team: u8| PlaceView {
            place,
            team,
            bot: true,
            present: true,
            alive: true,
            feet: Vec3::ZERO,
            yaw: 0.0,
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
        };
        MatchView {
            clock: 10_000,
            mode: Mode::Batteries,
            time_left_s: 600.0,
            total_s: 900.0,
            points: [0, 0],
            places: vec![place(0, 0), place(1, 1)],
            batteries,
            containers: vec![],
            sounds: vec![],
            deaths: vec![],
            world_time: 10_000,
            can_respawn: [true, true],
            next_battery: None,
        }
    }

    /// The client's HUD mask (`hud_update`): a battery that appears is on
    /// no HUD; one our carrier dropped is on ours where it lies, not on
    /// theirs; picked up by them it leaves our HUD, and put in it is on
    /// none. A battery only seen stays where it was seen.
    #[test]
    fn the_hud_shows_what_the_team_last_carried_and_drops_it_when_taken() {
        let (a, b) = (Vec3::new(10.0, 0.0, 10.0), Vec3::new(20.0, 0.0, 5.0));
        let mut ours = TeamKnowledge::default();
        let mut theirs = TeamKnowledge::default();
        let step = |ours: &mut TeamKnowledge, theirs: &mut TeamKnowledge, from, to| {
            let (before, after) = (battery_view(from), battery_view(to));
            ours.hud_update(0, &before, &after, 1);
            theirs.hud_update(1, &before, &after, 1);
        };
        // It appears: nobody's HUD.
        step(&mut ours, &mut theirs, vec![], vec![BatteryState::Free(a)]);
        assert!(ours.battery(0).is_none() && theirs.battery(0).is_none());
        // Our carrier (place 0) takes it and drops it at b (he died).
        step(
            &mut ours,
            &mut theirs,
            vec![BatteryState::Free(a)],
            vec![BatteryState::Carried(Some(0))],
        );
        step(
            &mut ours,
            &mut theirs,
            vec![BatteryState::Carried(Some(0))],
            vec![BatteryState::Free(b)],
        );
        assert_eq!(ours.battery(0).map(|k| (k.at, k.hud)), Some((b, true)));
        assert!(theirs.battery(0).is_none());
        // Lying still: still on our HUD.
        step(
            &mut ours,
            &mut theirs,
            vec![BatteryState::Free(b)],
            vec![BatteryState::Free(b)],
        );
        assert!(ours.battery(0).is_some_and(|k| k.hud));
        // Their player takes it: off our HUD; dropped, on theirs.
        step(
            &mut ours,
            &mut theirs,
            vec![BatteryState::Free(b)],
            vec![BatteryState::Carried(Some(1))],
        );
        assert!(ours.battery(0).is_none());
        step(
            &mut ours,
            &mut theirs,
            vec![BatteryState::Carried(Some(1))],
            vec![BatteryState::Free(a)],
        );
        assert!(ours.battery(0).is_none());
        assert!(theirs.battery(0).is_some_and(|k| k.hud && k.at == a));
        // Put in: on no HUD.
        step(
            &mut ours,
            &mut theirs,
            vec![BatteryState::Free(a)],
            vec![BatteryState::Home(1)],
        );
        assert!(theirs.battery(0).is_none());
        // Seen by one of our bots, then taken by them: still believed
        // where it was seen (no HUD said otherwise); taken by our own
        // teammate, known to be his.
        ours.set_battery(
            1,
            Some(KnownBattery {
                at: a,
                time: 1,
                hud: false,
            }),
        );
        let lying = vec![BatteryState::Home(1), BatteryState::Free(a)];
        let taken = vec![BatteryState::Home(1), BatteryState::Carried(Some(1))];
        step(&mut ours, &mut theirs, lying.clone(), taken);
        assert_eq!(ours.battery(1).map(|k| k.at), Some(a));
        let ours_taken = vec![BatteryState::Home(1), BatteryState::Carried(Some(0))];
        step(&mut ours, &mut theirs, lying, ours_taken);
        assert!(ours.battery(1).is_none());
    }

    #[test]
    fn callouts_reach_teammates_later_and_never_cheats() {
        let mut team = TeamKnowledge::default();
        team.call_out(1, &seen(5, 1_000), 1_000);
        let cheat = Percept {
            source: Source::Cheat(CheatKind::Radar),
            ..seen(6, 1_000)
        };
        team.call_out(1, &cheat, 1_000);
        assert!(team.heard_by(2, 1_500).is_empty());
        let heard = team.heard_by(2, 2_000);
        assert_eq!(heard.len(), 1);
        assert_eq!(heard[0].source, Source::Told { by: 1 });
        assert!(team.heard_by(1, 2_000).is_empty(), "not its own");
    }

    /// Match 0x5402a730: the human's track in the enemy bots' memories was
    /// told 252 times, heard 3. A teammate's callout (older, or weaker
    /// than a fresh report) does not replace what the bot heard itself; a
    /// cheat does not replace a fresh callout; a stale report gives way to
    /// a newer one of any kind.
    #[test]
    fn a_callout_does_not_wipe_the_bots_own_hearing() {
        let at = |x: f32| Vec3::new(x, 0.0, 0.0);
        let report = |source, x, time| Percept {
            source,
            at: at(x),
            ..seen(7, time)
        };
        let told = Source::Told { by: 2 };
        let mut memory = Memory::default();
        let update = |memory: &mut Memory, p: Percept, now| {
            memory.update(&[p], now, |_| 300, 3_000, 20_000);
            let t = memory.tracks()[0];
            (t.source, t.at.x, t.time)
        };
        assert_eq!(
            update(&mut memory, report(Source::Heard, 1.0, 5_000), 5_000),
            (Source::Heard, 1.0, 5_000)
        );
        // A callout of a moment before (its old time and place): kept.
        assert_eq!(
            update(&mut memory, report(told, 2.0, 4_200), 5_200),
            (Source::Heard, 1.0, 5_000)
        );
        // A callout newer but weaker while the hearing is fresh: kept.
        assert_eq!(
            update(&mut memory, report(told, 3.0, 5_100), 5_400),
            (Source::Heard, 1.0, 5_000)
        );
        // Heard again: the newer hearing.
        assert_eq!(
            update(&mut memory, report(Source::Heard, 4.0, 5_500), 5_500),
            (Source::Heard, 4.0, 5_500)
        );
        // The hearing stale (1.5 s): a newer callout takes it.
        assert_eq!(
            update(&mut memory, report(told, 5.0, 6_100), 7_100),
            (told, 5.0, 6_100)
        );
        // A cheat does not replace a fresh callout, then does.
        let radar = Source::Cheat(CheatKind::Radar);
        assert_eq!(
            update(&mut memory, report(radar, 6.0, 7_200), 7_200),
            (told, 5.0, 6_100)
        );
        assert_eq!(
            update(&mut memory, report(radar, 6.0, 7_700), 7_700),
            (radar, 6.0, 7_700)
        );
        // A sight replaces anything; a cheat's newer place keeps no sight
        // alive.
        assert_eq!(
            update(&mut memory, report(Source::Seen, 8.0, 7_800), 7_800),
            (Source::Seen, 8.0, 7_800)
        );
        update(&mut memory, report(Source::Seen, 8.0, 8_100), 8_100);
        assert!(memory.seen(7, 8_150).is_some());
        let thief = Source::Cheat(CheatKind::Thief);
        update(&mut memory, report(thief, 9.0, 8_300), 8_300);
        assert!(memory.seen(7, 8_550).is_none(), "no sight since 8.1 s");
    }

    #[test]
    fn a_callout_is_heard_once_and_a_dead_enemys_not_at_all() {
        let mut team = TeamKnowledge::default();
        team.call_out(1, &seen(5, 1_000), 1_000);
        team.call_out(1, &seen(6, 1_000), 1_000);
        assert_eq!(team.heard_by(2, 2_000).len(), 2);
        assert!(team.heard_by(2, 2_200).is_empty(), "taken already");
        assert_eq!(team.heard_by(3, 2_200).len(), 2, "another bot's own");
        team.forget_callouts(5);
        let left: Vec<Place> = team.heard_by(4, 2_300).iter().map(|p| p.who).collect();
        assert_eq!(left, vec![6]);
    }
}
