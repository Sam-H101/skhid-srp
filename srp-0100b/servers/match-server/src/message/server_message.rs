use survarium::player_input::{
    player, player_input, player_inventory_slot, player_stamina, player_state, weapon_core,
    weapon_core_state, weapon_state,
};
use survarium::player_profile::raw::player_profile;
use vostok::math::float3;
use vostok::network_client::NetworkResponse;
use vostok::network_packet::Packet;
use vostok::serde::Serialize;

use self::raw::*;
use crate::message::raw::{low_level_message_type_enum, udp_match_packets_count_enum};
use crate::message::{
    BodyPartState, PlayerHit, PlayerKill, VictoryItemAction, VictoryItemPlacement,
    VictoryItemsContainerContents,
};
use crate::sequence_number::SN16;

#[derive(Debug, PartialEq, Clone)]
pub struct ServerMessage {
    /// Sequential id of this message
    pub remote_sequence_id: SN16,
    /// TODO: ?
    pub local_sequence_id: SN16,
    /// TODO: Currently we only can send a single packet.
    /// So this means that the first bit in `local_ack_bits` should always be 0
    pub local_ack_bits: u16,

    pub kind: ServerMessageKind,
}

#[derive(Debug, PartialEq, Clone)]
pub enum ServerMessageKind {
    Low(low_level_message_type_enum),
    Messages(Vec<ServerGameMessage>),
}

#[derive(Debug, PartialEq, Clone)]
pub struct ServerGameMessage {
    pub order_id: SN16,
    pub game_message: ServerGameMessageKind,
}

#[derive(Debug, PartialEq, Clone)]
pub enum ServerGameMessageKind {
    ConnectionSuccessful,
    MatchOptions {
        map_id: u8,
        // 32 chars max
        map_name: String,
        match_mode: game_mode_type,
        player_count: u8,
        victory_item_count: u8,
        respawn_time: u8,
        match_time: u16,
    },
    SpawnPlayer {
        player_id: u8,
        player: player,
    },
    ServerPlayerInput {
        time_in_ms: u32,
        player_id: u8,
        player_input: player_input,
        player_state: player_state,
        weapon_state: weapon_state,
    },
    SyncResponse {
        is_connected_bitmask: u32,
    },
    PlayerVisibilityChanged {
        player_id: u8,
        is_visible: bool,
    },

    PlayerProfile {
        player_profile: Box<player_profile>,
    },
    HitPlayer(PlayerHit),
    KillPlayer(PlayerKill),
    /// `process_affect_damage_model`: an affect (`hit_affects_type_enum`)
    /// applied to or recalled from one body part. Only the server's copy of a
    /// remote player applies affects, so other clients learn them from this.
    AffectDamageModel {
        player_id: u8,
        body_part: String,
        affect: u32,
        /// `affect_event_type_enum`: 0 applying, 1 recalling.
        event: u32,
    },

    // Match lifecycle. Timer units follow the client's `game_world_ui` setters.
    GameStatusChanged {
        status: game_status,
    },
    /// Pre-match timer in whole seconds; the client labels it by the current
    /// status (waiting for players / final countdown).
    MatchWaitTimeChanged {
        time_left: u32,
    },
    /// Match timer in milliseconds; `0` shows the "time is up" text.
    MatchTimeChanged {
        time_left_ms: u32,
    },
    /// Sent to the dead player only, in whole seconds; `0` clears it.
    RespawnTimeChanged {
        time_left: u32,
    },
    PlayerKdStatsChanged {
        player_id: u8,
        kills: u32,
        deaths: u32,
    },
    /// The client leaves the match and returns to the lobby.
    MatchFinished,

    // Battery collection (`gather_victory_items`).
    /// The whole battery state. The client places ground batteries without
    /// removing them first, so each client gets this once per match.
    InitializeVictoryItems {
        team_1_points: i8,
        team_2_points: i8,
        items: Vec<VictoryItemPlacement>,
        containers: Vec<VictoryItemsContainerContents>,
    },
    VictoryItemTakeOrPut {
        player_id: u8,
        item_id: u8,
        action: VictoryItemAction,
    },

    // Traps (`booby_trap_set`): the server places and runs them all.
    /// `network_client::on_trap_placed`: trap `index` of the set in
    /// `player_id`'s quick slot `slot` (a `profile_slot_enum`) is armed at
    /// `create_rotation( angles ) * create_translation( position )`. The
    /// client takes one off the set's count. It checks none of the ids.
    TrapPlaced {
        player_id: u8,
        slot: u8,
        index: u8,
        position: float3,
        angles: float3,
    },
    /// That trap goes back in its set (`booby_trap_set_core::remove_trap`).
    TrapRemoved {
        player_id: u8,
        slot: u8,
        index: u8,
    },
    /// That trap went off: the client shows its fired model and plays its
    /// effects. The damage comes in `HitPlayer`s.
    TrapFired {
        player_id: u8,
        slot: u8,
        index: u8,
    },
    /// That trap was disarmed: the client shows its disarmed model.
    TrapDisarmed {
        player_id: u8,
        slot: u8,
        index: u8,
    },
    /// A trap already out, for a client that joins later
    /// (`booby_trap_core::deserialize`): it inserts the trap, then shows it
    /// in `state` (a `booby_trap_state`). The set's count stays as it is.
    GameWorldObjectState {
        player_id: u8,
        slot: u8,
        index: u8,
        state: u8,
        position: float3,
        angles: float3,
    },
    /// `network_client::damage_model_state_arrived`: a player's whole damage
    /// model (`damage_model::deserialize`), each body part in the client's
    /// order. There is no part count: the client reads one entry per part
    /// of its own model. A spawn resets the model (`insert_alive`), so this
    /// follows the player's spawn.
    DamageModelState {
        player_id: u8,
        parts: Vec<BodyPartState>,
    },
    /// `network_client::on_world_sync_request` (0x5C5D60), no body: the
    /// client drops its queued updates, takes every player in its world out
    /// (`player::remove`, which takes their traps with them) with the
    /// batteries they carry, empties the bases and picks up the batteries on
    /// the ground, then answers `world_synchronization_confirmation` (0x4A).
    /// It keeps its roster and its scores.
    WorldSynchronizationRequest,
}

