mod connection_state;
mod message;
mod rewards;

use self::connection_state::ConnectionState;
pub use self::message::client::raw::faction_id;
pub use self::message::{client, server};

use crate::UnknownSessions;
use crate::messaging_server::ChatRoom;
use session::{Account, LockExt, QueueState, SessionStore};
use vostok::config;
use vostok::network_client::TcpClient;

use std::ffi::CStr;
use std::sync::Arc;

/// The client's state in a `client_state` status
/// (`lobby_menu::on_client_status_received`): 0 is the lobby (it hides the
/// queue screen), 2 a player waiting for a match (it shows the queue screen
/// and asks again a second later).
const STATUS_IN_LOBBY: u8 = 0;
const STATUS_QUEUED: u8 = 2;

/// Takes a session out of the queue when its lobby connection ends.
struct QueueGuard<'a> {
    store: &'a SessionStore,
    session_id: u32,
}

impl Drop for QueueGuard<'_> {
    fn drop(&mut self) {
        self.store.leave_queue(self.session_id);
    }
}

/// One item in the shops: who sells it and for how much.
struct Price {
    dict_id: u16,
    cost: u16,
    sellers: &'static [faction_id],
}

const SCAVENGERS: &[faction_id] = &[faction_id::scavengers];
const BLACK_MARKET: &[faction_id] = &[faction_id::black_market];
const BOTH: &[faction_id] = &[faction_id::scavengers, faction_id::black_market];

/// The 2013 prices, from the Survarium wiki of that year; each source, and
/// how its names were matched to these items, is in
/// `docs/notes/prices.md`. Ammo is priced by the round: the client
/// multiplies the price by the clip size for a clip. The scope and the
/// lifebone were never sold.
#[rustfmt::skip]
const PRICES: &[Price] = &[
    // Weapons, May 2013.
    Price { dict_id: 13, cost: 350, sellers: BLACK_MARKET }, // AKS-74U
    Price { dict_id: 14, cost: 350, sellers: SCAVENGERS },   // Remington 700
    Price { dict_id: 15, cost: 300, sellers: BLACK_MARKET }, // Remington 870
    Price { dict_id: 19, cost: 300, sellers: SCAVENGERS },   // Vityaz
    Price { dict_id: 16, cost: 200, sellers: SCAVENGERS },   // TOZ-34
    Price { dict_id: 55, cost: 200, sellers: BLACK_MARKET }, // Uzi
    Price { dict_id: 56, cost: 150, sellers: BLACK_MARKET }, // Colt Python
    Price { dict_id: 12, cost: 100, sellers: SCAVENGERS },   // TOZ-122
    Price { dict_id: 17, cost: 100, sellers: BLACK_MARKET }, // TOZ-66
    Price { dict_id: 64, cost: 100, sellers: SCAVENGERS },   // Fort 17
    Price { dict_id: 18, cost: 10,  sellers: BOTH },         // TT-33, a starting weapon
    // Ammo, December 2013: each box cost as many as it held rounds, so 1 a
    // round. Sold where its weapons are (the wiki doesn't say).
    Price { dict_id: 7,  cost: 1, sellers: BLACK_MARKET }, // 5.45x39, 30 for 30
    Price { dict_id: 20, cost: 1, sellers: BLACK_MARKET }, // .357 Magnum, 20 for 20
    Price { dict_id: 22, cost: 1, sellers: BOTH },         // 12x70 2/0, 20 for 20
    Price { dict_id: 72, cost: 1, sellers: BOTH },         // 12x70 slug, 20 for 20
    Price { dict_id: 73, cost: 1, sellers: BOTH },         // 12x70 3/0: a guess, as 2/0
    Price { dict_id: 50, cost: 1, sellers: BOTH },         // 7.62x25, 50 for 50
    Price { dict_id: 51, cost: 1, sellers: SCAVENGERS },   // 7.62x51, 10 for 10
    Price { dict_id: 52, cost: 1, sellers: SCAVENGERS },   // 7.62x51 AP, 10 for 10
    Price { dict_id: 70, cost: 1, sellers: SCAVENGERS },   // 9x18, 50 for 50
    Price { dict_id: 53, cost: 1, sellers: BOTH },         // 9x19 FMJ, 50 for 50
    Price { dict_id: 71, cost: 1, sellers: BOTH },         // 9x19 HP, 50 for 50
    // Scavenger armour, May 2013.
    Price { dict_id: 43, cost: 50,  sellers: SCAVENGERS }, // "Amber" respirator
    Price { dict_id: 31, cost: 50,  sellers: SCAVENGERS }, // quilted jacket
    Price { dict_id: 34, cost: 100, sellers: SCAVENGERS }, // chemical protection jacket
    Price { dict_id: 48, cost: 150, sellers: SCAVENGERS }, // light armoured vest
    Price { dict_id: 49, cost: 100, sellers: SCAVENGERS }, // backpack with container
    Price { dict_id: 9,  cost: 100, sellers: SCAVENGERS }, // oxygen tank: a guess, as the backpack
    Price { dict_id: 45, cost: 50,  sellers: SCAVENGERS }, // linen pants
    Price { dict_id: 46, cost: 100, sellers: SCAVENGERS }, // chemical protection pants
    Price { dict_id: 47, cost: 150, sellers: SCAVENGERS }, // field pants
    Price { dict_id: 41, cost: 50,  sellers: SCAVENGERS }, // cut gloves
    Price { dict_id: 42, cost: 100, sellers: SCAVENGERS }, // chemical protection gloves
    Price { dict_id: 37, cost: 50,  sellers: SCAVENGERS }, // leather boots
    Price { dict_id: 38, cost: 100, sellers: SCAVENGERS }, // chemical protection boots
    Price { dict_id: 39, cost: 150, sellers: SCAVENGERS }, // military boots
    // Black Market armour, May 2013.
    Price { dict_id: 27, cost: 50,  sellers: BLACK_MARKET }, // biker's helmet
    Price { dict_id: 29, cost: 50,  sellers: BLACK_MARKET }, // leather sleeveless jacket
    Price { dict_id: 32, cost: 100, sellers: BLACK_MARKET }, // "Bruiser" jacket
    Price { dict_id: 33, cost: 150, sellers: BLACK_MARKET }, // "Corsair" jacket
    Price { dict_id: 28, cost: 50,  sellers: BLACK_MARKET }, // jeans
    Price { dict_id: 44, cost: 100, sellers: BLACK_MARKET }, // reinforced jeans
    Price { dict_id: 25, cost: 50,  sellers: BLACK_MARKET }, // biker gloves
    Price { dict_id: 40, cost: 100, sellers: BLACK_MARKET }, // spiked gloves
    Price { dict_id: 24, cost: 50,  sellers: BLACK_MARKET }, // cossacks
    Price { dict_id: 35, cost: 100, sellers: BLACK_MARKET }, // ropers
    Price { dict_id: 36, cost: 150, sellers: BLACK_MARKET }, // choppers
    // Consumables: the trap May 2013, the drugs December 2013.
    Price { dict_id: 68, cost: 5, sellers: BLACK_MARKET }, // trap
    Price { dict_id: 67, cost: 5, sellers: BOTH },         // medkit
    Price { dict_id: 66, cost: 5, sellers: SCAVENGERS },   // set of bandages
    Price { dict_id: 65, cost: 5, sellers: BLACK_MARKET }, // anesthetic
];

