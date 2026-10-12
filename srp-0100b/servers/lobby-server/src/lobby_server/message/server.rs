use crate::lobby_server::message::client::raw::{
    faction_id, lobby_client_message_types_enum, query_info_types_enum,
};
use survarium::player_profile;
use vostok::config;
use vostok::network_client::NetworkResponse;
use vostok::network_packet::Packet;
use vostok::serde::Serialize;

use self::raw::*;

#[derive(Debug)]
pub enum Message {
    ConnectionSuccessful,
    InvalidSessionId,
    InvalidPassword,
    ConnectToMatchServer {
        host: String,
        match_id: u32,
        team_id: u8,
    },
    OperationPermitted(Operation),
    OperationDenied(Operation),
    ClientStatus(ClientStatus),
    PingServerAnswer {
        ping_value: u32,
    },
}

#[derive(Debug, Clone)]
pub enum Operation {
    Inventory,
    Shop(ShopOperation),
    SkillsTree,
}

#[derive(Debug, Clone, Copy)]
pub enum ShopOperation {
    ActionBought {
        dict_id: u16,
        id: u32,
        condition_or_stack: u32,
    },
}

#[derive(Debug, Clone)]
pub enum ClientStatus {
    ClientState {
        // @TODO: This should be an enum of some kind.
        // Figure out its values and what they correspond to.
        status: u8,
        /// The client shows it in the lobby's status line; without it the
        /// client clears the line (`read_status_info`).
        last_status_message: Option<String>,
    },
    EnumerateProfiles(Vec<Profile>),
    ProfileContents(Box<player_profile::raw::player_profile>),
    EnumerateInventory(Vec<player_profile::raw::inventory_item_instance>),
    ProfileSlotsRestrictions(Vec<profile_slot_restriction>),
    ItemsCompatibility(Vec<items_compatibility>),
    PriceItems {
        faction_id: faction_id,
        price_items: Vec<price_item>,
    },
    AccountMoney {
        generic_money: u32,
        premium_money: u32,
        skill_points: u8,
        name: String,
    },
    PlayerSkills {
        total_experience: u32,
        next_level_experience: u32,
        prev_level_experience: u32,
        player_skills: [player_skill; 5],
        // @TODO: This is currently skipped
        // player_perks
    },
    PlayerSkillsTree {
        // @TODO: Make it into a type
        skills_tree: Vec<u8>,
    },
    ServicePrices {
        reroll_cost: u32,
        add_profile_cost: u32,
        rename_account_cost: u32,
    },
    PlayerReputations([player_reputation; 4]),
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub profile_id: u32,
    // @TODO: Use sized string to 128 bytes
    pub name: String,
}

//
// Raw types sent to the wire
//

pub mod raw {
    #![expect(non_camel_case_types)]

    #[rustfmt::skip]
    #[repr(u8)]
    #[derive(bytemuck::CheckedBitPattern, bytemuck::NoUninit, Copy, Clone, Debug, PartialEq)]
    pub enum lobby_server_message_types_enum {
        connection_successful             = 0x30,
        invalid_session_id                = 0x31,
        invalid_password                  = 0x32,
        connect_to_match_server           = 0x33,
        operation_permitted               = 0x34,
        operation_denied                  = 0x35,
        client_status                     = 0x36,
        ping_server_answer                = 0x37,
        #[expect(dead_code)]
        lobby_server_invalid_message_type = 0x3F,
    }

    #[repr(C)]
    #[derive(bytemuck::CheckedBitPattern, bytemuck::NoUninit, Copy, Clone, Debug, PartialEq)]
    pub struct profile_slot_restriction {
        pub slot_dict_id: u8,
        pub category_dict_id: u8,
    }

    #[repr(C)]
    #[derive(bytemuck::CheckedBitPattern, bytemuck::NoUninit, Copy, Clone, Debug, PartialEq)]
    pub struct items_compatibility {
        pub first_item_dict_id: u16,
        pub second_item_dict_id: u16,
    }

