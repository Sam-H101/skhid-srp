pub mod client_message;
pub mod server_message;

pub use client_message::{
    ClientGameMessage, ClientGameMessageKind, ClientMessage, ClientMessageKind,
};
pub use server_message::{
    ServerGameMessage, ServerGameMessageKind, ServerMessage, ServerMessageKind,
};

use vostok::math::float3;

pub const CLIENT_PLAYER_HIT_VERSION: u8 = 1;
pub const MAX_HIT_STRING_LENGTH: usize = 15;

#[derive(Debug, PartialEq, Clone)]
pub struct PlayerHit {
    pub hit_initiator: u8,
    pub being_hit: u8,
    pub body_part: String,
    pub damage_type: String,
    pub amount: f32,
    pub armor_piercing: f32,
}

#[derive(Debug, PartialEq, Clone)]
pub struct PlayerKill {
    pub victim_id: u8,
    pub killer_id: u8,
    pub is_headshot: bool,
    pub item_dict_id: u32,
}

/// One battery in `initialize_victory_items`: on the ground at `position`,
/// or carried by `carrier`. Batteries stored in a base are listed with the
/// base instead (`VictoryItemsContainerContents`).
#[derive(Debug, PartialEq, Clone)]
pub struct VictoryItemPlacement {
    pub carrier: Option<u8>,
    pub item_id: u8,
    pub position: float3,
}

/// The batteries stored in one base, oldest first: the client stacks them
/// and a theft takes the newest.
#[derive(Debug, PartialEq, Clone)]
pub struct VictoryItemsContainerContents {
    pub container_id: u8,
    pub item_ids: Vec<u8>,
}

/// One body part in `damage_model_state`, its times on the receiving
/// client's clock: health (absolute), the last hit (0: never; regeneration
/// waits `regeneration_timeout` after it) and each affect
/// (`hit_affects_type_enum`) with when it was applied (it lasts its
/// `affects_durations` from then).
#[derive(Debug, PartialEq, Clone)]
pub struct BodyPartState {
    pub health: f32,
    pub last_hit_time_ms: u32,
    pub affects: Vec<(u8, u32)>,
}

/// What `victory_item_take_or_put` tells the client a player did.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum VictoryItemAction {
    /// Picked a battery up from the ground.
    TakeFromGround,
    /// Took (stole) the newest battery from a base.
    TakeFromBase { container_id: u8 },
    /// Dropped it (death, disconnect). The client puts it at its own copy of
    /// the carrier; the position is read but not used.
    Drop { position: float3 },
    /// Stored it in a base.
    PutInBase { container_id: u8 },
}

pub mod raw {
    #![expect(non_camel_case_types)]
    #![expect(dead_code)]

    #[repr(u8)]
    #[derive(bytemuck::CheckedBitPattern, bytemuck::NoUninit, Copy, Clone, Debug, PartialEq)]
    #[rustfmt::skip]
    pub enum low_level_message_type_enum {
        initiate_disconnection = 0x0,
        confirm_disconnection  = 0x1,
        continuous_flow        = 0x2,
    }

    #[repr(u8)]
    #[derive(bytemuck::CheckedBitPattern, bytemuck::NoUninit, Copy, Clone, Debug, PartialEq)]
    #[rustfmt::skip]
    pub enum udp_match_packets_count_enum {
        single_packet    = 0x0,
        multiple_packets = 0x1,
    }
}