/// The container id the client reads as "no base: the ground".
const NO_CONTAINER: u8 = 0xFF;

pub mod raw {
    #![expect(non_camel_case_types)]
    #![expect(dead_code)]

    #[repr(u8)]
    #[derive(bytemuck::CheckedBitPattern, bytemuck::NoUninit, Copy, Clone, Debug, PartialEq)]
    #[rustfmt::skip]
    pub enum match_server_message_types_enum {
        match_server_connection_successful = 0x80, // changes connection_type to connected
        match_options_message_type         = 0x81, // [+] - survarium::network_client::on_match_packet_received
        server_player_input                = 0x82, //
        kill_player                        = 0x83, //
        spawn_player                       = 0x84, //
        team_base_capture_progress         = 0x85, //
        match_time_changed                 = 0x86, //
        respawn_time_changed               = 0x87, //
        player_kd_stats_changed            = 0x88, //
        hit_player                         = 0x89, //
        affect_damage_model                = 0x8A, //
        sync_response                      = 0x8B, //
        match_finished                     = 0x8C, //
        server_bullet_added                = 0x8D, //
        server_bullet_removed              = 0x8E, //
        server_bullet_moved                = 0x8F, //
        server_bullet_collided             = 0x90, //
        player_visibility_changed          = 0x91, //
        player_profile_message_type        = 0x92,
        team_bases_message_type            = 0x93,
        initialize_victory_items           = 0x94,
        victory_item_take_or_put           = 0x95,
        trap_placed                        = 0x96,
        trap_removed                       = 0x97,
        trap_fired                         = 0x98,
        trap_disarmed                      = 0x99,
        game_status_changed                = 0x9A,
        match_wait_time_changed            = 0x9B,
        game_world_object_state            = 0x9C,
        world_synchronization_request      = 0x9D,
        damage_model_state                 = 0x9E, // [+] hidden
        match_server_invalid_message_type  = 0xC0,
    }

    #[repr(u8)]
    #[derive(bytemuck::CheckedBitPattern, bytemuck::NoUninit, Copy, Clone, Debug, PartialEq)]
    #[rustfmt::skip]
    pub enum game_mode_type {
        capture_enemy_base   = 0x0,
        capture_neutral_base = 0x1,
        gather_victory_items = 0x2,
        invalid_game_mode    = 0xFF,
    }

    #[repr(u32)]
    #[derive(bytemuck::CheckedBitPattern, bytemuck::NoUninit, Copy, Clone, Debug, PartialEq)]
    #[rustfmt::skip]
    pub enum game_status {
        inactive                 = 0x0,
        waiting_for_first_player = 0x1,
        waiting_for_players      = 0x2,
        final_countdown          = 0x3,
        inprocess                = 0x4,
    }
}

impl NetworkResponse for ServerMessage {}

impl Serialize for ServerMessage {
    fn serialize(self, packet: &mut impl Packet) {
        packet.write(self.remote_sequence_id);
        packet.write(self.local_sequence_id);

        let match_packets_count = match &self.kind {
            ServerMessageKind::Low(_) => udp_match_packets_count_enum::multiple_packets,
            ServerMessageKind::Messages(messages) if messages.len() == 1 => {
                udp_match_packets_count_enum::single_packet
            }
            _ => udp_match_packets_count_enum::multiple_packets,
        } as u16;
        packet.write(self.local_ack_bits << 1 | match_packets_count);

        self.kind.serialize(packet);
    }
}

impl Serialize for ServerMessageKind {
    fn serialize(self, packet: &mut impl Packet) {
        match self {
            Self::Low(message_type) => {
                packet.write(1_u8); // low level message type size
                packet.write(message_type)
            }
            Self::Messages(mut messages) if messages.len() == 1 => {
                let message = messages.pop().unwrap();
                message.serialize(packet);
            }

            Self::Messages(messages) => {
                for message in messages {
                    packet.write(0_u8);
                    let len_index = packet.cursor_index();

                    message.serialize(packet);

                    let end_packet_idx = packet.cursor_index();
                    let packet_len = end_packet_idx - len_index;

                    // len     packet
                    // [0] [1, 2, 3, 4, 5]
                    //      ^             ^

                    let packet_len: u8 = packet_len.try_into().unwrap();
                    packet.rewrite_bytes(len_index - 1, [packet_len]);
                }
            }
        }
    }
}

impl Serialize for ServerGameMessage {
    fn serialize(self, packet: &mut impl Packet) {
        let Self {
            order_id,
            game_message,
        } = self;

        packet.write(game_message.message_type());
        packet.write(order_id);
        game_message.serialize(packet);
    }
}

