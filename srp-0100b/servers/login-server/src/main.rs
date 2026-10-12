use std::sync::Arc;

use session::SessionStore;

fn main() -> std::io::Result<()> {
    env_logger::init();
    let store = Arc::new(SessionStore::new());
    let pings = store.clone();
    std::thread::spawn(move || login_server::run_presence(pings));
    login_server::run(store)
}
