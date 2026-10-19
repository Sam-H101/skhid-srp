//! Public aggregate counts. This reads state without advancing matchmaking.
use session::SessionStore;

use crate::{ContentType, Game, LiveMatch, Response};

pub(super) fn response(store: &SessionStore, game: &Game) -> Response {
    let online = store.players_present();
    let registered = store.registered_accounts();
    let matches = game.matches.active_matches();
    let rooms = store.room_counts();
    let last_match_ends_in = last_match_ends_in(&game.matches.live_matches());
    Response {
        status: "200 OK",
        content_type: ContentType::Json,
        // Only integers enter the JSON; no player or account data is exposed.
        body: format!(
            "{{\"online\":{online},\"registered\":{registered},\"matches\":{matches},\"rooms\":{},\
             \"in_rooms\":{},\"queued\":{},\"last_match_ends_in\":{last_match_ends_in}}}",
            rooms.rooms, rooms.in_rooms, rooms.queued
        ),
    }
}

/// Whole seconds, rounded up, until the last of the matches ends at the
/// latest ([`LiveMatch::ends_within`]); 0 when none runs. A release plans
/// its maintenance from it (`release/release.sh`).
fn last_match_ends_in(live: &[LiveMatch]) -> u128 {
    live.iter()
        .map(LiveMatch::ends_within)
        .max()
        .map_or(0, |most| most.as_millis().div_ceil(1000))
}