pub struct ServerState {
    restricts: Vec<server::raw::profile_slot_restriction>,
    compats: Vec<server::raw::items_compatibility>,
    price_items: [(faction_id, Vec<server::raw::price_item>); 4],
    skills_tree: Vec<u8>,

    reroll_cost: u32,
    add_profile_cost: u32,
    rename_account_cost: u32,
}

impl ServerState {
    /// Serve one lobby connection until the client disconnects (the client drops
    /// it while in a match and reconnects afterwards).
    pub fn run(
        self: Arc<Self>,
        store: Arc<SessionStore>,
        chat: &ChatRoom,
        tcp_client: TcpClient,
        unknown: UnknownSessions,
    ) {
        let mut tcp_client = tcp_client;

        let message = match tcp_client.read::<client::Message>() {
            Ok(message) => message,
            Err(error) if error.is_disconnect() => {
                log::info!("Lobby connection closed before sign-in");
                return;
            }
            Err(error) => {
                log::error!("Unreadable lobby sign-in: {error}");
                return;
            }
        };
        let client::Message::SignInInfo { session_id } = message else {
            log::error!("First message must be 'SignInInfo', got {message:?}");
            return;
        };

        // The connection holds its session while it lasts. A session the
        // login server didn't open, or one that has ended (signed out, or
        // unseen for too long), is refused before `connection_successful`:
        // the client then only logs `invalid_session_id`
        // (`lobby_client::sign_in_on_packet_received`, 0x5C8146), and the
        // closed connection makes it reconnect every 5 s and, after three
        // failures, sign out and go back to the login screen
        // (`network_client::tick`, 0x5C6239).
        let hold = store.hold(session_id);
        let account = hold
            .as_ref()
            .and_then(|_| store.account_for_session(session_id));
        if account.is_none() && unknown == UnknownSessions::Refuse {
            log::warn!("Session {session_id:#x} is unknown or has ended; refused it");
            if let Err(error) = tcp_client.send(server::Message::InvalidSessionId) {
                log::info!("Session {session_id:#x} left before its refusal: {error}");
            }
            return;
        }

        if let Err(error) = tcp_client.send(server::Message::ConnectionSuccessful) {
            log::info!("Session {session_id:#x} left the lobby while signing in: {error}");
            return;
        }
        log::info!("Session {session_id:#x} connected to the lobby");

        // A lobby without a login server serves an unknown session as a
        // default account.
        let mut state = match account {
            Some(account) => ConnectionState::from_account(session_id, &account.locked()),
            None => {
                log::warn!("Unknown session {session_id:#x}; using a default account");
                ConnectionState::from_account(
                    session_id,
                    &Account::new_dummy(1, "dummy_name".to_string()),
                )
            }
        };
        let _hold = hold;
        if let (Ok(peer), Ok(reached)) = (tcp_client.peer_addr(), tcp_client.local_addr()) {
            let config = config::get();
            state.match_host =
                config
                    .match_server
                    .host_for(&config.lobby_server, peer.ip(), reached.ip());
        }

        let _queue = QueueGuard {
            store: &store,
            session_id,
        };
        // The place last shown on the queue screen.
        let mut told_place = None;

        loop {
            let message = match tcp_client.read::<client::Request>() {
                Ok(client::Request::Message(message)) => message,
                Ok(client::Request::Unreadable {
                    message_type,
                    error,
                }) => {
                    log::warn!(
                        "Session {session_id:#x}: skipped a lobby message of type                          {message_type:#x} srp cannot read: {error}"
                    );
                    // Refuse what the client waits on an answer for.
                    let refused = match message_type {
                        0x23 => Some(server::Operation::Inventory),
                        0x24 => Some(server::Operation::Shop(
                            server::ShopOperation::ActionBought {
                                dict_id: 0,
                                id: 0,
                                condition_or_stack: 0,
                            },
                        )),
                        0x25 => Some(server::Operation::SkillsTree),
                        _ => None,
                    };
                    if let Some(operation) = refused
                        && let Err(error) =
                            tcp_client.send(server::Message::OperationDenied(operation))
                    {
                        log::info!("Session {session_id:#x} disconnected from the lobby: {error}");
                        return;
                    }
                    continue;
                }
                Err(error) if error.is_disconnect() => {
                    log::info!("Session {session_id:#x} disconnected from the lobby");
                    return;
                }
                Err(error) => {
                    log::error!("{error}");
                    log::error!("{:?}", tcp_client.get_read_buffer());
                    log::error!("Session {session_id:#x}: dropping the lobby connection");
                    return;
                }
            };
            let listed = state.free_inventory();
            let response = self.handle_client_message(&store, &mut state, message);
            if let Some(notice) = state.reward_notice.take() {
                chat.tell_account(state.account_id, notice);
            }
            let Some(response) = response else {
                continue;
            };
            if response.is_sign_in_refusal() {
                log::error!("Session {session_id:#x}: not sending {response:?} after sign-in");
                continue;
            }
            // After an equip the client asks for the profile alone, so tell it
            // what the inventory has left as well (it takes the list any time).
            // Only when that changed: the list makes the client re-read every
            // profile, and its paper doll answers each with unequips for the
            // slots the armour closed, which would come back here again.
            let inventory_changed = matches!(
                response,
                server::Message::OperationPermitted(server::Operation::Inventory)
            ) && state.free_inventory() != listed;
            if let Err(error) = tcp_client.send(response) {
                log::info!("Session {session_id:#x} disconnected from the lobby: {error}");
                return;
            }
            // The queue screen shows the place the matchmaker last gave.
            let place = store.queue_place(session_id);
            if let Some(place) = place
                && told_place != Some(place)
            {
                chat.tell_queue_place(session_id, place);
            }
            told_place = place;
            if inventory_changed
                && let Err(error) = tcp_client.send(server::Message::ClientStatus(
                    server::ClientStatus::EnumerateInventory(state.free_inventory()),
                ))
            {
                log::info!("Session {session_id:#x} disconnected from the lobby: {error}");
                return;
            }
        }
    }

