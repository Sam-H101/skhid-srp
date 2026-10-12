use std::sync::Arc;

use session::SessionStore;

fn main() -> std::io::Result<()> {
    env_logger::init();
    let gather = session::gather_window_from_env().map_err(std::io::Error::other)?;
    lobby_server::run_standalone(Arc::new(SessionStore::with_gather_window(gather)))
}
