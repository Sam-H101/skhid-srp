use std::sync::Arc;

use session::SessionStore;

fn main() {
    env_logger::init();
    spawn_console();

    // Standalone: restart on panic, mirroring the srp supervisor.
    loop {
        log::info!("Starting a match server");
        let store = Arc::new(SessionStore::new());
        if std::panic::catch_unwind(|| match_server::run(store)).is_err() {
            log::error!("Match server panicked; restarting");
        }
    }
}

/// Read match console commands typed into this terminal, as `srp` does.
/// Without a terminal, stdin ends at once and so does the thread.
fn spawn_console() {
    let spawned = std::thread::Builder::new()
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
        log::warn!("match-server: no console: {error}");
    }
}
