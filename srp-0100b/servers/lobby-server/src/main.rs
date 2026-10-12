use std::sync::Arc;

use session::SessionStore;

fn main() -> std::io::Result<()> {
    env_logger::init();
    let queue_min_players = session::queue_min_players_from_env().map_err(std::io::Error::other)?;
    lobby_server::run_standalone(Arc::new(SessionStore::with_queue_min_players(
        queue_min_players,
    )))
}
