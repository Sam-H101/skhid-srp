//! A match recording ([`super::record`]) made into a `.sur` replay
//! ([`super::sur`]). Offline: nothing here runs in the server.
//!
//! What goes where:
//! - the options (chunk 1): the stream of a 0xE4 the server sent, without
//!   its first byte, from the place that got the largest roster;
//! - the events (chunk 2): what the server relayed to the players (0xEB,
//!   0xEC, 0xED, 0xEE, 0xF3, 0xE9, 0xEA), every message of every place put
//!   together, a copy of an event dropped. Its tick byte is read against the
//!   match clock the recording's own times give, at the time the server sent
//!   the event: the first copy of the same message to any place (a copy to a
//!   place whose link is backed up goes out seconds later, and a tick byte
//!   read more than 6.35 s from its own time is read as another tick); an
//!   event the clock still puts in the future is dropped;
//! - the key frames (chunk 3): the worlds the clients uploaded (0xE0
//!   chunks), the same upload of several clients as one when they agree,
//!   thinned to what the client can hold;
//! - the config (chunk 4): given by the caller.
//!
//! Every choice is listed in [`Report`].

use std::collections::{BTreeMap, HashMap};

use survarium::v026g::dictionary::Dictionary;
use survarium::v026g::lzma2;
use survarium::v026g::maps;
use survarium::v026g::match_msgs::snapshot;
use survarium::v026g::match_msgs::static_info;
use survarium::v026g::match_msgs::upload::{Chunk, Pushed, Reassembler};
use survarium::v026g::match_msgs::{mask_bytes, tick_time};
use udp_transport::table::{Direction, Table};
use udp_transport::wire;

use super::record::{IN, NOTE, OUT, Record};
use super::sur::{
    self, Coding, Event, InputValue, KeyFrame, MAX_KEY_FRAMES, MAX_PLAYERS, Parts, TICK_MS, Writer,
};

/// An event the clock puts this far ahead of the recording is dropped: the
/// server itself refuses inputs more than 2.5 s ahead.
const FUTURE_SLACK_MS: u32 = 1_000;
/// A copy of a relay is a sign that its place's link is backed up when no
/// other place got the same message (id and payload) within `BEHIND_MS`
/// before it (or `NEAR_AFTER_MS` after), but another place got it within
/// `LOOKBACK_MS` before. The server sends an event to every other place at
/// once, so on sound links the copies of one event are a few hundred ms
/// apart (a resend: under 1.1 s in every sample). The same bytes come back
/// 12.8 s later at the earliest; a copy that meets them then is read as
/// that later event, which has the same value: a copy of it.
const BEHIND_MS: u32 = 2_000;
const NEAR_AFTER_MS: u32 = 1_000;
const LOOKBACK_MS: u32 = 60_000;
/// Every copy a place gets this long before or after a sign that its link
/// is backed up is dropped: the copies of a backlog more than 6 s old are
/// read as other ticks, and match nothing in the window. The other places
/// got the same events on time.
const LAG_HOLD_MS: u32 = 15_000;
/// Bit 31 of a relayed actions mask is a flag the client splits off before
/// the input goes to the world (0xEB/0xEC 0x318479, 0xED/0xEE 0x27C501);
/// the replay passes the record's u64 as it is with the flag 0 (0x1EE2A1),
/// so left in, the bit would be an action.
const INPUT_FLAG: u32 = 1 << 31;
/// The least gap between two key frames, unless the caller gives one.
const DEFAULT_KEY_FRAME_EVERY_MS: u32 = 10_000;
/// The world kept for a late joiner is the first one this long after its
/// entry: the item must be later than the entry's tick.
const LATE_JOINER_WORLD_AFTER_MS: u32 = 300;
/// How long before its entry a late joiner's install is looked for.
const INSTALL_WINDOW_MS: u32 = 60_000;

/// What the caller chooses.
#[derive(Clone, Debug)]
pub struct Settings {
    pub coding: Coding,
    /// The least gap between two key frames; larger if 128 would not do.
    pub key_frame_every_ms: u32,
    /// Key frames at these times are left out (for finding what the
    /// client chokes on).
    pub drop_key_frames: Vec<u32>,
    /// Only the first world as a key frame (the replay is never put
    /// right).
    pub first_key_frame_only: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            coding: Coding::Packed,
            key_frame_every_ms: DEFAULT_KEY_FRAME_EVERY_MS,
            drop_key_frames: Vec::new(),
            first_key_frame_only: false,
        }
    }
}