    #[repr(C)]
    #[derive(bytemuck::CheckedBitPattern, bytemuck::NoUninit, Copy, Clone, Debug, PartialEq)]
    pub struct price_item {
        pub item_dict_id: u16,
        pub cost: u16,
        pub reputation_level: u8,
        pub padding: [u8; 1],
    }

    pub use survarium::account::{player_reputation, player_skill};
}

impl Message {
    /// A refusal the client only takes while signing in: after
    /// `connection_successful` it crashes on one (see `serialize`).
    pub fn is_sign_in_refusal(&self) -> bool {
        matches!(self, Self::InvalidSessionId | Self::InvalidPassword)
    }

    #[rustfmt::skip]
    fn tag(&self) -> lobby_server_message_types_enum {
        match self {
            Self::ConnectionSuccessful { .. } => lobby_server_message_types_enum::connection_successful  ,
            Self::InvalidSessionId { .. }     => lobby_server_message_types_enum::invalid_session_id     ,
            Self::InvalidPassword { .. }      => lobby_server_message_types_enum::invalid_password       ,
            Self::ConnectToMatchServer { .. } => lobby_server_message_types_enum::connect_to_match_server,
            Self::OperationPermitted { .. }   => lobby_server_message_types_enum::operation_permitted    ,
            Self::OperationDenied { .. }      => lobby_server_message_types_enum::operation_denied       ,
            Self::ClientStatus { .. }         => lobby_server_message_types_enum::client_status          ,
            Self::PingServerAnswer { .. }     => lobby_server_message_types_enum::ping_server_answer     ,
        }
    }
}

impl Operation {
    #[rustfmt::skip]
    fn tag(&self) -> lobby_client_message_types_enum {
        match self {
            Self::Inventory { .. }  => lobby_client_message_types_enum::inventory_action  ,
            Self::Shop { .. }       => lobby_client_message_types_enum::shop_action       ,
            Self::SkillsTree { .. } => lobby_client_message_types_enum::skills_tree_action,
        }
    }
}

impl ClientStatus {
    #[rustfmt::skip]
    fn tag(&self) -> query_info_types_enum {
        match self {
            Self::ClientState { .. }              => query_info_types_enum::q_client_state              ,
            Self::EnumerateProfiles { .. }        => query_info_types_enum::q_enumerate_profiles        ,
            Self::ProfileContents { .. }          => query_info_types_enum::q_profile_contents          ,
            Self::EnumerateInventory { .. }       => query_info_types_enum::q_enumerate_inventory       ,
            Self::ProfileSlotsRestrictions { .. } => query_info_types_enum::q_profile_slots_restrictions,
            Self::ItemsCompatibility { .. }       => query_info_types_enum::q_items_compatibility       ,
            Self::PriceItems { .. }               => query_info_types_enum::q_price_items               ,
            Self::AccountMoney { .. }             => query_info_types_enum::q_account_money             ,
            Self::PlayerSkills { .. }             => query_info_types_enum::q_player_skills             ,
            Self::PlayerSkillsTree { .. }         => query_info_types_enum::q_player_skills_tree        ,
            Self::ServicePrices { .. }            => query_info_types_enum::q_service_prices            ,
            Self::PlayerReputations { .. }        => query_info_types_enum::q_player_reputations        ,
        }
    }
}

//
//
//

//
//
//

/// The client copies these strings into fixed buffers without checking their
/// length (`on_lobby_packet_received`, `lobby_client`): the match server host
/// into 64 bytes, the status message into 128, profile and account names into
/// 32, each with its terminating zero. It has room for three profiles.
const HOST_MAX_LEN: usize = 63;
const STATUS_MESSAGE_MAX_LEN: usize = 127;
const NAME_MAX_LEN: usize = 31;
const MAX_PROFILES: usize = 3;

