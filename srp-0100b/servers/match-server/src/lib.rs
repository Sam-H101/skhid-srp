#![feature(iter_intersperse)]

mod batteries;
mod combat;
mod console;
mod game;
mod lifecycle;
mod map;
mod match_server;
mod message;
mod sequence_number;
mod supplies;
mod transport;
mod utils;

use std::sync::Arc;

use session::SessionStore;
use vostok::config;

use crate::match_server::MatchServer;

/// Run the match server: bind UDP and drive the relay loop forever (it never
/// returns). Panics are contained by the caller (the standalone `main`, or the
/// `srp` supervisor thread), which restarts it.
pub fn run(store: Arc<SessionStore>) -> std::io::Result<()> {
    let match_server = &config::get().match_server;
    let addr = format!("{}:{}", match_server.bind_host, match_server.port);
    MatchServer::bind(&addr, store)?.run()
}

/// Queue one line typed into the `srp` terminal for the match loop (see
/// `console.rs`). `Err` carries text for the operator: help, or why the line
/// was rejected.
pub fn console_command(line: &str) -> Result<(), String> {
    console::submit(line)
}
