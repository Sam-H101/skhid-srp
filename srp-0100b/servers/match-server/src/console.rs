// Debug console for the match lifecycle and batteries.
//
// The `srp` binary feeds lines typed into its terminal to `submit`; the match
// loop drains the queue once per frame. A global queue (rather than a channel
// owned by one `MatchServer`) survives the supervisor restarting the server.

use std::collections::VecDeque;
use std::sync::Mutex;

use session::LockExt;

pub const HELP: &str =
    "match console commands (MATCH is a match id like 0x123; omitted = all matches):
  status               phase, time left and players of every match
  start [MATCH]        skip waiting/countdown and start the match now
  end [MATCH]          finish the match now (clients return to the lobby)
  time SECS [MATCH]    set the time left in the current phase
  batteries [MATCH]    where every battery is, and the team scores
  resync [MATCH]       make every client rebuild its world from the server's
  help                 this text";

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Command {
    Status,
    Start { match_id: Option<u32> },
    End { match_id: Option<u32> },
    Time { seconds: u32, match_id: Option<u32> },
    Batteries { match_id: Option<u32> },
    Resync { match_id: Option<u32> },
}

static QUEUE: Mutex<VecDeque<Command>> = Mutex::new(VecDeque::new());

/// Parse one console line and queue it for the match loop. `Err` carries text
/// for the operator (the help text, or why the line was rejected).
pub fn submit(line: &str) -> Result<(), String> {
    let command = parse(line)?;
    QUEUE.locked().push_back(command);
    Ok(())
}

/// Every command queued since the last call.
pub fn take_all() -> Vec<Command> {
    QUEUE.locked().drain(..).collect()
}

fn parse(line: &str) -> Result<Command, String> {
    let mut words = line.split_whitespace();
    let command = words.next().unwrap_or_default().to_ascii_lowercase();
    let arguments: Vec<&str> = words.collect();

    let command = match (command.as_str(), arguments.as_slice()) {
        ("status", []) => Command::Status,
        ("start", match_id) if match_id.len() <= 1 => Command::Start {
            match_id: optional_match_id(match_id)?,
        },
        ("end", match_id) if match_id.len() <= 1 => Command::End {
            match_id: optional_match_id(match_id)?,
        },
        ("batteries", match_id) if match_id.len() <= 1 => Command::Batteries {
            match_id: optional_match_id(match_id)?,
        },
        ("resync", match_id) if match_id.len() <= 1 => Command::Resync {
            match_id: optional_match_id(match_id)?,
        },
        ("time", [seconds, match_id @ ..]) if match_id.len() <= 1 => Command::Time {
            seconds: seconds
                .parse()
                .map_err(|_| format!("not a number of seconds: {seconds}"))?,
            match_id: optional_match_id(match_id)?,
        },
        _ => return Err(HELP.to_owned()),
    };
    Ok(command)
}

fn optional_match_id(words: &[&str]) -> Result<Option<u32>, String> {
    let Some(word) = words.first() else {
        return Ok(None);
    };
    let parsed = match word.strip_prefix("0x").or_else(|| word.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => word.parse(),
    };
    parsed
        .map(Some)
        .map_err(|_| format!("not a match id: {word}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_console_commands() {
        assert_eq!(parse("status"), Ok(Command::Status));
        assert_eq!(parse(" START "), Ok(Command::Start { match_id: None }));
        assert_eq!(
            parse("end 0x124"),
            Ok(Command::End {
                match_id: Some(0x124)
            })
        );
        assert_eq!(
            parse("time 30 292"),
            Ok(Command::Time {
                seconds: 30,
                match_id: Some(292)
            })
        );
        assert_eq!(
            parse("batteries"),
            Ok(Command::Batteries { match_id: None })
        );
        assert_eq!(
            parse("resync 0x123"),
            Ok(Command::Resync {
                match_id: Some(0x123)
            })
        );
    }

    #[test]
    fn rejects_unknown_or_malformed_commands_with_help() {
        assert_eq!(parse("help"), Err(HELP.to_owned()));
        assert_eq!(parse(""), Err(HELP.to_owned()));
        assert_eq!(parse("status 1"), Err(HELP.to_owned()));
        assert_eq!(parse("time"), Err(HELP.to_owned()));
        assert!(parse("time soon").unwrap_err().contains("seconds"));
        assert!(parse("end zz").unwrap_err().contains("match id"));
    }
}