impl NetworkResponse for Message {}
impl Serialize for Message {
    fn serialize(self, packet: &mut impl Packet) {
        packet.write(self.tag());
        match self {
            // @TODO: Why is this skipped?
            Self::ConnectionSuccessful => (),

            // The client reads only the type of both. Before
            // `connection_successful` it logs one and keeps waiting
            // (`lobby_client::sign_in_on_packet_received`, 0x5C8120); after
            // it, its dispatch (`network_client::on_lobby_packet_received`,
            // 0x5C6B78) jumps through a table of 0x33..=0x37 unchecked, so
            // either one crashes it. Send them only while signing in.
            Self::InvalidSessionId | Self::InvalidPassword => (),

            Self::ConnectToMatchServer {
                host,
                match_id,
                team_id,
            } => {
                packet.write_str_capped(&host, HOST_MAX_LEN);
                packet.write(config::get().match_server.port);
                packet.write(match_id);
                packet.write(team_id);
            }

            Self::OperationPermitted(action) => {
                action.serialize(packet);
            }

            // `on_lobby_packet_received` reads the operation, a faction (unused)
            // and a text it shows in a message box.
            Self::OperationDenied(action) => {
                packet.write(action.tag());
                packet.write(0_u8);
                packet.write_str("");
            }

            Self::ClientStatus(client_status) => {
                client_status.serialize(packet);
            }

            Self::PingServerAnswer { ping_value } => {
                packet.write(ping_value);
            }
        }
    }
}

impl Serialize for Operation {
    fn serialize(self, packet: &mut impl Packet) {
        packet.write(self.tag());

        match self {
            Self::Shop(ShopOperation::ActionBought {
                dict_id,
                id,
                condition_or_stack,
            }) => {
                // `process_shop_action` ignores the purchase unless this is 0.
                packet.write(0_u8);
                packet.write(dict_id);
                packet.write(id);
                packet.write(condition_or_stack);
            }
            Self::Inventory | Self::SkillsTree => (),
        }
    }
}