/// What the conversion found and chose.
#[derive(Debug, Default)]
pub struct Report {
    pub match_id: String,
    pub map: String,
    pub mode: u64,
    pub places: usize,
    /// Recording ms less match ms.
    pub clock_offset_ms: i64,
    pub datagrams_out: usize,
    pub datagrams_in: usize,
    pub unparsed: usize,
    /// What the transport parser said, by direction, for those.
    pub unparsed_why: BTreeMap<String, usize>,
    pub inputs: usize,
    pub input_copies: usize,
    pub input_conflicts: usize,
    pub dropped_future: usize,
    /// Relay copies dropped because their place's link was backed up, by
    /// place; and the messages none of whose copies was kept.
    pub lagging_copies: BTreeMap<u8, usize>,
    pub messages_lost: usize,
    /// Inputs whose actions mask had the flag bit (cleared).
    pub flagged_inputs: usize,
    /// The match let players in late: the masks of 0xED/0xEE are of 20.
    pub growing_roster: bool,
    pub profiles: usize,
    pub entered: usize,
    pub left: usize,
    pub roster: u8,
    /// Places that entered beyond the roster the first 0xE4 had.
    pub late_joiners: usize,
    /// `(world time, objects)`: where a late joiner's grenades and traps
    /// first are in the uploaded worlds, and how many.
    pub installs: Vec<(u32, usize)>,
    /// Late joiners whose install was not found in the worlds: the key
    /// frames before their entry are left out.
    pub installs_not_found: Vec<u8>,
    /// Key frames left out because a joiner's install was not found.
    pub dropped_unsure_key_frames: usize,
    /// Key frames given zero bytes for joiners not installed yet.
    pub padded_key_frames: usize,
    pub worlds_complete: usize,
    pub world_times: usize,
    pub worlds_disagreeing: usize,
    pub first_world_ms: Option<u32>,
    pub start_ms: u32,
    pub end_ms: u32,
    pub key_frames: Vec<u32>,
    /// The size of each key frame's parts: players, bullets, statistics,
    /// objects, physics, rules, effects (`None`: the world did not walk).
    pub key_frame_parts: Vec<(u32, Option<[usize; 7]>)>,
    /// `(time, 'E' entered / 'L' left, place)`.
    pub roster_events: Vec<(u32, char, u8)>,
    /// Inputs in each minute of the match.
    pub inputs_per_minute: Vec<usize>,
    pub body_bytes: usize,
    pub decoded_bytes: usize,
    pub notes: Vec<String>,
}

/// Why a recording cannot be converted.
#[derive(Debug)]
pub enum ConvertError {
    NoHeader,
    NoOptions,
    NoWorlds,
    Sur(sur::SurError),
    /// The file made fails its own check: a bug.
    Verify(String),
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoHeader => write!(f, "no header note"),
            Self::NoOptions => write!(f, "no 0xE4 (static info) in the recording"),
            Self::NoWorlds => write!(f, "no complete uploaded world in the recording"),
            Self::Sur(error) => write!(f, "{error}"),
            Self::Verify(why) => write!(f, "the file made does not check out: {why}"),
        }
    }
}

impl std::error::Error for ConvertError {}

impl From<sur::SurError> for ConvertError {
    fn from(error: sur::SurError) -> Self {
        Self::Sur(error)
    }
}

/// The name of the recording's mode config under `gameplay/game_modes/`
/// (`level_01_day_2.options`): the `maps_dict` row of its map id.
pub fn config_name(records: &[Record]) -> Option<String> {
    let id = u8::try_from(header(records)?.get("map_id")?.as_u64()?).ok()?;
    maps::map_mode(id).map(|row| row.config.to_string())
}

fn header(records: &[Record]) -> Option<serde_json::Value> {
    records
        .iter()
        .find(|r| r.kind == NOTE)
        .and_then(|r| serde_json::from_slice(&r.bytes).ok())
}

/// recording ms less match ms: the median over every note with both.
fn clock_offset(records: &[Record]) -> Option<i64> {
    let mut offsets: Vec<i64> = records
        .iter()
        .filter(|r| r.kind == NOTE)
        .filter_map(|r| {
            let note: serde_json::Value = serde_json::from_slice(&r.bytes).ok()?;
            let match_ms = note.get("match_ms")?.as_u64()?;
            Some(i64::from(r.at_ms) - match_ms as i64)
        })
        .collect();
    offsets.sort_unstable();
    offsets.get(offsets.len() / 2).copied()
}

/// The world of an item time that most uploads agree on.
fn majority(bodies: &[Vec<u8>]) -> (usize, usize) {
    let mut counts: Vec<(usize, usize)> = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        match counts.iter_mut().find(|(first, _)| bodies[*first] == *body) {
            Some((_, n)) => *n += 1,
            None => counts.push((i, 1)),
        }
    }
    let best = counts
        .iter()
        .max_by_key(|(_, n)| *n)
        .copied()
        .unwrap_or((0, 0));
    (best.0, counts.len())
}

#[derive(Default)]
struct PlaceState {
    /// The roster of the 0xE4 this place got.
    roster: u8,
    /// 0xE4 parts by unique id: (count, parts).
    options: HashMap<u8, (u8, BTreeMap<u8, Vec<u8>>)>,
    uploads: Reassembler,
}