impl ServerGameMessageKind {
    #[rustfmt::skip]
    pub fn message_type(&self) -> match_server_message_types_enum {
        match self {
            Self::ConnectionSuccessful { .. } => match_server_message_types_enum::match_server_connection_successful,
            Self::MatchOptions { .. }         => match_server_message_types_enum::match_options_message_type,
            Self::SpawnPlayer { .. }          => match_server_message_types_enum::spawn_player,
            Self::ServerPlayerInput { .. }    => match_server_message_types_enum::server_player_input,
            Self::SyncResponse { .. }         => match_server_message_types_enum::sync_response,
            Self::PlayerVisibilityChanged { .. } => match_server_message_types_enum::player_visibility_changed,
            Self::PlayerProfile { .. }        => match_server_message_types_enum::player_profile_message_type,
            Self::HitPlayer(..)               => match_server_message_types_enum::hit_player,
            Self::KillPlayer(..)              => match_server_message_types_enum::kill_player,
            Self::AffectDamageModel { .. }    => match_server_message_types_enum::affect_damage_model,
            Self::GameStatusChanged { .. }    => match_server_message_types_enum::game_status_changed,
            Self::MatchWaitTimeChanged { .. } => match_server_message_types_enum::match_wait_time_changed,
            Self::MatchTimeChanged { .. }     => match_server_message_types_enum::match_time_changed,
            Self::RespawnTimeChanged { .. }   => match_server_message_types_enum::respawn_time_changed,
            Self::PlayerKdStatsChanged { .. } => match_server_message_types_enum::player_kd_stats_changed,
            Self::MatchFinished               => match_server_message_types_enum::match_finished,
            Self::InitializeVictoryItems { .. } => match_server_message_types_enum::initialize_victory_items,
            Self::VictoryItemTakeOrPut { .. } => match_server_message_types_enum::victory_item_take_or_put,
            Self::TrapPlaced { .. }           => match_server_message_types_enum::trap_placed,
            Self::TrapRemoved { .. }          => match_server_message_types_enum::trap_removed,
            Self::TrapFired { .. }            => match_server_message_types_enum::trap_fired,
            Self::TrapDisarmed { .. }         => match_server_message_types_enum::trap_disarmed,
            Self::GameWorldObjectState { .. } => match_server_message_types_enum::game_world_object_state,
            Self::DamageModelState { .. }     => match_server_message_types_enum::damage_model_state,
            Self::WorldSynchronizationRequest => match_server_message_types_enum::world_synchronization_request,
        }
    }

    /// Whether this is about what stands in a client's world: players,
    /// their moves, hits and affects, batteries and traps. A client that has
    /// cleared its world for a `world_synchronization_request` has none of
    /// it until the server puts it back, and handles most of these against
    /// players it took out; the world as it is then supersedes them.
    /// `kill_player` checks that its victim is in the world and alive
    /// (`process_player_kill`), and still adds the kill feed line.
    pub fn is_world_state(&self) -> bool {
        matches!(
            self,
            Self::SpawnPlayer { .. }
                | Self::ServerPlayerInput { .. }
                | Self::PlayerVisibilityChanged { .. }
                | Self::HitPlayer(..)
                | Self::AffectDamageModel { .. }
                | Self::InitializeVictoryItems { .. }
                | Self::VictoryItemTakeOrPut { .. }
                | Self::TrapPlaced { .. }
                | Self::TrapRemoved { .. }
                | Self::TrapFired { .. }
                | Self::TrapDisarmed { .. }
                | Self::GameWorldObjectState { .. }
                | Self::DamageModelState { .. }
        )
    }

    /// This message for one client, which numbers the players by their place
    /// in its startup roster (`network_client::query_players`): `client_id`
    /// gives that number for a server player id, `None` for a player the client
    /// never got. The client looks players up by these numbers without checking
    /// them, so a message about a player it doesn't have is `None`. An unknown
    /// attacker is sent as none (`u8(-1)`, which the client checks for), and an
    /// unknown killer as the victim itself.
    pub fn for_client(self, client_id: impl Fn(u8) -> Option<u8>) -> Option<Self> {
        Some(match self {
            Self::SpawnPlayer { player_id, player } => Self::SpawnPlayer {
                player_id: client_id(player_id)?,
                player,
            },
            Self::ServerPlayerInput {
                time_in_ms,
                player_id,
                player_input,
                player_state,
                weapon_state,
            } => Self::ServerPlayerInput {
                time_in_ms,
                player_id: client_id(player_id)?,
                player_input,
                player_state,
                weapon_state,
            },
            Self::SyncResponse {
                is_connected_bitmask,
            } => Self::SyncResponse {
                is_connected_bitmask: (0..u32::BITS as u8)
                    .filter(|id| is_connected_bitmask & 1 << id != 0)
                    .filter_map(&client_id)
                    .fold(0, |mask, id| mask | 1 << id),
            },
            Self::PlayerVisibilityChanged {
                player_id,
                is_visible,
            } => Self::PlayerVisibilityChanged {
                player_id: client_id(player_id)?,
                is_visible,
            },
            Self::HitPlayer(hit) => Self::HitPlayer(PlayerHit {
                being_hit: client_id(hit.being_hit)?,
                hit_initiator: client_id(hit.hit_initiator).unwrap_or(u8::MAX),
                ..hit
            }),
            Self::KillPlayer(kill) => {
                let victim_id = client_id(kill.victim_id)?;
                Self::KillPlayer(PlayerKill {
                    victim_id,
                    killer_id: client_id(kill.killer_id).unwrap_or(victim_id),
                    ..kill
                })
            }
            Self::AffectDamageModel {
                player_id,
                body_part,
                affect,
                event,
            } => Self::AffectDamageModel {
                player_id: client_id(player_id)?,
                body_part,
                affect,
                event,
            },
            Self::PlayerKdStatsChanged {
                player_id,
                kills,
                deaths,
            } => Self::PlayerKdStatsChanged {
                player_id: client_id(player_id)?,
                kills,
                deaths,
            },
            // A battery carried by a player the client doesn't have lies where
            // that player is.
            Self::InitializeVictoryItems {
                team_1_points,
                team_2_points,
                items,
                containers,
            } => Self::InitializeVictoryItems {
                team_1_points,
                team_2_points,
                items: items
                    .into_iter()
                    .map(|item| VictoryItemPlacement {
                        carrier: item.carrier.and_then(&client_id),
                        ..item
                    })
                    .collect(),
                containers,
            },
            Self::VictoryItemTakeOrPut {
                player_id,
                item_id,
                action,
            } => Self::VictoryItemTakeOrPut {
                player_id: client_id(player_id)?,
                item_id,
                action,
            },
            Self::TrapPlaced {
                player_id,
                slot,
                index,
                position,
                angles,
            } => Self::TrapPlaced {
                player_id: client_id(player_id)?,
                slot,
                index,
                position,
                angles,
            },
            Self::TrapRemoved {
                player_id,
                slot,
                index,
            } => Self::TrapRemoved {
                player_id: client_id(player_id)?,
                slot,
                index,
            },
            Self::TrapFired {
                player_id,
                slot,
                index,
            } => Self::TrapFired {
                player_id: client_id(player_id)?,
                slot,
                index,
            },
            Self::TrapDisarmed {
                player_id,
                slot,
                index,
            } => Self::TrapDisarmed {
                player_id: client_id(player_id)?,
                slot,
                index,
            },
            Self::GameWorldObjectState {
                player_id,
                slot,
                index,
                state,
                position,
                angles,
            } => Self::GameWorldObjectState {
                player_id: client_id(player_id)?,
                slot,
                index,
                state,
                position,
                angles,
            },
            Self::DamageModelState { player_id, parts } => Self::DamageModelState {
                player_id: client_id(player_id)?,
                parts,
            },
            // The profiles are numbered when the roster is sent.
            message @ (Self::ConnectionSuccessful
            | Self::MatchOptions { .. }
            | Self::PlayerProfile { .. }
            | Self::GameStatusChanged { .. }
            | Self::MatchWaitTimeChanged { .. }
            | Self::MatchTimeChanged { .. }
            | Self::RespawnTimeChanged { .. }
            | Self::MatchFinished
            | Self::WorldSynchronizationRequest) => message,
        })
    }
}

