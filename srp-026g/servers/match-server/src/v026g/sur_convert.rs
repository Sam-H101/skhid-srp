//! A match recording ([`super::record`]) made into a `.sur` replay
//! ([`super::sur`]). Offline: nothing here runs in the server.
//!
//! What goes where:
//! - the options (chunk 1): the stream of a 0xE4 the server sent, without
//!   its first byte, from the place that got the largest roster;
//! - the events (chunk 2): what the server relayed to the players (0xEB,
//!   0xEC, 0xED, 0xEE, 0xF3, 0xE9, 0xEA), every message of every place put
//!   together, a copy of an event dropped. Its tick byte is read against the
//!   match clock the recording's own times give (as a client does, with the
//!   clock the sender had); an event the clock puts in the future is a copy
//!   of an older one that came 12.8 s late, and is dropped;
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
/// The match clock is the recording's time less an offset; the offset is
/// worked out from the notes that carry both.
const DEFAULT_KEY_FRAME_EVERY_MS: u32 = 10_000;
/// The world kept for a late joiner is the first one this long after its
/// entry: the item must be later than the entry's tick.
const LATE_JOINER_WORLD_AFTER_MS: u32 = 300;

/// What the caller chooses.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub coding: Coding,
    /// The least gap between two key frames; larger if 128 would not do.
    pub key_frame_every_ms: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            coding: Coding::Packed,
            key_frame_every_ms: DEFAULT_KEY_FRAME_EVERY_MS,
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
    pub profiles: usize,
    pub entered: usize,
    pub left: usize,
    pub roster: u8,
    /// Places that entered beyond the roster the first 0xE4 had.
    pub late_joiners: usize,
    pub worlds_complete: usize,
    pub world_times: usize,
    pub worlds_disagreeing: usize,
    pub first_world_ms: Option<u32>,
    pub start_ms: u32,
    pub end_ms: u32,
    pub key_frames: Vec<u32>,
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
        ..Report::default()
    };
    let offset = clock_offset(records).unwrap_or(0);
    report.clock_offset_ms = offset;
    let dictionary = Dictionary::shared();

    let mut places: HashMap<u8, PlaceState> = HashMap::new();
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
        let clock = (i64::from(record.at_ms) - offset).max(0) as u32;
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
        let parsed = wire::parse(
            Table::Match,
            Direction::ToClient,
            &record.bytes,
            if place.roster == 0 {
                0
            } else {
                mask_bytes(place.roster)
            },
        );
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
                }
                // A 0xEA to the leaver itself is its farewell (match.md: the
                // server closes the game with it, at tick 0 when the game
                // never loaded); the others' copies are the leave.
                0xEA if payload.len() == 2 && payload[1] == record.place => {}
                0xE9 | 0xEA if payload.len() == 2 => {
                    let Some(time) = resolve(payload[0], 0, clock, &mut report) else {
                        continue;
                    };
                    let key = (time, payload[1]);
                    if message.id == 0xE9 {
                        entered.entry(key).or_insert(());
                    } else {
                        left.entry(key).or_insert(());
                    }
                }
                0xEB if payload.len() == 10 => {
                    let Some(time) = resolve(payload[0], 0, clock, &mut report) else {
                        continue;
                    };
                    keep_input(&mut inputs, &mut report, time, payload[1], &payload[2..10]);
                }
                0xEC if payload.len() == 11 => {
                    let Some(time) = resolve(payload[0], payload[1], clock, &mut report) else {
                        continue;
                    };
                    keep_input(&mut inputs, &mut report, time, payload[2], &payload[3..11]);
                }
                0xED | 0xEE => {
                    let precise = message.id == 0xEE;
                    let head = if precise { 2 } else { 1 };
                    let mask_len = mask_bytes(place.roster);
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
                0xF3 if payload.len() == 3 => {
                    let Some(time) = resolve(payload[0], 0, clock, &mut report) else {
                        continue;
                    };
                    profiles.entry((time, payload[1])).or_insert(payload[2]);
                }
                _ => {}
            }
        }
    }

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

    // The replay starts with the first world; events before it are cut.
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
    let mut frames = Vec::new();
    for &time in &times {
        // Where the reading goes on: the first record after the world's time.
        let offset = records_read
            .iter()
            .find(|record| record.event.time > time)
            .map_or(body.len(), |record| record.offset);
        frames.push(KeyFrame::new(time, &chosen[&time], offset));
    }
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
    let value = InputValue {
        rotation: bytes[0..4].try_into().expect("4 bytes"),
        actions: u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes")),
    };
    match inputs.get(&(time, player)) {
        None => {
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
}
