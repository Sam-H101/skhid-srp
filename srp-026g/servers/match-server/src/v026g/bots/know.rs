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

use super::view::Place;

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
                    // A better or newer report replaces the place; a vague
                    // one does not overwrite a fresh sight.
                    let fresh_sight =
                        track.source == Source::Seen && track.time + 500 >= percept.time;
                    if seen || !fresh_sight {
                        track.at = percept.at;
                        track.error_m = percept.error_m;
                        track.source = percept.source;
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
                    track.time = track.time.max(percept.time);
                }
                None => self.tracks.push(Track {
                    who: percept.who,
                    at: percept.at,
                    error_m: percept.error_m,
                    time: percept.time,
                    source: percept.source,
                    sight_since: (percept.source == Source::Seen).then_some(percept.time),
                    notice_ms: notice,
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

/// What a team's bots share: callouts (one bot's sense, the others' a
/// moment later), danger spots, thefts.
#[derive(Clone, Debug, Default)]
pub struct TeamKnowledge {
    /// Callouts: (when the others know it, the percept).
    callouts: Vec<(u32, Percept)>,
    pub danger: Vec<DangerSpot>,
    pub thefts: Vec<Theft>,
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
        ));
    }

    /// The callouts the bot at `me` hears by `now` (not its own), and the
    /// old ones dropped.
    pub fn heard_by(&mut self, me: Place, now: u32) -> Vec<Percept> {
        self.callouts.retain(|(at, _)| *at + 3_000 >= now);
        self.callouts
            .iter()
            .filter(|(at, percept)| *at <= now && percept.source != Source::Told { by: me })
            .map(|(_, percept)| *percept)
            .collect()
    }

    /// Danger spots of the last `ms`.
    pub fn danger_within(&self, now: u32, ms: u32) -> impl Iterator<Item = &DangerSpot> {
        self.danger.iter().filter(move |spot| spot.time + ms >= now)
    }

    /// The player at `who` left the match: no callout about it is heard,
    /// and no theft names it. The battery it took is still where it was
    /// last known.
    pub fn forget(&mut self, who: Place) {
        self.callouts.retain(|(_, percept)| percept.who != who);
        for theft in &mut self.thefts {
            if theft.thief == Some(who) {
                theft.thief = None;
            }
        }
    }

    pub fn forget_old(&mut self, now: u32) {
        self.danger.retain(|spot| spot.time + 30_000 >= now);
        self.thefts.retain(|theft| theft.time + 60_000 >= now);
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
}