    /// Answer one message and keep what it changed in the session's account.
    pub fn handle_client_message(
        &self,
        store: &SessionStore,
        connection_state: &mut ConnectionState,
        msg: client::Message,
    ) -> Option<server::Message> {
        // What finished matches earned and used up, given at the first
        // message after the match server kept it: a player back from a match,
        // or one that left a match some other way and signed in again.
        let results = store.take_match_results(connection_state.session_id);
        if !results.is_empty() {
            let reward = rewards::apply(connection_state, &results);
            log::info!(
                "Session {:#x}: {} match result(s) gave {reward:?}",
                connection_state.session_id,
                results.len()
            );
            connection_state.reward_notice = Some(reward.notice(&connection_state.name));
        }
        let response = self.answer(store, connection_state, msg);
        // The client reconnects to the lobby after every match, and this
        // connection's state goes with the old connection.
        if let Some(account) = store.account_for_session(connection_state.session_id) {
            connection_state.save_to(&mut account.locked());
        }
        response
    }

    fn answer(
        &self,
        store: &SessionStore,
        connection_state: &mut ConnectionState,
        msg: client::Message,
    ) -> Option<server::Message> {
        log::debug!("[writer] Received {msg:?}");

        match msg {
            client::Message::ReadyForMatch { profile_id } => {
                let Some(selected_profile) = connection_state
                    .profile_contents
                    .iter()
                    .find(|profile| profile.profile_id == profile_id)
                    .copied()
                else {
                    log::warn!(
                        "Session {:#x} selected unknown profile {profile_id}",
                        connection_state.session_id
                    );
                    return Some(server::Message::OperationDenied(
                        server::Operation::Inventory,
                    ));
                };
                connection_state.selected_profile_id = profile_id;
                use survarium::player_profile::raw::profile_slot_enum;
                if [
                    profile_slot_enum::weapon1_slot,
                    profile_slot_enum::weapon2_slot,
                ]
                .into_iter()
                .all(|slot| selected_profile.slots[slot].id == 0)
                {
                    log::warn!(
                        "Session {:#x} selected a profile without a weapon",
                        connection_state.session_id
                    );
                    return Some(server::Message::OperationDenied(
                        server::Operation::Inventory,
                    ));
                }

                // Place this session into a match (balancing teams) and tell the
                // client where to go, or queue it. Persist the edited profiles
                // and selected loadout so the match server spawns exactly what
                // the user chose.
                let state = store.ready_for_match(
                    connection_state.session_id,
                    selected_profile,
                    &connection_state.profile_contents,
                );
                Some(queue_answer(connection_state, state))
            }

            // @TODO: Currently we allow all inventory actions :shrug:
            client::Message::InventoryAction(actions) if actions.is_empty() => Some(
                server::Message::OperationDenied(server::Operation::Inventory),
            ),

            client::Message::InventoryAction(actions) => {
                let mut state = (
                    connection_state.profile_contents.clone(),
                    connection_state.selected_profile_id,
                );
                for action in actions {
                    if let Err(reason) = equip(&connection_state.inventory, &mut state, action) {
                        log::warn!(
                            "Session {:#x}: inventory action refused: {reason}",
                            connection_state.session_id
                        );
                        return Some(server::Message::OperationDenied(
                            server::Operation::Inventory,
                        ));
                    }
                }
                (
                    connection_state.profile_contents,
                    connection_state.selected_profile_id,
                ) = state;

                Some(server::Message::OperationPermitted(
                    server::Operation::Inventory,
                ))
            }
            client::Message::SkillsTreeAction(action) => {
                log::warn!(
                    "Session {:#x}: skills tree changes are not supported; refused {action:?}",
                    connection_state.session_id
                );
                Some(server::Message::OperationDenied(
                    server::Operation::SkillsTree,
                ))
            }

            client::Message::SignInInfo { session_id } => {
                log::warn!(
                    "Session {:#x}: ignoring a repeated sign-in as {session_id:#x}",
                    connection_state.session_id
                );
                None
            }
            client::Message::DiscardPlayingOrder { order_id } => {
                // The client sends this on its next lobby sign-in after leaving a
                // match (whether it quit or the match finished). Free its
                // matchmaking slot so the next "ready" starts a fresh match. No
                // reply: the stock client does not wait for one.
                let released = store.leave_match(connection_state.session_id);
                log::info!(
                    "Session {:#x} discarded playing order {order_id:#x}; released {released:?}",
                    connection_state.session_id
                );
                None
            }
            // The client sends its clock and halves `now - answer` as an
            // unsigned number (`read_ping_server_answer`), so its own time back
            // gives the trip; anything later would wrap to billions.
            client::Message::PingServer { alive_ms } => Some(server::Message::PingServerAnswer {
                ping_value: alive_ms,
            }),

            client::Message::ShopAction(client::ShopAction::Buy {
                dict_id,
                amount,
                faction_id: _,
            }) => {
                use survarium::player_profile::{FULL_CONDITION, is_stack};

                // A stack costs its price for each one (the client's buy
                // dialog shows the count times the price; its ammo window,
                // clips times the price times the clip size); a piece, its
                // price. The client asks for the money again after a purchase
                // (`process_shop_action`), and shows what is left.
                let refused = server::Message::OperationDenied(server::Operation::Shop(
                    server::ShopOperation::ActionBought {
                        dict_id,
                        id: 0,
                        condition_or_stack: 0,
                    },
                ));
                let Some(price) = PRICES.iter().find(|price| price.dict_id == dict_id) else {
                    log::warn!(
                        "Session {:#x} tried to buy item {dict_id}, which is not for sale",
                        connection_state.session_id
                    );
                    return Some(refused);
                };
                let count = if is_stack(dict_id) { amount } else { 1 };
                let cost = u64::from(price.cost) * u64::from(count);
                if cost > u64::from(connection_state.generic_money) {
                    log::warn!(
                        "Session {:#x} cannot pay {cost} for {count} of item {dict_id}; it has {}",
                        connection_state.session_id,
                        connection_state.generic_money
                    );
                    return Some(refused);
                }
                // Fits: it is at most the money.
                connection_state.generic_money -= cost as u32;

                // A stack comes as its count; a piece new, one per purchase.
                let (condition_or_stack, amount_in_inventory) = if is_stack(dict_id) {
                    (amount, amount)
                } else {
                    if amount != 1 {
                        log::warn!(
                            "Session {:#x} bought {amount} of item {dict_id}; it gets one",
                            connection_state.session_id
                        );
                    }
                    (FULL_CONDITION, 1)
                };
                let op = server::Message::OperationPermitted(server::Operation::Shop(
                    server::ShopOperation::ActionBought {
                        dict_id,
                        id: connection_state.next_item_id,
                        condition_or_stack,
                    },
                ));
                // The client adds it to its inventory under this id
                // (`process_shop_action`), so it can be equipped by it.
                connection_state.inventory.push(
                    survarium::player_profile::raw::inventory_item_instance {
                        condition_or_stack,
                        amount_in_inventory,
                        id: connection_state.next_item_id,
                        dict_id,
                        padding: Default::default(),
                    },
                );
                connection_state.next_item_id += 1;
                Some(op)
            }

            // A queued client asks every second; it goes to its match as soon
            // as it has one.
            client::Message::QueryClientStatus(client::QueryClientStatus::ClientState) => {
                Some(match store.poll_queue(connection_state.session_id) {
                    Some(state) => queue_answer(connection_state, state),
                    None => server::Message::ClientStatus(server::ClientStatus::ClientState {
                        status: STATUS_IN_LOBBY,
                        last_status_message: None,
                    }),
                })
            }

            client::Message::QueryClientStatus(status) => {
                let status = match status {
                    client::QueryClientStatus::ClientState => {
                        unreachable!("answered above")
                    }
                    client::QueryClientStatus::EnumerateProfiles => {
                        server::ClientStatus::EnumerateProfiles(
                            connection_state
                                .profile_contents
                                .iter()
                                .map(|profile| server::Profile {
                                    profile_id: profile.profile_id,
                                    name: CStr::from_bytes_until_nul(&profile.profile_name)
                                        .unwrap()
                                        .to_string_lossy()
                                        .to_string(),
                                })
                                .collect(),
                        )
                    }

                    client::QueryClientStatus::ProfileContents { profile_id } => {
                        let Some(profile) = connection_state
                            .profile_contents
                            .iter()
                            .find(|profile| profile.profile_id == profile_id)
                        else {
                            log::warn!(
                                "Session {:#x} asked for unknown profile {profile_id}",
                                connection_state.session_id
                            );
                            return None;
                        };
                        server::ClientStatus::ProfileContents(Box::new(*profile))
                    }

                    client::QueryClientStatus::EnumerateInventory => {
                        server::ClientStatus::EnumerateInventory(connection_state.free_inventory())
                    }

                    // In which slot what type of weapon can be placed.
                    client::QueryClientStatus::ProfileSlotsRestrictions => {
                        server::ClientStatus::ProfileSlotsRestrictions(self.restricts.clone())
                    }

                    // Used to connect ammo and weapons
                    client::QueryClientStatus::ItemsCompatibility => {
                        server::ClientStatus::ItemsCompatibility(self.compats.clone())
                    }

                    client::QueryClientStatus::PriceItems(faction_id) => {
                        server::ClientStatus::PriceItems {
                            faction_id,
                            price_items: self
                                .price_items
                                .iter()
                                .find(|price_item| price_item.0 == faction_id)
                                .map(|price_item| price_item.1.clone())
                                .unwrap_or_default(),
                        }
                    }

                    client::QueryClientStatus::AccountMoney => server::ClientStatus::AccountMoney {
                        generic_money: connection_state.generic_money,
                        premium_money: connection_state.premium_money,
                        skill_points: connection_state.skills_points,
                        name: connection_state.name.clone(),
                    },

                    client::QueryClientStatus::PlayerSkills => server::ClientStatus::PlayerSkills {
                        total_experience: connection_state.total_experience,
                        next_level_experience: connection_state.next_level_experience,
                        prev_level_experience: connection_state.prev_level_experience,
                        player_skills: connection_state.player_skills,
                    },

                    client::QueryClientStatus::PlayerSkillsTree => {
                        server::ClientStatus::PlayerSkillsTree {
                            skills_tree: self.skills_tree.clone(),
                        }
                    }

                    client::QueryClientStatus::ServicePrices => {
                        server::ClientStatus::ServicePrices {
                            reroll_cost: self.reroll_cost,
                            add_profile_cost: self.add_profile_cost,
                            rename_account_cost: self.rename_account_cost,
                        }
                    }

                    client::QueryClientStatus::PlayerReputations => {
                        server::ClientStatus::PlayerReputations(connection_state.reps)
                    }
                };

                Some(server::Message::ClientStatus(status))
            }
        }
    }
}