/// Makes the `.sur` of a recording. `config` is the mode's `.options` file.
pub fn convert(
    records: &[Record],
    config: &[u8],
    settings: Settings,
) -> Result<(Vec<u8>, Report), ConvertError> {
    let head = header(records).ok_or(ConvertError::NoHeader)?;
    let mut report = Report {
        match_id: head["match_id"].as_str().unwrap_or("?").to_string(),
        map: head["map"].as_str().unwrap_or("?").to_string(),
        mode: head["mode"].as_u64().unwrap_or(0),
        places: head["places"].as_array().map_or(0, Vec::len),
        growing_roster: head["growing_roster"].as_bool().unwrap_or(false),
        ..Report::default()
    };
    let offset = clock_offset(records).unwrap_or(0);
    report.clock_offset_ms = offset;
    let dictionary = Dictionary::shared();

    let mut places: HashMap<u8, PlaceState> = HashMap::new();
    // The relays, by message: (id, mask bytes, payload) -> id in `sent`;
    // when and to which place each copy went, to read them once it is
    // known which places' links were backed up.
    let mut messages: HashMap<(u8, usize, Vec<u8>), usize> = HashMap::new();
    let mut sent: Vec<Vec<(u32, u8)>> = Vec::new();
    let mut copies: Vec<(u32, u8, usize)> = Vec::new();
    // Events by (time, player), the first copy kept.
    let mut inputs: BTreeMap<(u32, u8), InputValue> = BTreeMap::new();
    let mut profiles: BTreeMap<(u32, u8), u8> = BTreeMap::new();
    let mut entered: BTreeMap<(u32, u8), ()> = BTreeMap::new();
    let mut left: BTreeMap<(u32, u8), ()> = BTreeMap::new();
    let mut worlds: BTreeMap<u32, Vec<Vec<u8>>> = BTreeMap::new();
    // The 0xE4 streams (without the index byte), by roster.
    let mut options: BTreeMap<u8, Vec<u8>> = BTreeMap::new();

    for record in records {
        if record.kind != IN && record.kind != OUT {
            continue;
        }
        let place = places.entry(record.place).or_default();
        if record.kind == IN {
            report.datagrams_in += 1;
            let Ok(datagram) = wire::parse(Table::Match, Direction::ToServer, &record.bytes, 0)
            else {
                report.unparsed += 1;
                continue;
            };
            for message in &datagram.messages {
                if message.id != wire::UPLOAD_CARRIER
                    || message.part.is_some()
                    || message.payload.is_empty()
                {
                    continue;
                }
                let Ok(chunk) = Chunk::parse(message.payload) else {
                    continue;
                };
                if let Pushed::Complete(upload) = place.uploads.push(&chunk) {
                    report.worlds_complete += 1;
                    worlds
                        .entry(upload.item_time)
                        .or_default()
                        .push(upload.body);
                }
            }
            continue;
        }
        report.datagrams_out += 1;
        // A match that lets players in late sends its masks for 20 from
        // the start (`Game::snapshot_players`).
        let mask_len = if report.growing_roster {
            mask_bytes(MAX_PLAYERS as u8)
        } else if place.roster == 0 {
            0
        } else {
            mask_bytes(place.roster)
        };
        let parsed = wire::parse(Table::Match, Direction::ToClient, &record.bytes, mask_len);
        let datagram = match parsed {
            Ok(datagram) => datagram,
            Err(error) => {
                report.unparsed += 1;
                *report.unparsed_why.entry(format!("{error:?}")).or_default() += 1;
                continue;
            }
        };
        for message in &datagram.messages {
            let payload = message.payload;
            match message.id {
                0xE4 => {
                    let Some(x) = message.x else { continue };
                    let (count, parts) = place.options.entry(x).or_insert((1, BTreeMap::new()));
                    match message.part {
                        Some(part) => {
                            if let Some(n) = part.count {
                                *count = n;
                            }
                            parts.insert(part.index, payload.to_vec());
                            if parts.len() < usize::from(*count) {
                                continue;
                            }
                        }
                        None => {
                            parts.insert(0, payload.to_vec());
                        }
                    }
                    let whole: Vec<u8> = parts.values().flatten().copied().collect();
                    place.options.remove(&x);
                    if let Ok((_, parsed)) = static_info::parse_payload(&whole, dictionary) {
                        let roster = parsed.players.len() as u8;
                        place.roster = roster;
                        options.entry(roster).or_insert_with(|| whole[1..].to_vec());
                    }
                    continue;
                }
                // A 0xEA to the leaver itself is its farewell (match.md: the
                // server closes the game with it, at tick 0 when the game
                // never loaded); the others' copies are the leave.
                0xEA if payload.len() == 2 && payload[1] == record.place => continue,
                // The relays, read below.
                0xE9 | 0xEA if payload.len() == 2 => {}
                0xEB if payload.len() == 10 => {}
                0xEC if payload.len() == 11 => {}
                0xED | 0xEE => {}
                0xF3 if payload.len() == 3 => {}
                _ => continue,
            }
            let next = sent.len();
            let id = *messages
                .entry((message.id, mask_len, payload.to_vec()))
                .or_insert(next);
            if id == next {
                sent.push(Vec::new());
            }
            sent[id].push((record.at_ms, record.place));
            copies.push((record.at_ms, record.place, id));
        }
    }

    // The signs of a backed-up link: a copy well behind the same message
    // to another place.
    let mut by_id: Vec<(u8, usize, &[u8])> = vec![(0, 0, &[]); sent.len()];
    for ((message_id, mask_len, payload), &id) in &messages {
        by_id[id] = (*message_id, *mask_len, payload);
    }
    let mut behind: BTreeMap<u8, Vec<u32>> = BTreeMap::new();
    for times in &mut sent {
        times.sort_unstable();
        for &(at_ms, place) in times.iter() {
            let from = times.partition_point(|&(t, _)| t + LOOKBACK_MS < at_ms);
            let to = times.partition_point(|&(t, _)| t <= at_ms + NEAR_AFTER_MS);
            let others = || times[from..to].iter().filter(|&&(_, other)| other != place);
            if others().any(|&(t, _)| t + BEHIND_MS < at_ms)
                && !others().any(|&(t, _)| t + BEHIND_MS >= at_ms)
            {
                behind.entry(place).or_default().push(at_ms);
            }
        }
    }
    for signs in behind.values_mut() {
        signs.sort_unstable();
    }
    let lagging = |place: u8, at_ms: u32| {
        behind.get(&place).is_some_and(|signs| {
            let next = signs.partition_point(|&t| t + LAG_HOLD_MS < at_ms);
            signs.get(next).is_some_and(|&t| t <= at_ms + LAG_HOLD_MS)
        })
    };
    let mut kept = vec![false; sent.len()];
    for &(at_ms, place, id) in &copies {
        if lagging(place, at_ms) {
            *report.lagging_copies.entry(place).or_default() += 1;
            continue;
        }
        kept[id] = true;
        let clock = (i64::from(at_ms) - offset).max(0) as u32;
        let (message_id, mask_len, payload) = by_id[id];
        match message_id {
            0xE9 | 0xEA => {
                let Some(time) = resolve(payload[0], 0, clock, &mut report) else {
                    continue;
                };
                let key = (time, payload[1]);
                if message_id == 0xE9 {
                    entered.entry(key).or_insert(());
                } else {
                    left.entry(key).or_insert(());
                }
            }
            0xEB => {
                let Some(time) = resolve(payload[0], 0, clock, &mut report) else {
                    continue;
                };
                keep_input(&mut inputs, &mut report, time, payload[1], &payload[2..10]);
            }
            0xEC => {
                let Some(time) = resolve(payload[0], payload[1], clock, &mut report) else {
                    continue;
                };
                keep_input(&mut inputs, &mut report, time, payload[2], &payload[3..11]);
            }
            0xED | 0xEE => {
                let precise = message_id == 0xEE;
                let head = if precise { 2 } else { 1 };
                if payload.len() < head + mask_len {
                    continue;
                }
                let ms = if precise { payload[1] } else { 0 };
                let Some(time) = resolve(payload[0], ms, clock, &mut report) else {
                    continue;
                };
                let mask = &payload[head..head + mask_len];
                let players: Vec<u8> = (0..(mask_len * 8) as u8)
                    .filter(|p| mask[usize::from(p / 8)] & (1 << (p % 8)) != 0)
                    .collect();
                let n = players.len();
                let rest = &payload[head + mask_len..];
                if rest.len() < 8 * n {
                    continue;
                }
                let (actions, rest) = rest.split_at(4 * n);
                let (rot_x, rot_y) = rest.split_at(2 * n);
                for (i, &player) in players.iter().enumerate() {
                    let mut bytes = [0u8; 8];
                    bytes[0..2].copy_from_slice(&rot_x[2 * i..2 * i + 2]);
                    bytes[2..4].copy_from_slice(&rot_y[2 * i..2 * i + 2]);
                    bytes[4..8].copy_from_slice(&actions[4 * i..4 * i + 4]);
                    keep_input(&mut inputs, &mut report, time, player, &bytes);
                }
            }
            0xF3 => {
                let Some(time) = resolve(payload[0], 0, clock, &mut report) else {
                    continue;
                };
                profiles.entry((time, payload[1])).or_insert(payload[2]);
            }
            _ => unreachable!("only relays are kept"),
        }
    }
    report.messages_lost = kept.iter().filter(|kept| !**kept).count();

    report.inputs = inputs.len();
    report.profiles = profiles.len();
    report.entered = entered.len();
    report.left = left.len();
    report.world_times = worlds.len();
    for &(time, player) in entered.keys() {
        report.roster_events.push((time, 'E', player));
    }
    for &(time, player) in left.keys() {
        report.roster_events.push((time, 'L', player));
    }
    report.roster_events.sort_unstable();
    for &(time, _) in inputs.keys() {
        let minute = (time / 60_000) as usize;
        if report.inputs_per_minute.len() <= minute {
            report.inputs_per_minute.resize(minute + 1, 0);
        }
        report.inputs_per_minute[minute] += 1;
    }

    let (&roster, options_stream) = options.iter().next_back().ok_or(ConvertError::NoOptions)?;
    report.roster = roster;
    let options_decoded = lzma2::unpack(options_stream, 0x578A0)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    if options.len() > 1 {
        report.notes.push(format!(
            "the roster was {:?} in the 0xE4s the places got; used {roster}",
            options.keys().collect::<Vec<_>>()
        ));
    }

    // One world for each time.
    let mut chosen: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    for (time, bodies) in &worlds {
        let (best, kinds) = majority(bodies);
        if kinds > 1 {
            report.worlds_disagreeing += 1;
        }
        chosen.insert(*time, bodies[best].clone());
    }
    let first_world = *chosen.keys().next().ok_or(ConvertError::NoWorlds)?;
    report.first_world_ms = Some(first_world);

    // Events by time.
    let mut events: BTreeMap<u32, Event> = BTreeMap::new();
    for (&(time, player), value) in &inputs {
        events
            .entry(time)
            .or_insert_with(|| Event::at(time))
            .inputs
            .push((player, *value));
    }
    for (&(time, player), &value) in &profiles {
        events
            .entry(time)
            .or_insert_with(|| Event::at(time))
            .profiles
            .push((player, value));
    }
    for &(time, player) in entered.keys() {
        events.entry(time).or_insert_with(|| Event::at(time)).enter |= 1 << player;
    }
    for &(time, player) in left.keys() {
        events.entry(time).or_insert_with(|| Event::at(time)).leave |= 1 << player;
    }

    // The replay starts with the first world (50 ms in every sample, the
    // time a fresh world core has, 0x2D0765), or with an event before it.
    let start = first_world.min(events.keys().next().copied().unwrap_or(first_world));
    report.start_ms = start;
    let last_event = events.keys().next_back().copied().unwrap_or(start);
    let last_world = *chosen.keys().next_back().expect("a world");
    let end = last_event.max(last_world);
    report.end_ms = end;

    let mut writer = Writer::new(start);
    for event in events.values() {
        // An input of a player outside the roster the client knows.
        let mut event = event.clone();
        event
            .inputs
            .retain(|&(player, _)| usize::from(player) < MAX_PLAYERS);
        writer.push(&event)?;
    }
    let body = writer.finish(end);
    report.body_bytes = body.len();
    let records_read = sur::read_body(&body).expect("the writer's own body reads back");

    // Key frames: the first world; one soon after each late joiner enters
    // (the client has no late join to run in a replay, so the world that
    // holds the newcomer is what puts it there); and one at least every
    // `every` ms in between, the gap raised until at most 128 do.
    let initial = options.keys().next().copied().unwrap_or(roster);
    let mut must: Vec<u32> = entered
        .keys()
        .filter(|&&(_, player)| player >= initial)
        .filter_map(|&(time, _)| {
            chosen
                .range(time + LATE_JOINER_WORLD_AFTER_MS..)
                .next()
                .map(|(&t, _)| t)
        })
        .collect();
    must.sort_unstable();
    must.dedup();
    report.late_joiners = entered.keys().filter(|&&(_, p)| p >= initial).count();
    let span = last_world.saturating_sub(first_world);
    let room = (MAX_KEY_FRAMES - 2).saturating_sub(must.len()).max(1) as u32;
    let every = settings.key_frame_every_ms.max(span / room);
    let mut times = vec![first_world];
    for &time in chosen.keys() {
        if time >= times[times.len() - 1] + every {
            times.push(time);
        }
    }
    times.extend(&must);
    times.sort_unstable();
    times.dedup();
    times.truncate(MAX_KEY_FRAMES);
    times.retain(|time| !settings.drop_key_frames.contains(time));
    if settings.first_key_frame_only {
        times.truncate(1);
    }
    // The viewer binds every place of chunk 1 at its start, so its list of
    // game objects holds each late joiner's grenades and traps from the
    // first tick; the recorded games got them only at the install, as zero
    // bytes put in front of the saved objects (client_patch.md 3.14). A
    // world from before an install is given those bytes here, or every
    // object of the viewer reads out of step (docs/notes/replay_sur.md).
    let installs = find_installs(&chosen, &entered, initial, &mut report);
    // A joiner whose install was not found: a key frame from before its
    // entry may lack its objects' bytes, and is left out.
    let unsure_until = entered
        .keys()
        .filter(|(_, place)| report.installs_not_found.contains(place))
        .map(|&(time, _)| time)
        .max();
    if let Some(until) = unsure_until {
        let before = times.len();
        times.retain(|&time| time == first_world || time >= until);
        report.dropped_unsure_key_frames = before - times.len();
    }
    let mut frames = Vec::new();
    for &time in &times {
        // Where the reading goes on: the first record after the world's time.
        let offset = records_read
            .iter()
            .find(|record| record.event.time > time)
            .map_or(body.len(), |record| record.offset);
        let missing: usize = installs
            .iter()
            .filter(|install| install.at > time)
            .map(|install| install.objects)
            .sum();
        let world = if missing == 0 {
            chosen[&time].clone()
        } else {
            pad_objects(&chosen[&time], missing).ok_or(ConvertError::Verify(format!(
                "the world at {time} ms does not split into its parts"
            )))?
        };
        if missing != 0 {
            report.padded_key_frames += 1;
        }
        frames.push(KeyFrame::new(time, &world, offset));
    }
    report.key_frame_parts = times
        .iter()
        .map(|time| {
            let parts = snapshot::Sections::walk(&chosen[time]).map(|s| {
                [
                    s.players.len(),
                    s.bullets.len(),
                    s.statistics.len(),
                    s.objects.len(),
                    s.physics.len(),
                    s.rules.len(),
                    s.effects.len(),
                ]
            });
            (*time, parts)
        })
        .collect();
    report.key_frames = times;

    let parts = Parts {
        options_lzma2: options_stream,
        body: &body,
        key_frames: &frames,
        config,
    };
    report.decoded_bytes = sur::decoded_size(options_decoded, &parts);
    let file = sur::write(&parts, options_decoded, settings.coding)?;
    sur::verify(&file).map_err(ConvertError::Verify)?;
    Ok((file, report))
}