impl Serialize for ServerGameMessageKind {
    fn serialize(self, packet: &mut impl Packet) {
        // Note that tag is written before `order_id`, which is not part of the message
        match self {
            Self::ConnectionSuccessful {} => (),
            Self::MatchOptions {
                map_id,
                map_name,
                match_mode,
                player_count,
                victory_item_count,
                respawn_time,
                match_time,
            } => {
                packet.write(map_id);
                assert!(map_name.len() < 30);
                packet.write_str(&map_name);
                packet.write(match_mode);
                packet.write(player_count);
                packet.write(victory_item_count);
                packet.write(respawn_time);
                packet.write(match_time);
            }

            Self::SpawnPlayer { player_id, player } => {
                packet.write(player_id);
                let player {
                    position,
                    orientation,
                    look_pitch,
                    is_alive,
                    server_current_active_slot,
                    server_target_active_slot,
                    player_stamina,
                    player_inventory,
                } = player;

                packet.write(position);
                packet.write(orientation);
                packet.write(look_pitch);
                packet.write(is_alive);
                packet.write(server_current_active_slot);
                packet.write(server_target_active_slot);

                let player_stamina {
                    value,
                    last_spending_time_in_ms,
                    last_tick_time_in_ms,
                    lower_threshold_was_reached,
                } = player_stamina;
                packet.write(value);
                packet.write(last_spending_time_in_ms);
                packet.write(last_tick_time_in_ms);
                packet.write(lower_threshold_was_reached);

                for item in player_inventory {
                    match item {
                        player_inventory_slot::weapon_slot(weapon_core) => {
                            let weapon_core {
                                inventory_item,
                                random_seed,
                                normal_random_seed,
                                weapon_target,
                                old_actions_mask,
                                ammo_in_magazine,
                                bullets_in_queue,
                                fire_queue_type,
                                ammo_slot,
                                is_there_chamber_a_round_state,
                                weapon_core_state,
                            } = weapon_core;

                            packet.write(inventory_item);
                            packet.write(random_seed);
                            packet.write(normal_random_seed);
                            packet.write(weapon_target);
                            packet.write(old_actions_mask);
                            packet.write(ammo_in_magazine);
                            packet.write(bullets_in_queue);
                            packet.write(fire_queue_type);
                            packet.write(ammo_slot);
                            if let Some(is_there_chamber_a_round_state) =
                                is_there_chamber_a_round_state
                            {
                                packet.write(is_there_chamber_a_round_state);
                            }
                            if let Some(weapon_core_state) = weapon_core_state {
                                let weapon_core_state {
                                    is_shown,
                                    active_hands,
                                    start_transition_time_in_ms_lhs,
                                    start_transition_time_in_ms_rhs,
                                    weapon_sound_target_state,
                                    logic_sprint_target_state,
                                } = weapon_core_state;
                                packet.write(is_shown);
                                packet.write(active_hands);
                                packet.write(start_transition_time_in_ms_lhs);
                                packet.write(start_transition_time_in_ms_rhs);

                                packet.write(weapon_sound_target_state.0);
                                if let Some((interval_id, interval_time)) =
                                    weapon_sound_target_state.1
                                {
                                    packet.write(interval_id);
                                    packet.write(interval_time);
                                }

                                packet.write(logic_sprint_target_state.0);
                                if let Some((interval_id, interval_time)) =
                                    logic_sprint_target_state.1
                                {
                                    packet.write(interval_id);
                                    packet.write(interval_time);
                                }
                            }
                        }
                        player_inventory_slot::item_amount(amount) => {
                            packet.write(amount);
                        }
                    }
                }
            }

            // `network_client::on_match_packet_received` case 0x82 reads one u32
            // time, then `process_player_action` entries (u8 id + the three POD
            // structs) until the packet ends. For a remote player `time_warp` keeps
            // the time only as an ordering stamp: a time older than the last one
            // for that player is dropped, a later one is clamped to the receiver's
            // clock. One entry per message here.
            Self::ServerPlayerInput {
                time_in_ms,
                player_id,
                player_input,
                player_state,
                weapon_state,
            } => {
                packet.write(time_in_ms);
                packet.write(player_id);
                packet.write(player_input);
                packet.write(player_state);
                packet.write(weapon_state);
            }
            Self::SyncResponse {
                is_connected_bitmask,
            } => packet.write(is_connected_bitmask),

            Self::PlayerVisibilityChanged {
                player_id,
                is_visible,
            } => {
                packet.write(player_id);
                packet.write(is_visible);
            }

            Self::PlayerProfile { player_profile } => {
                player_profile.serialize_udp(packet);
            }

            Self::HitPlayer(hit) => {
                // The PDB types these reads as bool, but target
                // `packet_reader::r<bool>` copies the raw byte without
                // normalization. hit_info therefore retains IDs 0..19.
                packet.write(hit.hit_initiator);
                packet.write(hit.being_hit);
                packet.write_str(&hit.body_part);
                packet.write_str(&hit.damage_type);
                packet.write(hit.amount);
                packet.write(hit.armor_piercing);
            }

            Self::KillPlayer(kill) => {
                packet.write(kill.victim_id);
                packet.write(kill.killer_id);
                packet.write(kill.is_headshot);
                packet.write(kill.item_dict_id);
            }

            Self::AffectDamageModel {
                player_id,
                body_part,
                affect,
                event,
            } => {
                packet.write(player_id);
                // Read into a 16-byte buffer; the sender checks the length.
                debug_assert!(body_part.len() < 16);
                packet.write_str(&body_part);
                packet.write(affect);
                packet.write(event);
            }

            Self::GameStatusChanged { status } => packet.write(status),
            Self::MatchWaitTimeChanged { time_left } => packet.write(time_left),
            Self::MatchTimeChanged { time_left_ms } => packet.write(time_left_ms),
            Self::RespawnTimeChanged { time_left } => packet.write(time_left),
            Self::PlayerKdStatsChanged {
                player_id,
                kills,
                deaths,
            } => {
                packet.write(player_id);
                packet.write(kills);
                packet.write(deaths);
            }
            Self::MatchFinished | Self::WorldSynchronizationRequest => (),
            Self::InitializeVictoryItems {
                team_1_points,
                team_2_points,
                items,
                containers,
            } => {
                packet.write(team_1_points);
                packet.write(team_2_points);
                packet.write(u8::try_from(items.len()).unwrap());
                for item in items {
                    packet.write(item.carrier.unwrap_or(NO_CONTAINER));
                    packet.write(item.item_id);
                    packet.write(item.position);
                }
                packet.write(u8::try_from(containers.len()).unwrap());
                for container in containers {
                    packet.write(container.container_id);
                    packet.write(u8::try_from(container.item_ids.len()).unwrap());
                    for item_id in container.item_ids {
                        packet.write(item_id);
                    }
                }
            }
            Self::VictoryItemTakeOrPut {
                player_id,
                item_id,
                action,
            } => {
                packet.write(player_id);
                packet.write(item_id);
                let (is_take, container_id) = match action {
                    VictoryItemAction::TakeFromGround => (true, NO_CONTAINER),
                    VictoryItemAction::TakeFromBase { container_id } => (true, container_id),
                    VictoryItemAction::Drop { .. } => (false, NO_CONTAINER),
                    VictoryItemAction::PutInBase { container_id } => (false, container_id),
                };
                packet.write(is_take);
                packet.write(container_id);
                if let VictoryItemAction::Drop { position } = action {
                    packet.write(position);
                }
            }
            Self::TrapPlaced {
                player_id,
                slot,
                index,
                position,
                angles,
            } => {
                packet.write(player_id);
                packet.write(slot);
                packet.write(index);
                packet.write(position);
                packet.write(angles);
            }
            Self::TrapRemoved {
                player_id,
                slot,
                index,
            }
            | Self::TrapFired {
                player_id,
                slot,
                index,
            }
            | Self::TrapDisarmed {
                player_id,
                slot,
                index,
            } => {
                packet.write(player_id);
                packet.write(slot);
                packet.write(index);
            }
            Self::GameWorldObjectState {
                player_id,
                slot,
                index,
                state,
                position,
                angles,
            } => {
                packet.write(player_id);
                packet.write(slot);
                packet.write(index);
                packet.write(state);
                packet.write(position);
                packet.write(angles);
            }
            // `body_part_parameters::deserialize` (0x597B90) per part: f32
            // health, u32 last hit time, u8 affect count, then per affect
            // (`deserialize_affect`) a u8 `hit_affects_type_enum` and a u32
            // time, which `apply_affect_by_force` takes as when it started.
            Self::DamageModelState { player_id, parts } => {
                packet.write(player_id);
                for part in parts {
                    packet.write(part.health);
                    packet.write(part.last_hit_time_ms);
                    packet.write(u8::try_from(part.affects.len()).unwrap());
                    for (affect, started_ms) in part.affects {
                        packet.write(affect);
                        packet.write(started_ms);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use vostok::network_packet::UdpPacket;

    #[test]
    fn serializes_hit_player_for_the_stock_client() {
        let message = ServerMessage {
            remote_sequence_id: 1.into(),
            local_sequence_id: 2.into(),
            local_ack_bits: 0,
            kind: ServerMessageKind::Messages(vec![ServerGameMessage {
                order_id: 3.into(),
                game_message: ServerGameMessageKind::HitPlayer(PlayerHit {
                    hit_initiator: 0,
                    being_hit: 1,
                    body_part: "head".to_owned(),
                    damage_type: "injury".to_owned(),
                    amount: 50.0,
                    armor_piercing: 0.25,
                }),
            }]),
        };
        let mut packet = UdpPacket::new();
        message.serialize(&mut packet);

        #[rustfmt::skip]
        let expected: &[u8] = &[
            1, 0, 2, 0, 0, 0,
            0x89, 3, 0,
            0,
            1,
            4, b'h', b'e', b'a', b'd',
            6, b'i', b'n', b'j', b'u', b'r', b'y',
            0, 0, 72, 66,
            0, 0, 128, 62,
        ];
        assert_eq!(packet.get_message(), expected);
    }

    #[test]
    fn serializes_server_player_input_with_the_leading_time() {
        let mut packet = UdpPacket::new();
        ServerGameMessage {
            order_id: 3.into(),
            game_message: ServerGameMessageKind::ServerPlayerInput {
                time_in_ms: 0x0102_0304,
                player_id: 5,
                player_input: player_input {
                    angular_velocity: vostok::math::float2 { x: 0.0, y: 0.0 },
                    angualr_acceleration: vostok::math::float2 { x: 0.0, y: 0.0 },
                    action_mask: 0x20,
                },
                player_state: player_state {
                    translation_w: vostok::math::float3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    euler_angle_y: 0.0,
                    look_pitch: 0.0,
                },
                weapon_state: weapon_state {
                    slot_id: 8,
                    ammo_slot_id: 9,
                    state: 0,
                },
            },
        }
        .serialize(&mut packet);

        #[rustfmt::skip]
        let expected: &[u8] = &[
            0x82, 3, 0,
            4, 3, 2, 1,
            5,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x20, 0, 0, 0,
            0, 0, 128, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            8, 9, 0,
        ];
        assert_eq!(packet.get_message(), expected);
    }

    #[test]
    fn serializes_full_u8_player_ids_preserved_by_the_target_client() {
        let mut packet = UdpPacket::new();
        ServerGameMessage {
            order_id: 3.into(),
            game_message: ServerGameMessageKind::HitPlayer(PlayerHit {
                hit_initiator: 18,
                being_hit: 19,
                body_part: "head".to_owned(),
                damage_type: "injury".to_owned(),
                amount: 25.0,
                armor_piercing: 0.0,
            }),
        }
        .serialize(&mut packet);

        assert_eq!(&packet.get_message()[3..5], &[18, 19]);
    }

    #[test]
    fn serializes_kill_player_for_the_stock_client() {
        let message = ServerMessage {
            remote_sequence_id: 1.into(),
            local_sequence_id: 2.into(),
            local_ack_bits: 0,
            kind: ServerMessageKind::Messages(vec![ServerGameMessage {
                order_id: 3.into(),
                game_message: ServerGameMessageKind::KillPlayer(PlayerKill {
                    victim_id: 1,
                    killer_id: 0,
                    is_headshot: true,
                    item_dict_id: 13,
                }),
            }]),
        };
        let mut packet = UdpPacket::new();
        message.serialize(&mut packet);

        #[rustfmt::skip]
        let expected: &[u8] = &[
            1, 0, 2, 0, 0, 0,
            0x83, 3, 0,
            1, 0, 1,
            13, 0, 0, 0,
        ];
        assert_eq!(packet.get_message(), expected);
    }

    #[test]
    fn serializes_trap_placed_for_the_stock_client() {
        let mut packet = UdpPacket::new();
        ServerGameMessage {
            order_id: 3.into(),
            game_message: ServerGameMessageKind::TrapPlaced {
                player_id: 2,
                slot: 0x10,
                index: 7,
                position: float3 {
                    x: 1.0,
                    y: 0.5,
                    z: -2.0,
                },
                angles: float3 {
                    x: 0.0,
                    y: 2.0,
                    z: 0.0,
                },
            },
        }
        .serialize(&mut packet);

        #[rustfmt::skip]
        let expected: &[u8] = &[
            0x96, 3, 0,
            2, 0x10, 7,
            0, 0, 128, 63, 0, 0, 0, 63, 0, 0, 0, 192,
            0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0,
        ];
        assert_eq!(packet.get_message(), expected);
    }

    #[test]
    fn serializes_game_world_object_state_for_the_stock_client() {
        let mut packet = UdpPacket::new();
        ServerGameMessage {
            order_id: 3.into(),
            game_message: ServerGameMessageKind::GameWorldObjectState {
                player_id: 2,
                slot: 0x10,
                index: 7,
                state: 2,
                position: float3 {
                    x: 1.0,
                    y: 0.5,
                    z: -2.0,
                },
                angles: float3 {
                    x: 0.0,
                    y: 2.0,
                    z: 0.0,
                },
            },
        }
        .serialize(&mut packet);

        #[rustfmt::skip]
        let expected: &[u8] = &[
            0x9C, 3, 0,
            2, 0x10, 7, 2,
            0, 0, 128, 63, 0, 0, 0, 63, 0, 0, 0, 192,
            0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0,
        ];
        assert_eq!(packet.get_message(), expected);
    }

    #[test]
    fn serializes_trap_changes_for_the_stock_client() {
        for (tag, game_message) in [
            (
                0x97,
                ServerGameMessageKind::TrapRemoved {
                    player_id: 2,
                    slot: 0x10,
                    index: 7,
                },
            ),
            (
                0x98,
                ServerGameMessageKind::TrapFired {
                    player_id: 2,
                    slot: 0x10,
                    index: 7,
                },
            ),
            (
                0x99,
                ServerGameMessageKind::TrapDisarmed {
                    player_id: 2,
                    slot: 0x10,
                    index: 7,
                },
            ),
        ] {
            let mut packet = UdpPacket::new();
            ServerGameMessage {
                order_id: 3.into(),
                game_message,
            }
            .serialize(&mut packet);
            assert_eq!(packet.get_message(), [tag, 3, 0, 2, 0x10, 7]);
        }
    }

    #[test]
    fn serializes_affect_damage_model_for_the_stock_client() {
        let mut packet = UdpPacket::new();
        ServerGameMessage {
            order_id: 3.into(),
            game_message: ServerGameMessageKind::AffectDamageModel {
                player_id: 4,
                body_part: "left_leg".to_owned(),
                affect: 4,
                event: 1,
            },
        }
        .serialize(&mut packet);

        #[rustfmt::skip]
        let expected: &[u8] = &[
            0x8A, 3, 0,
            4,
            8, b'l', b'e', b'f', b't', b'_', b'l', b'e', b'g',
            4, 0, 0, 0,
            1, 0, 0, 0,
        ];
        assert_eq!(packet.get_message(), expected);
    }

    #[test]
    fn serializes_damage_model_state_for_the_stock_client() {
        let mut packet = UdpPacket::new();
        ServerGameMessage {
            order_id: 3.into(),
            game_message: ServerGameMessageKind::DamageModelState {
                player_id: 2,
                parts: vec![
                    BodyPartState {
                        health: 1.0,
                        last_hit_time_ms: 0,
                        affects: vec![],
                    },
                    BodyPartState {
                        health: 0.5,
                        last_hit_time_ms: 0x0001_0203,
                        affects: vec![(4, 0x0A0B_0C0D), (1, 7)],
                    },
                ],
            },
        }
        .serialize(&mut packet);

        #[rustfmt::skip]
        let expected: &[u8] = &[
            0x9E, 3, 0,
            2,
            0, 0, 128, 63, 0, 0, 0, 0, 0,
            0, 0, 0, 63, 3, 2, 1, 0, 2,
            4, 0x0D, 0x0C, 0x0B, 0x0A,
            1, 7, 0, 0, 0,
        ];
        assert_eq!(packet.get_message(), expected);
        assert_eq!(
            ServerGameMessageKind::DamageModelState {
                player_id: 3,
                parts: vec![],
            }
            .for_client(roster_client),
            Some(ServerGameMessageKind::DamageModelState {
                player_id: 1,
                parts: vec![],
            })
        );
    }

    #[test]
    fn serializes_player_visibility_for_drop_in_and_out() {
        let mut packet = UdpPacket::new();
        ServerGameMessage {
            order_id: 3.into(),
            game_message: ServerGameMessageKind::PlayerVisibilityChanged {
                player_id: 7,
                is_visible: false,
            },
        }
        .serialize(&mut packet);

        assert_eq!(packet.get_message(), &[0x91, 3, 0, 7, 0]);
    }

    fn serialized(game_message: ServerGameMessageKind) -> Vec<u8> {
        let mut packet = UdpPacket::new();
        ServerGameMessage {
            order_id: 3.into(),
            game_message,
        }
        .serialize(&mut packet);
        packet.get_message().to_vec()
    }

    #[test]
    fn serializes_match_lifecycle_messages_for_the_stock_client() {
        #[rustfmt::skip]
        let cases: [(ServerGameMessageKind, &[u8]); 7] = [
            (
                ServerGameMessageKind::GameStatusChanged { status: game_status::inprocess },
                &[0x9A, 3, 0, 4, 0, 0, 0],
            ),
            (
                ServerGameMessageKind::MatchWaitTimeChanged { time_left: 10 },
                &[0x9B, 3, 0, 10, 0, 0, 0],
            ),
            (
                ServerGameMessageKind::MatchTimeChanged { time_left_ms: 900_000 },
                &[0x86, 3, 0, 0xA0, 0xBB, 0x0D, 0],
            ),
            (
                ServerGameMessageKind::RespawnTimeChanged { time_left: 7 },
                &[0x87, 3, 0, 7, 0, 0, 0],
            ),
            (
                ServerGameMessageKind::PlayerKdStatsChanged { player_id: 19, kills: 2, deaths: 1 },
                &[0x88, 3, 0, 19, 2, 0, 0, 0, 1, 0, 0, 0],
            ),
            (ServerGameMessageKind::MatchFinished, &[0x8C, 3, 0]),
            (ServerGameMessageKind::WorldSynchronizationRequest, &[0x9D, 3, 0]),
        ];

        for (message, expected) in cases {
            assert_eq!(serialized(message.clone()), expected, "{message:?}");
        }
    }

    #[test]
    fn serializes_battery_messages_for_the_stock_client() {
        let position = vostok::math::float3 {
            x: 1.0,
            y: 2.0,
            z: -1.0,
        };
        // 1.0, 2.0, -1.0 as little-endian f32s.
        let position_bytes = [0, 0, 0x80, 0x3F, 0, 0, 0, 0x40, 0, 0, 0x80, 0xBF];

        let mut initialize = vec![0x94, 3, 0, 1, 0xFE, 2];
        initialize.extend([0xFF, 4]);
        initialize.extend(position_bytes);
        initialize.extend([7, 5]);
        initialize.extend(position_bytes);
        initialize.extend([2, 0, 0, 1, 2, 9, 8]);
        assert_eq!(
            serialized(ServerGameMessageKind::InitializeVictoryItems {
                team_1_points: 1,
                team_2_points: -2,
                items: vec![
                    VictoryItemPlacement {
                        carrier: None,
                        item_id: 4,
                        position,
                    },
                    VictoryItemPlacement {
                        carrier: Some(7),
                        item_id: 5,
                        position,
                    },
                ],
                containers: vec![
                    VictoryItemsContainerContents {
                        container_id: 0,
                        item_ids: vec![],
                    },
                    VictoryItemsContainerContents {
                        container_id: 1,
                        item_ids: vec![9, 8],
                    },
                ],
            }),
            initialize
        );

        let take_or_put = |action| {
            serialized(ServerGameMessageKind::VictoryItemTakeOrPut {
                player_id: 6,
                item_id: 2,
                action,
            })
        };
        assert_eq!(
            take_or_put(VictoryItemAction::TakeFromGround),
            [0x95, 3, 0, 6, 2, 1, 0xFF]
        );
        assert_eq!(
            take_or_put(VictoryItemAction::TakeFromBase { container_id: 1 }),
            [0x95, 3, 0, 6, 2, 1, 1]
        );
        assert_eq!(
            take_or_put(VictoryItemAction::PutInBase { container_id: 0 }),
            [0x95, 3, 0, 6, 2, 0, 0]
        );
        let mut drop = vec![0x95, 3, 0, 6, 2, 0, 0xFF];
        drop.extend(position_bytes);
        assert_eq!(take_or_put(VictoryItemAction::Drop { position }), drop);
    }

    #[test]
    fn spawn_player_contains_all_selected_ammo_slot_amounts() {
        let mut packet = UdpPacket::new();
        let profile =
            survarium::player_profile::raw::player_profile::new_dummy(1, 0, "demo_player");
        ServerGameMessageKind::SpawnPlayer {
            player_id: 0,
            player: crate::game::spawn_content(0, &profile, [(0, 1); 2], |_| None),
        }
        .serialize(&mut packet);

        // The dummy loadout's ammo slots: 60 and 30, 10 and 10.
        for (amount, times) in [(60_u16, 1), (30, 1), (10, 2)] {
            assert!(
                packet
                    .get_message()
                    .windows(2)
                    .filter(|window| *window == amount.to_le_bytes())
                    .count()
                    >= times,
                "spawn state must contain the ammo slot amount {amount}"
            );
        }
    }

    /// A client whose roster listed server players 0, 3 and 1, in that order.
    fn roster_client(player_id: u8) -> Option<u8> {
        [0, 3, 1]
            .iter()
            .position(|listed| *listed == player_id)
            .map(|client_id| client_id as u8)
    }

    #[test]
    fn numbers_the_players_as_the_client_does_and_drops_the_others() {
        let visibility = |player_id| ServerGameMessageKind::PlayerVisibilityChanged {
            player_id,
            is_visible: true,
        };
        assert_eq!(visibility(3).for_client(roster_client), Some(visibility(1)));
        assert_eq!(visibility(2).for_client(roster_client), None);
        assert_eq!(
            ServerGameMessageKind::SyncResponse {
                is_connected_bitmask: 0b1110,
            }
            .for_client(roster_client),
            Some(ServerGameMessageKind::SyncResponse {
                is_connected_bitmask: 0b110,
            })
        );
        assert_eq!(
            ServerGameMessageKind::MatchFinished.for_client(roster_client),
            Some(ServerGameMessageKind::MatchFinished)
        );
    }

    #[test]
    fn an_unknown_attacker_is_nobody_and_an_unknown_killer_the_victim() {
        let hit = |hit_initiator, being_hit| {
            ServerGameMessageKind::HitPlayer(PlayerHit {
                hit_initiator,
                being_hit,
                body_part: "head".to_owned(),
                damage_type: "injury".to_owned(),
                amount: 10.0,
                armor_piercing: 0.0,
            })
        };
        assert_eq!(hit(2, 3).for_client(roster_client), Some(hit(u8::MAX, 1)));
        assert_eq!(hit(3, 2).for_client(roster_client), None);

        let kill = |victim_id, killer_id| {
            ServerGameMessageKind::KillPlayer(PlayerKill {
                victim_id,
                killer_id,
                is_headshot: false,
                item_dict_id: 13,
            })
        };
        assert_eq!(kill(1, 3).for_client(roster_client), Some(kill(2, 1)));
        assert_eq!(kill(1, 2).for_client(roster_client), Some(kill(2, 2)));
        assert_eq!(kill(2, 1).for_client(roster_client), None);
    }

    #[test]
    fn a_battery_carried_by_an_unknown_player_lies_where_the_player_is() {
        let carried_by = |carrier| ServerGameMessageKind::InitializeVictoryItems {
            team_1_points: 0,
            team_2_points: 0,
            items: vec![VictoryItemPlacement {
                carrier,
                item_id: 0,
                position: float3 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
            }],
            containers: Vec::new(),
        };
        assert_eq!(
            carried_by(Some(3)).for_client(roster_client),
            Some(carried_by(Some(1)))
        );
        assert_eq!(
            carried_by(Some(2)).for_client(roster_client),
            Some(carried_by(None))
        );
    }
}