/// Apply one `relocate_item_descr` to the profiles (with the selected
/// profile's id), as the client's lobby asked: `amount` is what the target
/// slot holds afterwards, its count for a stack.
fn equip(
    inventory: &[survarium::player_profile::raw::inventory_item_instance],
    (profiles, selected): &mut (Vec<survarium::player_profile::raw::player_profile>, u32),
    action: client::InventoryAction,
) -> Result<(), String> {
    use survarium::player_profile::{self, raw::inventory_item_instance};

    let client::InventoryAction::Equip {
        profile_id,
        id,
        dict_id,
        kind,
        amount,
    } = action;
    let profile = profiles
        .iter_mut()
        .find(|profile| profile.profile_id == profile_id)
        .ok_or(format!("no profile {profile_id}"))?;
    *selected = profile_id;
    match kind {
        client::EquipKind::Equip { to_slot } => {
            let owned = inventory
                .iter()
                .find(|item| item.id == id && item.dict_id == dict_id)
                .ok_or(format!("no item {id} ({dict_id}) in the inventory"))?;
            if player_profile::is_stack(dict_id) {
                // What the other slots leave of the stack.
                let elsewhere: u32 = profile
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|&(slot, item)| slot != to_slot as usize && item.id == id)
                    .map(|(_, item)| item.condition_or_stack)
                    .sum();
                let free = owned.condition_or_stack.saturating_sub(elsewhere);
                let count = u32::from(amount).min(free);
                if count == 0 {
                    return Err(format!("none of item {id} left to equip"));
                }
                profile.slots[to_slot] = inventory_item_instance {
                    condition_or_stack: count,
                    amount_in_inventory: owned.condition_or_stack,
                    ..*owned
                };
            } else {
                // One piece goes in one slot, in the condition it is in.
                for slot in profile.slots.iter_mut().filter(|slot| slot.id == id) {
                    *slot = inventory_item_instance::default();
                }
                profile.slots[to_slot] = *owned;
            }
        }
        client::EquipKind::Unequip { from_slot } => {
            profile.slots[from_slot] = inventory_item_instance::default();
        }
        client::EquipKind::Move { from_slot, to_slot } => {
            let item = profile.slots[from_slot];
            if item.id == 0 {
                return Err(format!("nothing in slot {from_slot:?} to move"));
            }
            profile.slots[from_slot] = inventory_item_instance::default();
            profile.slots[to_slot] = item;
        }
    }
    Ok(())
}