/// A late joiner's grenades and traps put into the recorded games.
struct Install {
    /// The first world that has them.
    at: u32,
    objects: usize,
}

/// For each late joiner, the first world whose game objects are the world
/// before it with zero bytes in front: the install, and how many objects
/// (0 for a place with no grenade or trap: the objects did not grow).
/// Looked for between 60 s before the entry and the entry.
fn find_installs(
    chosen: &BTreeMap<u32, Vec<u8>>,
    entered: &BTreeMap<(u32, u8), ()>,
    initial: u8,
    report: &mut Report,
) -> Vec<Install> {
    let objects = |body: &Vec<u8>| snapshot::Sections::walk(body).map(|s| s.objects.to_vec());
    let mut installs = Vec::new();
    let mut joins: Vec<(u32, u8)> = entered
        .keys()
        .filter(|&&(_, place)| place >= initial)
        .copied()
        .collect();
    joins.sort_unstable();
    for (entry, place) in joins {
        let window: Vec<(u32, Option<Vec<u8>>)> = chosen
            .range(entry.saturating_sub(INSTALL_WINDOW_MS)..=entry)
            .map(|(&time, body)| (time, objects(body)))
            .collect();
        let mut found = None;
        let mut unchanged = false;
        for pair in window.windows(2) {
            let (Some(before), Some(after)) = (&pair[0].1, &pair[1].1) else {
                continue;
            };
            if after.len() > before.len() {
                let grown = after.len() - before.len();
                if after[..grown].iter().all(|&b| b == 0) && after[grown..] == before[..] {
                    found = Some(Install {
                        at: pair[1].0,
                        objects: grown,
                    });
                }
            } else if after == before {
                unchanged = true;
            }
        }
        // A joiner already counted by an earlier join's window is not
        // counted twice.
        match found {
            Some(install) if !installs.iter().any(|i: &Install| i.at == install.at) => {
                report.installs.push((install.at, install.objects));
                installs.push(install);
            }
            Some(_) => {}
            None if unchanged => report.installs.push((entry, 0)),
            None => report.installs_not_found.push(place),
        }
    }
    installs
}

