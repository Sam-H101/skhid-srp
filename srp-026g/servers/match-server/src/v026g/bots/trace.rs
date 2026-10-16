//! The decision trace (`player_bots_research.md` B8 point 6): one JSON line
//! per bot when its play changes and every `trace_every_ms`, with the
//! time, the pose, the play and its step, the top candidates with each
//! consideration's input and result, the reason, the team's modifiers, the
//! skill and the tuning's hash. One line answers "why did it do that".
//!
//! Written to `<SRP_BOT_TRACE_DIR>/bots_<match>.jsonl` when that is set;
//! otherwise nothing is kept (the log has the play changes).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

pub struct Trace {
    out: Option<BufWriter<File>>,
    pub tuning_hash: u32,
}

impl Trace {
    pub fn none(tuning_hash: u32) -> Self {
        Self {
            out: None,
            tuning_hash,
        }
    }

    /// A trace file for match `match_id` in `dir`; none if it cannot be
    /// made (logged once).
    pub fn open(dir: &Path, match_id: u32, tuning_hash: u32) -> Self {
        let path = dir.join(format!("bots_{match_id:08x}.jsonl"));
        let out = std::fs::create_dir_all(dir)
            .and_then(|()| File::create(&path))
            .map(BufWriter::new)
            .map_err(|e| log::warn!("bot trace {}: {e}", path.display()))
            .ok();
        Self { out, tuning_hash }
    }

    pub fn enabled(&self) -> bool {
        self.out.is_some()
    }

    pub fn line(&mut self, mut value: serde_json::Value) {
        let Some(out) = self.out.as_mut() else {
            return;
        };
        if let Some(object) = value.as_object_mut() {
            object.insert("tuning".into(), format!("{:08x}", self.tuning_hash).into());
        }
        if writeln!(out, "{value}").is_err() {
            self.out = None;
        }
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        if let Some(out) = self.out.as_mut() {
            let _ = out.flush();
        }
    }
}
