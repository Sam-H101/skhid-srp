//! Unified mock server: runs the login, browser and lobby servers in **one
//! process** sharing a single [`SessionStore`], so a client signed in at the
//! login server is recognised by the lobby (see PLAN Phases 2–4). The 0.21d
//! match server (a lockstep input relay) is not written yet; the 0.100b one
//! is in the `srp` folder.
//!
//! Each server runs on its own supervised thread: a panic or error in one is
//! logged and the server is restarted, so no single server can take down the
//! whole process. Accounts are kept in `<SRP_DATA_DIR>/srp.db` (see
//! `session::persist`); a saver thread writes changes every 2 s, and Ctrl+C
//! (or closing the console) writes the rest before exiting.

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

    let gather = match session::gather_window_from_env() {
        Ok(gather) => gather,
        Err(error) => {
            log::error!("srp: {error}");
            std::process::exit(1);
        }
    };
    // Accounts are loaded before any server can sign one in.
    let store = match SessionStore::from_env(gather) {
        Ok(store) => Arc::new(store),
        Err(error) => {
            log::error!("srp: cannot load the accounts: {error}");
            std::process::exit(1);
        }
    };
    store.spawn_saver();
    let on_exit = store.clone();
    let stop = ctrlc::set_handler(move || {
        log::info!("srp: stopping; saving the accounts");
        on_exit.flush();
        log::info!("srp: stopped");
        std::process::exit(0);
    });
    if let Err(error) = stop {
        log::warn!("srp: no Ctrl+C handler ({error}); changes of the last seconds may be lost");
    }

    let servers: [(&str, Run); 6] = [
        ("login", login_server::run),
        ("presence", login_server::run_presence),
        ("browser", browser_server::run),
        ("lobby", lobby_server::run),
        ("match", match_server::run),
        ("web", web_server::run),
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

    log::info!("srp: started login, browser, lobby and match servers");

    for handle in handles {
        let _ = handle.join();
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