/// `body` with `count` zero bytes in front of its game objects.
fn pad_objects(body: &[u8], count: usize) -> Option<Vec<u8>> {
    let sections = snapshot::Sections::walk(body)?;
    let mut objects = vec![0u8; count];
    objects.extend_from_slice(sections.objects);
    snapshot::Sections {
        objects: &objects,
        ..sections
    }
    .to_body()
}

/// An input's time: the tick byte read against the match clock; an event
/// the clock puts well into the future is a late copy of an older one.
fn resolve(tick: u8, ms: u8, clock: u32, report: &mut Report) -> Option<u32> {
    let time = tick_time(tick, clock)? + u32::from(ms);
    if time > clock + FUTURE_SLACK_MS {
        report.dropped_future += 1;
        return None;
    }
    Some(time)
}

fn keep_input(
    inputs: &mut BTreeMap<(u32, u8), InputValue>,
    report: &mut Report,
    time: u32,
    player: u8,
    bytes: &[u8],
) {
    let actions = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
    let value = InputValue {
        rotation: bytes[0..4].try_into().expect("4 bytes"),
        actions: actions & !INPUT_FLAG,
    };
    match inputs.get(&(time, player)) {
        None => {
            if actions & INPUT_FLAG != 0 {
                report.flagged_inputs += 1;
            }
            inputs.insert((time, player), value);
        }
        Some(known) if *known == value => report.input_copies += 1,
        Some(_) => report.input_conflicts += 1,
    }
}

