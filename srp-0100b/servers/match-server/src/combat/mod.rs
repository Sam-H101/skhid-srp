// Real combat on the stock client, rebuilt from the game's shared gameplay code
// (`vostok/game_core`): the client simulates bullets only for show and never
// reports hits, so the server replays every shot from the players' inputs and
// sends the results. See WORK.md for what is exact and what is approximated.

pub mod ballistics;
pub mod collision;
pub mod damage;
pub mod data;
pub mod drugs;
pub mod math;
pub mod pose;
pub mod resources;
pub mod system;
pub mod traps;
pub mod weapon;