impl Serialize for ClientStatus {
    fn serialize(self, packet: &mut impl Packet) {
        packet.write(self.tag());
        match self {
            Self::ClientState {
                status,
                last_status_message,
            } => {
                packet.write(status);
                if let Some(text) = last_status_message {
                    packet.write_str_capped(&text, STATUS_MESSAGE_MAX_LEN);
                }
            }

            Self::EnumerateProfiles(profiles) => {
                let profiles = &profiles[..profiles.len().min(MAX_PROFILES)];
                packet.write(profiles.len() as u8);

                for Profile { profile_id, name } in profiles {
                    packet.write(*profile_id);
                    packet.write_str_capped(name, NAME_MAX_LEN);
                }
            }

            Self::ProfileContents(player_profile) => player_profile.serialize_tcp(packet),

            Self::EnumerateInventory(items) => {
                packet.write_vec::<u32, _, _>(items);
            }

            Self::ProfileSlotsRestrictions(restricts) => {
                packet.write_vec::<u32, _, _>(restricts);
            }

            Self::ItemsCompatibility(compats) => {
                packet.write_vec::<u32, _, _>(compats);
            }

            Self::PriceItems {
                faction_id,
                price_items,
            } => {
                packet.write(faction_id);
                // `lobby_client::read_price_items` reads a u16 count.
                packet.write_vec::<u16, _, _>(price_items);
            }

            Self::AccountMoney {
                generic_money,
                premium_money,
                skill_points,
                name,
            } => {
                packet.write(generic_money);
                packet.write(premium_money);
                packet.write(skill_points);
                packet.write_str_capped(&name, NAME_MAX_LEN);
            }

            Self::PlayerSkills {
                total_experience,
                next_level_experience,
                prev_level_experience,
                player_skills,
            } => {
                packet.write(total_experience);
                packet.write(next_level_experience);
                packet.write(prev_level_experience);
                packet.write_slice::<u8, _, _>(&player_skills);
                packet.write(0_u8); // player_perks_len
            }

            Self::PlayerSkillsTree { skills_tree } => packet.append_bytes(skills_tree.as_slice()),

            Self::ServicePrices {
                reroll_cost,
                add_profile_cost,
                rename_account_cost,
            } => {
                packet.write(reroll_cost);
                packet.write(add_profile_cost);
                packet.write(rename_account_cost);
            }

            Self::PlayerReputations(reps) => {
                packet.write_slice::<u8, _, _>(&reps);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use survarium::account::faction_id;
    use vostok::network_packet::TcpPacket;

    /// A short message without its one-byte length frame.
    fn body(message: Message) -> Vec<u8> {
        let mut packet = TcpPacket::new();
        message.serialize(&mut packet);
        let framed = packet.get_buffer();
        assert_eq!(usize::from(framed[0]), framed.len() - 1);
        framed[1..].to_vec()
    }

    #[test]
    fn price_items_have_a_u16_count() {
        let price = price_item {
            item_dict_id: 13,
            cost: 500,
            reputation_level: 2,
            padding: [0],
        };
        assert_eq!(
            body(Message::ClientStatus(ClientStatus::PriceItems {
                faction_id: faction_id::scavengers,
                price_items: vec![price],
            })),
            [0x36, 6, 1, 1, 0, 13, 0, 0xF4, 1, 2, 0]
        );
    }

    #[test]
    fn a_purchase_is_permitted_with_a_zero_the_client_accepts() {
        assert_eq!(
            body(Message::OperationPermitted(Operation::Shop(
                ShopOperation::ActionBought {
                    dict_id: 13,
                    id: 7,
                    condition_or_stack: 100,
                }
            ))),
            [0x34, 0x24, 0, 13, 0, 7, 0, 0, 0, 100, 0, 0, 0]
        );
    }

    #[test]
    fn names_fit_the_clients_buffers() {
        let long = "ü".repeat(40);
        let profile = |profile_id| Profile {
            profile_id,
            name: long.clone(),
        };
        let profiles = body(Message::ClientStatus(ClientStatus::EnumerateProfiles(
            (1..=4).map(profile).collect(),
        )));
        // Three profiles, each a u32 id and 30 bytes of name (15 two-byte
        // characters fit in 31).
        assert_eq!(profiles[..3], [0x36, 1, 3]);
        assert_eq!(profiles.len(), 3 + 3 * (4 + 1 + 30));
        assert_eq!(profiles[3 + 4], 30);

        let money = body(Message::ClientStatus(ClientStatus::AccountMoney {
            generic_money: 0,
            premium_money: 0,
            skill_points: 0,
            name: "a".repeat(40),
        }));
        assert_eq!(money[2 + 9], 31);
        assert_eq!(money.len(), 2 + 9 + 1 + 31);
    }

    #[test]
    fn a_status_without_text_ends_after_the_status() {
        assert_eq!(
            body(Message::ClientStatus(ClientStatus::ClientState {
                status: 0,
                last_status_message: None,
            })),
            [0x36, 0, 0]
        );
    }

    #[test]
    fn the_match_server_is_where_the_connection_says() {
        let port = config::get().match_server.port.to_le_bytes();
        assert_eq!(
            body(Message::ConnectToMatchServer {
                host: "192.168.50.134".to_string(),
                match_id: 0x123,
                team_id: 1,
            }),
            [
                &[0x33, 14][..],
                b"192.168.50.134",
                &port,
                &[0x23, 0x01, 0, 0, 1],
            ]
            .concat()
        );
    }

    #[test]
    fn sign_in_refusals_are_their_type_alone() {
        assert_eq!(body(Message::InvalidSessionId), [0x31]);
        assert_eq!(body(Message::InvalidPassword), [0x32]);
    }

    #[test]
    fn a_denial_carries_a_faction_and_an_empty_text() {
        assert_eq!(
            body(Message::OperationDenied(Operation::Inventory)),
            [0x35, 0x23, 0, 0]
        );
    }
}