/// The start of the tick a time is in; here for the CLI's summary.
pub fn tick_of(time: u32) -> u32 {
    time / TICK_MS
}

#[cfg(test)]
mod tests {
    use super::*;
    use survarium::v026g::match_msgs::static_info::{
        MatchItem, MatchKit, MatchOptions, MatchProfile, NO_PROTECTION, SCORE_EVENTS, StaticInfo,
    };
    use survarium::v026g::match_msgs::upload;

    fn profile(name: &str, team: u8) -> MatchProfile {
        MatchProfile {
            team,
            name: name.into(),
            clan_tag: String::new(),
            protection: NO_PROTECTION,
            kits: vec![MatchKit::empty(1, "kit")],
            items: vec![MatchItem::default()],
            squad: 0,
        }
    }

    fn note(at_ms: u32, json: &str) -> Record {
        Record {
            kind: NOTE,
            place: 0xFF,
            at_ms,
            bytes: json.as_bytes().to_vec(),
        }
    }

    /// `seq rseq ack0 id [x] payload`, a single datagram.
    fn datagram(kind: u8, place: u8, at_ms: u32, id: u8, x: Option<u8>, payload: &[u8]) -> Record {
        let mut bytes = vec![0, 0, 0, id];
        bytes.extend(x);
        bytes.extend_from_slice(payload);
        Record {
            kind,
            place,
            at_ms,
            bytes,
        }
    }

