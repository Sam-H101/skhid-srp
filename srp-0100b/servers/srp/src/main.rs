//! Unified mock server: runs the login, browser, lobby and match servers in
//! **one process** sharing a single [`SessionStore`], so a client signed in at
//! the login server is recognised by the lobby, and the match it was routed to is
//! recognised by the match server (see PLAN Phases 2–4).
//!
//! Each server runs on its own supervised thread: a panic or error in one is
//! logged and the server is restarted, so no single server can take down the
//! whole process.

mod logger;

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use log::LevelFilter;
use session::SessionStore;

/// Delay before restarting a server that exited, to avoid a hot crash loop.
const RESTART_DELAY: Duration = Duration::from_secs(1);

type Run = fn(Arc<SessionStore>) -> std::io::Result<()>;

fn main() {
    // A single level from RUST_LOG (e.g. `RUST_LOG=debug`), defaulting to info.
    let level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(LevelFilter::Info);
    logger::init(level).expect("failed to install logger");
    log_panics();
    // Before any server: a bad setting would restart it forever.
    if let Err(error) = vostok::config::load() {
        log::error!("srp: {error}");
        std::process::exit(1);
    }

    let queue_min_players = match session::queue_min_players_from_env() {
        Ok(count) => count,
        Err(error) => {
            log::error!("srp: {error}");
            std::process::exit(1);
        }
    };
    let store = Arc::new(SessionStore::with_queue_min_players(queue_min_players));

    let servers: [(&str, Run); 5] = [
        ("login", login_server::run),
        ("presence", login_server::run_presence),
        ("browser", browser_server::run),
        ("lobby", lobby_server::run),
        ("match", match_server::run),
    ];

    let handles: Vec<_> = servers
        .into_iter()
        .map(|(name, run)| {
            let store = store.clone();
            thread::Builder::new()
                .name(name.to_string())
                .spawn(move || supervise(name, store, run))
                .expect("failed to spawn server thread")
        })
        .collect();

    spawn_console();
    log::info!("srp: started login, browser, lobby and match servers");
    log::info!("srp: type `help` here for the match console commands");

    for handle in handles {
        let _ = handle.join();
    }
}

/// Read match console commands (`status`, `start`, `end`, `time`) typed into
/// this terminal. Without a terminal, stdin ends at once and so does the thread.
fn spawn_console() {
    let spawned = thread::Builder::new()
        .name("console".to_string())
        .spawn(|| {
            for line in std::io::stdin().lines() {
                let Ok(line) = line else {
                    break;
                };
                if line.trim().is_empty() {
                    continue;
                }
                if let Err(message) = match_server::console_command(&line) {
                    for line in message.lines() {
                        log::info!("{line}");
                    }
                }
            }
        });
    if let Err(error) = spawned {
        log::warn!("srp: no console: {error}");
    }
}

/// Record every panic's message and location in the originating server's log.
/// Connection handlers and the supervisor contain panics with `catch_unwind`,
/// which otherwise leaves only a "panicked" line and loses the reason to stderr.
fn log_panics() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        let location = info
            .location()
            .map(|location| location.to_string())
            .unwrap_or_default();
        // Route by the source crate, since connection threads are unnamed.
        let target = [
            ("login-server", "login_server"),
            ("lobby-server", "lobby_server"),
            ("browser-server", "browser_server"),
            ("match-server", "match_server"),
        ]
        .into_iter()
        .find(|(directory, _)| location.contains(directory))
        .map_or("srp", |(_, target)| target);
        let thread = thread::current();
        let thread = thread.name().unwrap_or("<unnamed>");
        log::error!(target: target, "panic in thread '{thread}' at {location}: {message}");
        default_hook(info);
    }));
}

/// Run one server forever, restarting it (after a short delay) if it returns or
/// panics. Keeps a crashing server from bringing down the others.
fn supervise(name: &str, store: Arc<SessionStore>, run: Run) {
    loop {
        let store = store.clone();
        match std::panic::catch_unwind(AssertUnwindSafe(|| run(store))) {
            Ok(Ok(())) => log::warn!("{name} server exited; restarting"),
            Ok(Err(error)) => log::error!("{name} server failed: {error}; restarting"),
            Err(_) => log::error!("{name} server panicked; restarting"),
        }
        thread::sleep(RESTART_DELAY);
    }
}