impl ServerState {
    pub fn new_dummy() -> Self {
        Self {
            restricts: {
                // As `docs/notes/restrictions.md` lists them, but for its
                // slots 19-22, which this client's profiles don't have.
                #[rustfmt::skip]
                let restricts: [(u8, u8); 49] = [
                    // (category_id, profile_slot_id)
                    (1, 0),
                    (2, 1),
                    (3, 2),
                    (4, 3),
                    (5, 4),
                    (6, 5),
                    (7, 6),
                    // Ammo, in the two ammo slots of either weapon.
                    (9, 8), (9, 9), (9, 11), (9, 12),
                    (18, 8), (18, 9), (18, 11), (18, 12),
                    (19, 8), (19, 9), (19, 11), (19, 12),
                    (20, 8), (20, 9), (20, 11), (20, 12),
                    (21, 8), (21, 9), (21, 11), (21, 12),
                    // Meds and traps, in the quick slots.
                    (10, 13), (10, 14), (10, 15), (10, 16), (10, 17), (10, 18),
                    (11, 13), (11, 14), (11, 15), (11, 16), (11, 17), (11, 18),
                    // Weapons.
                    (13, 7), (13, 10),
                    (14, 7), (14, 10),
                    (15, 7), (15, 10),
                    (16, 7), (16, 10),
                    (17, 7), (17, 10),
                ];

                restricts
                    .into_iter()
                    .map(
                        |(category_dict_id, slot_dict_id)| server::raw::profile_slot_restriction {
                            slot_dict_id,
                            category_dict_id,
                        },
                    )
                    .collect()
            },
            compats: {
                let compats: [(u16, u16); 18] = [
                    (12, 51),
                    (12, 52),
                    (13, 7),
                    (14, 51),
                    (14, 52),
                    (15, 22),
                    (15, 72),
                    (16, 22),
                    (16, 72),
                    (17, 22),
                    (17, 72),
                    (18, 50),
                    (19, 53),
                    (19, 71),
                    (55, 53),
                    (55, 71),
                    (56, 20),
                    (64, 70),
                ];

                compats
                    .into_iter()
                    .map(|(first_item_dict_id, second_item_dict_id)| {
                        server::raw::items_compatibility {
                            first_item_dict_id,
                            second_item_dict_id,
                        }
                    })
                    .collect()
            },
            price_items: {
                let shop = |faction| {
                    PRICES
                        .iter()
                        .filter(|price| price.sellers.contains(&faction))
                        .map(|price| server::raw::price_item {
                            item_dict_id: price.dict_id,
                            cost: price.cost,
                            // Reputation was off in this alpha: everything
                            // could be bought from the start.
                            reputation_level: 0,
                            padding: Default::default(),
                        })
                        .collect()
                };
                [
                    (faction_id::scavengers, shop(faction_id::scavengers)),
                    (faction_id::black_market, shop(faction_id::black_market)),
                    // @NOTE: in 001b are not supported
                    (faction_id::army, Vec::new()),
                    (faction_id::fringe_settlers, Vec::new()),
                ]
            },
            skills_tree: include_bytes!(
                "../../../../resources/binary-config-examples/skills_tree.bin"
            )
            .to_vec(),

            reroll_cost: 100,
            add_profile_cost: 200,
            rename_account_cost: 300,
        }
    }
}