    fn relay(tick: u8, player: u8, rotation: [u8; 4], actions: u32) -> Vec<u8> {
        let mut payload = vec![tick, player];
        payload.extend(rotation);
        payload.extend(actions.to_le_bytes());
        payload
    }

    /// Two places, a match clock 1300 ms behind the recording.
    fn recording() -> Vec<Record> {
        let map = survarium::v026g::maps::playable().next().unwrap();
        let options = MatchOptions {
            players: vec![profile("a", 0), profile("b", 1)],
            map_id: map.id,
            match_id: 1,
            match_level: 1,
            score_weights: [0.0; SCORE_EVENTS],
        };
        let info = StaticInfo::new(&options, Dictionary::shared()).unwrap();
        let mut records = vec![
            note(
                0,
                &format!(
                    r#"{{"note":"header","map":"{}","map_id":{},"mode":{},"match_id":"0x1","places":[{{"place":0}},{{"place":1}}]}}"#,
                    map.name, map.id, map.mode
                ),
            ),
            note(1350, r#"{"note":"clock","match_ms":50}"#),
        ];
        for place in 0..2u8 {
            records.push(datagram(
                OUT,
                place,
                1400,
                0xE4,
                Some(0),
                &info.payload(place).unwrap(),
            ));
        }
        // Both places hear of the entries; place 1 also hears the leave of 0.
        for place in 0..2u8 {
            records.push(datagram(OUT, place, 1700, 0xE9, None, &[8, 0]));
        }
        // Player 1's input at 1000 ms (tick 20), twice to place 0 (a copy),
        // player 0's at 1050 ms (tick 21) to place 1; one more of player 1
        // at 1000 ms + 7 ms through 0xEC.
        records.push(datagram(
            OUT,
            0,
            2400,
            0xEB,
            None,
            &relay(20, 1, [1, 2, 3, 4], 0x10),
        ));
        records.push(datagram(
            OUT,
            0,
            2450,
            0xEB,
            None,
            &relay(20, 1, [1, 2, 3, 4], 0x10),
        ));
        records.push(datagram(
            OUT,
            1,
            2420,
            0xEB,
            None,
            &relay(21, 0, [5, 6, 7, 8], 0x20),
        ));
        let mut precise = vec![20, 7, 1];
        precise.extend([9, 9, 9, 9]);
        precise.extend(0x30u32.to_le_bytes());
        records.push(datagram(OUT, 0, 2460, 0xEC, None, &precise));
        // A kit choice, and a leave at tick 40 (a farewell to the leaver
        // itself is not one).
        records.push(datagram(OUT, 0, 2500, 0xF3, None, &[22, 1, 3]));
        records.push(datagram(OUT, 0, 2600, 0xEA, None, &[40, 1]));
        records.push(datagram(OUT, 1, 2600, 0xEA, None, &[0, 1]));
        // Worlds at 50 ms and 1050 ms, uploaded by place 0.
        for (time, fill) in [(50u32, 1u8), (1050, 2)] {
            for chunk in upload::split(time, &vec![fill; 700]).unwrap() {
                records.push(datagram(IN, 0, 3000 + time, 0xE0, None, &chunk));
            }
        }
        records
    }

    #[test]
    fn a_recording_becomes_a_checked_file() {
        let records = recording();
        let config = b"config bytes";
        for coding in [Coding::Stored, Coding::Packed] {
            let settings = Settings {
                coding,
                key_frame_every_ms: 10_000,
                drop_key_frames: Vec::new(),
                first_key_frame_only: false,
            };
            let (file, report) = convert(&records, config, settings).unwrap();
            assert_eq!(report.clock_offset_ms, 1300);
            assert_eq!(report.inputs, 3);
            assert_eq!(report.input_copies, 1);
            assert_eq!(report.unparsed, 0, "{:?}", report.unparsed_why);
            assert_eq!(report.roster, 2);
            assert_eq!(report.worlds_complete, 2);
            let verified = sur::verify(&file).unwrap();
            assert_eq!(verified.start_ms, 50);
            assert_eq!(
                verified.key_frames.iter().map(|k| k.0).collect::<Vec<_>>(),
                [50]
            );
            let chunks = sur::chunks(&file).unwrap();
            let body = lzma2::unpack(chunks[2].1, 1 << 20).unwrap();
            let events = sur::read_body(&body).unwrap();
            let seen: Vec<(u8, u32)> = events.iter().map(|r| (r.kind, r.event.time)).collect();
            // marker, enter at 400 (tick 8), input of 1 at 1000 and its
            // precise one at 1007, the kit choice at 1100, the input of 0
            // at 1050, the leave at 2000; each tick closed.
            assert_eq!(seen[0], (4, 50));
            assert_eq!(seen[1], (5 + 4, 400));
            let inputs: Vec<_> = events
                .iter()
                .flat_map(|r| {
                    r.event
                        .inputs
                        .iter()
                        .map(move |i| (r.event.time, i.0, i.1.actions))
                })
                .collect();
            assert_eq!(inputs, [(1000, 1, 0x10), (1007, 1, 0x30), (1050, 0, 0x20)]);
            let profiles: Vec<_> = events
                .iter()
                .flat_map(|r| r.event.profiles.clone())
                .collect();
            assert_eq!(profiles, [(1, 3)]);
            let leaves: Vec<_> = events
                .iter()
                .filter(|r| r.event.leave != 0)
                .map(|r| (r.event.time, r.event.leave))
                .collect();
            assert_eq!(leaves, [(2000, 2)]);
        }
    }

    /// `(time, player, actions)` of every input in a file.
    fn inputs_of(file: &[u8]) -> Vec<(u32, u8, u32)> {
        let chunks = sur::chunks(file).unwrap();
        let body = lzma2::unpack(chunks[2].1, 1 << 20).unwrap();
        sur::read_body(&body)
            .unwrap()
            .iter()
            .flat_map(|r| {
                r.event
                    .inputs
                    .iter()
                    .map(move |i| (r.event.time, i.0, i.1.actions))
            })
            .collect()
    }

    #[test]
    fn a_copy_from_a_backed_up_link_is_not_read_as_another_tick() {
        let mut records = recording();
        // Player 0's input of 1050 ms (tick 21) reaches place 0 18.6 s
        // after place 1 got it: read against that clock its tick byte is
        // 13 850 ms, a time the server never meant.
        records.push(datagram(
            OUT,
            0,
            21_000,
            0xEB,
            None,
            &relay(21, 0, [5, 6, 7, 8], 0x20),
        ));
        let (file, report) = convert(&records, b"config", Settings::default()).unwrap();
        assert_eq!(
            inputs_of(&file),
            [(1000, 1, 0x10), (1007, 1, 0x30), (1050, 0, 0x20)]
        );
        assert_eq!(report.lagging_copies, BTreeMap::from([(0, 1)]));
        assert_eq!(report.messages_lost, 0);
        assert_eq!(report.input_conflicts, 0);
    }

    #[test]
    fn the_flag_bit_of_an_input_is_not_an_action() {
        let mut records = recording();
        records.push(datagram(
            OUT,
            0,
            2470,
            0xEB,
            None,
            &relay(22, 1, [1, 2, 3, 4], 0x8000_0010),
        ));
        let (file, report) = convert(&records, b"config", Settings::default()).unwrap();
        assert!(inputs_of(&file).contains(&(1100, 1, 0x10)));
        assert_eq!(report.flagged_inputs, 1);
    }

    #[test]
    fn a_growing_roster_reads_masks_of_twenty() {
        let mut records = recording();
        let mut header: serde_json::Value = serde_json::from_slice(&records[0].bytes).unwrap();
        header["growing_roster"] = true.into();
        records[0].bytes = serde_json::to_vec(&header).unwrap();
        // 0xED at tick 23: players 0 and 1, a mask of three bytes, the
        // actions, then the first and the second halves of the rotations.
        let mut payload = vec![23, 0b11, 0, 0];
        payload.extend(0x40u32.to_le_bytes());
        payload.extend(0x41u32.to_le_bytes());
        payload.extend([1, 2, 5, 6]);
        payload.extend([3, 4, 7, 8]);
        records.push(datagram(OUT, 0, 2480, 0xED, None, &payload));
        let (file, report) = convert(&records, b"config", Settings::default()).unwrap();
        assert!(report.growing_roster);
        assert_eq!(report.unparsed, 0, "{:?}", report.unparsed_why);
        let inputs = inputs_of(&file);
        assert!(inputs.contains(&(1150, 0, 0x40)), "{inputs:?}");
        assert!(inputs.contains(&(1150, 1, 0x41)), "{inputs:?}");
    }
}
