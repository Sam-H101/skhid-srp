//! Public aggregate counts. This reads state without advancing matchmaking.
use session::SessionStore;

use crate::{ContentType, Game, Response};

pub(super) fn response(store: &SessionStore, game: &Game) -> Response {
    let online = store.players_present();
    let registered = store.registered_accounts();
    let matches = game.matches.active_matches();
    let queued = store.matchmaking().queued_players();
    Response {
        status: "200 OK",
        content_type: ContentType::Json,
        // Only integers enter the JSON; no player or account data is exposed.
        body: format!(
            "{{\"online\":{online},\"registered\":{registered},\"matches\":{matches},\"queued\":{queued}}}"
        ),
    }
}