/// What a client that asked for a match hears: where to go, or that it is
/// waiting in the queue.
fn queue_answer(connection: &ConnectionState, state: QueueState) -> server::Message {
    let session_id = connection.session_id;
    match state {
        QueueState::Placed(assignment) => {
            log::info!(
                "Session {session_id:#x} -> match {:#x} team {}",
                assignment.match_id,
                assignment.team_id
            );
            server::Message::ConnectToMatchServer {
                host: connection.match_host.clone(),
                match_id: assignment.match_id,
                team_id: assignment.team_id,
            }
        }
        QueueState::Waiting { place } => {
            log::debug!("Session {session_id:#x} waits for a match, place {place}");
            server::Message::ClientStatus(server::ClientStatus::ClientState {
                status: STATUS_QUEUED,
                last_status_message: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use survarium::player_profile::raw::profile_slot_enum;

    #[test]
    fn a_bought_piece_is_new_and_a_bought_stack_counts() {
        use survarium::account::faction_id;

        let store = SessionStore::new();
        let account = Account::new_dummy(1, "demo_player".to_owned());
        let mut connection = ConnectionState::from_account(0xDD00, &account);
        let server = ServerState::new_dummy();
        let mut buy = |dict_id, amount| {
            let response = server.handle_client_message(
                &store,
                &mut connection,
                client::Message::ShopAction(client::ShopAction::Buy {
                    dict_id,
                    amount,
                    faction_id: faction_id::scavengers,
                }),
            );
            let Some(server::Message::OperationPermitted(server::Operation::Shop(
                server::ShopOperation::ActionBought {
                    condition_or_stack, ..
                },
            ))) = response
            else {
                panic!("purchase refused: {response:?}");
            };
            condition_or_stack
        };
        assert_eq!(buy(31, 1), 100, "a torso, new");
        assert_eq!(buy(7, 30), 30, "30 rounds of 5.45");

        let bought: Vec<(u16, u32, u32)> = connection.inventory[connection.inventory.len() - 2..]
            .iter()
            .map(|item| {
                (
                    item.dict_id,
                    item.condition_or_stack,
                    item.amount_in_inventory,
                )
            })
            .collect();
        assert_eq!(bought, [(31, 100, 1), (7, 30, 30)]);
    }

    #[test]
    fn the_starting_loadout_follows_the_slot_rules() {
        // Its items' categories (`item_category`, `docs/notes/restrictions.md`).
        let category = |dict_id| match dict_id {
            27 => 1,
            43 => 2,
            48 => 3,
            9 => 4,
            47 => 5,
            40 => 6,
            24 => 7,
            7 => 9,
            66 => 10,
            68 => 11,
            14 => 13,
            13 => 15,
            51 | 52 => 18,
            _ => panic!("no category for item {dict_id}"),
        };
        let restricts = ServerState::new_dummy().restricts;
        let profile = survarium::player_profile::raw::player_profile::new_dummy(1, 10, "selected");
        for (slot, item) in profile.slots.iter().enumerate() {
            if item.id == 0 {
                continue;
            }
            assert!(
                restricts.iter().any(|rule| {
                    usize::from(rule.slot_dict_id) == slot
                        && rule.category_dict_id == category(item.dict_id)
                }),
                "item {} may not go in slot {slot}",
                item.dict_id
            );
        }
    }

    #[test]
    fn ready_for_match_preserves_the_chosen_lobby_loadout() {
        let store = SessionStore::new();
        let account = Account::new_dummy(1, "demo_player".to_owned());
        let mut connection = ConnectionState::from_account(0xDD00, &account);
        let selected_id = connection.profile_contents[1].profile_id;
        connection.profile_contents[1].slots[profile_slot_enum::weapon1_slot].dict_id = 55;

        let response = ServerState::new_dummy().handle_client_message(
            &store,
            &mut connection,
            client::Message::ReadyForMatch {
                profile_id: selected_id,
            },
        );

        assert!(matches!(
            response,
            Some(server::Message::ConnectToMatchServer { .. })
        ));
        let selected = store.selected_profile(0xDD00).unwrap();
        assert_eq!(selected.profile_id, selected_id);
        assert_eq!(selected.slots[profile_slot_enum::weapon1_slot].dict_id, 55);
    }

    #[test]
    fn discard_playing_order_releases_the_match_slot() {
        let store = SessionStore::new();
        let account = Account::new_dummy(1, "demo_player".to_owned());
        let mut connection = ConnectionState::from_account(0xDD00, &account);
        let profile_id = connection.profile_contents[1].profile_id;
        let state = ServerState::new_dummy();

        state.handle_client_message(
            &store,
            &mut connection,
            client::Message::ReadyForMatch { profile_id },
        );
        assert!(store.match_assignment(0xDD00).is_some());

        let response = state.handle_client_message(
            &store,
            &mut connection,
            client::Message::DiscardPlayingOrder { order_id: u32::MAX },
        );
        assert!(response.is_none());
        assert_eq!(store.match_assignment(0xDD00), None);
    }

    fn equip(
        connection: &mut ConnectionState,
        dict_id: u16,
        kind: client::EquipKind,
        amount: u16,
    ) -> Option<server::Message> {
        let profile_id = connection.profile_contents[0].profile_id;
        ServerState::new_dummy().handle_client_message(
            &SessionStore::new(),
            connection,
            client::Message::InventoryAction(vec![client::InventoryAction::Equip {
                profile_id,
                id: survarium::player_profile::demo_item_id(dict_id),
                dict_id,
                kind,
                amount,
            }]),
        )
    }

    fn free(connection: &ConnectionState, dict_id: u16) -> Option<u32> {
        connection
            .free_inventory()
            .iter()
            .find(|item| item.dict_id == dict_id)
            .map(|item| item.condition_or_stack)
    }

    #[test]
    fn equipping_part_of_a_stack_leaves_the_rest_in_the_inventory() {
        let account = Account::new_dummy(1, "demo_player".to_owned());
        let mut connection = ConnectionState::from_account(0xDD00, &account);
        // The dummy loadout has 2 of the 100 traps in quick slot 4.
        assert_eq!(free(&connection, 68), Some(98));

        let response = equip(
            &mut connection,
            68,
            client::EquipKind::Equip {
                to_slot: profile_slot_enum::quick_slot3,
            },
            5,
        );
        assert!(matches!(
            response,
            Some(server::Message::OperationPermitted(
                server::Operation::Inventory
            ))
        ));
        let slot = connection.profile_contents[0].slots[profile_slot_enum::quick_slot3];
        assert_eq!((slot.dict_id, slot.condition_or_stack), (68, 5));
        assert_eq!(free(&connection, 68), Some(93));

        // No more than the stack holds.
        equip(
            &mut connection,
            68,
            client::EquipKind::Equip {
                to_slot: profile_slot_enum::quick_slot3,
            },
            500,
        );
        let slot = connection.profile_contents[0].slots[profile_slot_enum::quick_slot3];
        assert_eq!(slot.condition_or_stack, 98);
        assert_eq!(free(&connection, 68), Some(0));

        equip(
            &mut connection,
            68,
            client::EquipKind::Unequip {
                from_slot: profile_slot_enum::quick_slot3,
            },
            98,
        );
        assert_eq!(free(&connection, 68), Some(98));
    }

    #[test]
    fn equipping_a_piece_keeps_its_condition_and_takes_it_from_its_old_slot() {
        let account = Account::new_dummy(1, "demo_player".to_owned());
        let mut connection = ConnectionState::from_account(0xDD00, &account);
        // The AK-74u goes from the first weapon slot to the second.
        equip(
            &mut connection,
            13,
            client::EquipKind::Equip {
                to_slot: profile_slot_enum::weapon2_slot,
            },
            1,
        );
        let slots = connection.profile_contents[0].slots;
        assert_eq!(slots[profile_slot_enum::weapon1_slot].id, 0);
        let weapon = slots[profile_slot_enum::weapon2_slot];
        assert_eq!((weapon.dict_id, weapon.condition_or_stack), (13, 100));
        assert_eq!(free(&connection, 13), Some(100), "still listed");
    }

    #[test]
    fn an_item_not_in_the_inventory_cannot_be_equipped() {
        let account = Account::new_dummy(1, "demo_player".to_owned());
        let mut connection = ConnectionState::from_account(0xDD00, &account);
        let response = equip(
            &mut connection,
            999,
            client::EquipKind::Equip {
                to_slot: profile_slot_enum::quick_slot3,
            },
            1,
        );
        assert!(matches!(
            response,
            Some(server::Message::OperationDenied(
                server::Operation::Inventory
            ))
        ));
    }

    #[test]
    fn purchases_and_loadouts_outlast_the_lobby_connection() {
        use std::net::{IpAddr, Ipv4Addr};
        use survarium::account::faction_id;

        let store = SessionStore::new();
        let account_id = store.get_or_create_account(IpAddr::V4(Ipv4Addr::LOCALHOST), "buyer");
        let session_id = store.create_session(account_id);
        let account = store.account_for_session(session_id).unwrap();
        let server = ServerState::new_dummy();
        let buy = |connection: &mut ConnectionState| {
            let response = server.handle_client_message(
                &store,
                connection,
                client::Message::ShopAction(client::ShopAction::Buy {
                    dict_id: 31,
                    amount: 1,
                    faction_id: faction_id::scavengers,
                }),
            );
            let Some(server::Message::OperationPermitted(server::Operation::Shop(
                server::ShopOperation::ActionBought { id, .. },
            ))) = response
            else {
                panic!("purchase refused: {response:?}");
            };
            id
        };

        let mut first = ConnectionState::from_account(session_id, &account.locked());
        let bought = buy(&mut first);
        let profile_id = first.profile_contents[0].profile_id;
        let equipped = server.handle_client_message(
            &store,
            &mut first,
            client::Message::InventoryAction(vec![client::InventoryAction::Equip {
                profile_id,
                id: bought,
                dict_id: 31,
                kind: client::EquipKind::Equip {
                    to_slot: profile_slot_enum::torso_slot,
                },
                amount: 100,
            }]),
        );
        assert!(matches!(
            equipped,
            Some(server::Message::OperationPermitted(
                server::Operation::Inventory
            ))
        ));
        drop(first);

        // The client signs in to the lobby again after its match.
        let mut second = ConnectionState::from_account(session_id, &account.locked());
        assert!(second.inventory.iter().any(|item| item.id == bought));
        let profile = &second.profile_contents[0];
        assert_eq!(profile.slots[profile_slot_enum::torso_slot].id, bought);
        assert_eq!(buy(&mut second), bought + 1, "an id is never reused");
    }

    fn buy(connection: &mut ConnectionState, dict_id: u16, amount: u32) -> Option<server::Message> {
        ServerState::new_dummy().handle_client_message(
            &SessionStore::new(),
            connection,
            client::Message::ShopAction(client::ShopAction::Buy {
                dict_id,
                amount,
                faction_id: survarium::account::faction_id::black_market,
            }),
        )
    }

    #[test]
    fn a_purchase_takes_its_price() {
        let account = Account::new_dummy(1, "demo_player".to_owned());
        let mut connection = ConnectionState::from_account(0xDD00, &account);
        connection.generic_money = 1000;

        assert!(matches!(
            buy(&mut connection, 13, 1),
            Some(server::Message::OperationPermitted(_))
        ));
        assert_eq!(connection.generic_money, 650, "an AKS-74U");
        // Two clips of 5.45, as the ammo window sends them.
        assert!(matches!(
            buy(&mut connection, 7, 60),
            Some(server::Message::OperationPermitted(_))
        ));
        assert_eq!(connection.generic_money, 590, "60 rounds");
        // A piece is one, whatever the count.
        buy(&mut connection, 27, 3);
        assert_eq!(connection.generic_money, 540, "a biker's helmet");
    }

    #[test]
    fn a_purchase_beyond_the_money_or_off_the_list_is_refused() {
        let account = Account::new_dummy(1, "demo_player".to_owned());
        let mut connection = ConnectionState::from_account(0xDD00, &account);
        connection.generic_money = 300;
        let items = connection.inventory.len();

        for (dict_id, amount) in [(13, 1), (7, 301), (69, 1), (7, u32::MAX)] {
            assert!(
                matches!(
                    buy(&mut connection, dict_id, amount),
                    Some(server::Message::OperationDenied(server::Operation::Shop(_)))
                ),
                "{dict_id} x {amount}"
            );
        }
        assert_eq!(connection.generic_money, 300);
        assert_eq!(connection.inventory.len(), items);
    }

    #[test]
    fn the_money_left_outlasts_the_lobby_connection() {
        use std::net::{IpAddr, Ipv4Addr};

        let store = SessionStore::new();
        let account_id = store.get_or_create_account(IpAddr::V4(Ipv4Addr::LOCALHOST), "buyer");
        let session_id = store.create_session(account_id);
        let account = store.account_for_session(session_id).unwrap();
        let before = account.locked().generic_money;

        let mut first = ConnectionState::from_account(session_id, &account.locked());
        ServerState::new_dummy().handle_client_message(
            &store,
            &mut first,
            client::Message::ShopAction(client::ShopAction::Buy {
                dict_id: 13,
                amount: 1,
                faction_id: survarium::account::faction_id::black_market,
            }),
        );
        drop(first);

        let second = ConnectionState::from_account(session_id, &account.locked());
        assert_eq!(second.generic_money, before - 350);
    }

    #[test]
    fn a_queued_player_waits_on_the_queue_screen_until_the_match_starts() {
        let store = SessionStore::with_queue_min_players(2);
        let server = ServerState::new_dummy();
        let connect = |email: &str| {
            let account_id =
                store.get_or_create_account(std::net::IpAddr::from([127, 0, 0, 1]), email);
            ConnectionState::from_account(
                store.create_session(account_id),
                &Account::new_dummy(1, email.to_owned()),
            )
        };
        let mut first = connect("first");
        let mut second = connect("second");
        let ready = |connection: &mut ConnectionState| {
            let profile_id = connection.profile_contents[0].profile_id;
            server.handle_client_message(
                &store,
                connection,
                client::Message::ReadyForMatch { profile_id },
            )
        };
        let poll = |connection: &mut ConnectionState| {
            server.handle_client_message(
                &store,
                connection,
                client::Message::QueryClientStatus(client::QueryClientStatus::ClientState),
            )
        };
        let queued = |answer: Option<server::Message>| {
            matches!(
                answer,
                Some(server::Message::ClientStatus(
                    server::ClientStatus::ClientState {
                        status: STATUS_QUEUED,
                        ..
                    }
                ))
            )
        };
        let sent_to_match = |answer: Option<server::Message>| {
            matches!(answer, Some(server::Message::ConnectToMatchServer { .. }))
        };

        assert!(queued(ready(&mut first)));
        assert!(queued(poll(&mut first)));
        assert!(sent_to_match(ready(&mut second)));
        assert!(sent_to_match(poll(&mut first)));
        // Back to the lobby's own status afterwards.
        assert!(matches!(
            poll(&mut first),
            Some(server::Message::ClientStatus(
                server::ClientStatus::ClientState {
                    status: STATUS_IN_LOBBY,
                    ..
                }
            ))
        ));
    }

    #[test]
    fn a_match_result_is_given_at_the_next_message_and_kept() {
        let store = SessionStore::new();
        let account_id =
            store.get_or_create_account(std::net::IpAddr::from([127, 0, 0, 1]), "veteran");
        let session_id = store.create_session(account_id);
        let account = store.account_for_session(session_id).unwrap();
        let money = account.locked().generic_money;
        store.record_match_result(
            session_id,
            session::MatchResult {
                kills: 2,
                finished: true,
                ..session::MatchResult::default()
            },
        );

        let mut connection = ConnectionState::from_account(session_id, &account.locked());
        ServerState::new_dummy().handle_client_message(
            &store,
            &mut connection,
            client::Message::PingServer { alive_ms: 1 },
        );

        let notice = connection.reward_notice.take().unwrap();
        assert!(notice.starts_with("Player [ veteran ] "), "{notice}");
        assert!(notice.ends_with("#e:[20]"), "{notice}");
        let account = account.locked();
        assert_eq!(account.generic_money, money + 100 + 200);
        assert_eq!(account.total_experience, 20);
        assert!(
            store.take_match_results(session_id).is_empty(),
            "given once"
        );
    }

    #[test]
    fn the_shops_sell_at_the_2013_prices() {
        let state = ServerState::new_dummy();
        let shop = |faction| {
            state
                .price_items
                .iter()
                .find(|(seller, _)| *seller == faction)
                .map(|(_, items)| {
                    items
                        .iter()
                        .map(|item| (item.item_dict_id, item.cost))
                        .collect::<Vec<_>>()
                })
                .unwrap()
        };
        let scavengers = shop(faction_id::scavengers);
        let black_market = shop(faction_id::black_market);
        assert!(scavengers.contains(&(64, 100)), "Fort 17");
        assert!(!black_market.iter().any(|(id, _)| *id == 64));
        assert!(black_market.contains(&(13, 350)), "AKS-74U");
        assert!(black_market.contains(&(7, 1)), "a round of 5.45x39");
        assert!(scavengers.contains(&(18, 10)) && black_market.contains(&(18, 10)));
        // The client divides the money by the price for what it can buy.
        assert!(PRICES.iter().all(|price| price.cost > 0));
        // Each item once; never the scope or the lifebone.
        let mut ids: Vec<u16> = PRICES.iter().map(|price| price.dict_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PRICES.len());
        assert!(!ids.iter().any(|id| [54, 57, 69].contains(id)));
    }

    #[test]
    fn the_ping_answer_is_the_clients_own_time() {
        let account = Account::new_dummy(1, "demo_player".to_owned());
        let mut connection = ConnectionState::from_account(0xDD00, &account);
        let response = ServerState::new_dummy().handle_client_message(
            &SessionStore::new(),
            &mut connection,
            client::Message::PingServer { alive_ms: 123_456 },
        );
        assert!(matches!(
            response,
            Some(server::Message::PingServerAnswer {
                ping_value: 123_456
            })
        ));
    }
}
