//! The 0.21d messaging (chat) protocol the stock client speaks on the lobby
//! port (`docs/notes/lobby_spec.md` §1.5 and §5).
//!
//! Frames are the lobby's: a `u8` length, or `0` and a `u16` length for 256
//! bytes and more. Strings are a `u8` length and the bytes, in the client's
//! multibyte code page, so they stay raw bytes here. The client copies a
//! string into a fixed buffer without checking its length (spec R3), so
//! every string written is cut to its buffer.

use vostok::network_client::{NetworkRequest, NetworkResponse};
use vostok::network_packet::Packet;
use vostok::serde::{Deserialize, DeserializeError, Serialize, advance_buffer, advance_by};

use super::{
    friendship_actions_enum, messaging_client_message_types_enum,
    messaging_server_message_types_enum,
};

/// The `result` byte of an accepted friendship change: the client then asks
/// for the list again; anything else it only logs.
const FRIENDSHIP_ACCEPTED: u8 = b'4';

/// A name's buffer in the client (`char[64]`).
pub const NAME_CAPACITY: usize = 64;
/// A chat line's or an important message's buffer (`char[256]`).
const TEXT_CAPACITY: usize = 256;

#[derive(Debug, PartialEq, Eq)]
pub enum ClientMessage {
    /// The first message on the connection.
    SignInInfo {
        session_id: u32,
        client_type: u8,
    },
    /// The channel ids the client listens to, by
    /// `messaging_message_channel_enum`: [0, language, -1, 0, 1, match,
    /// team 1, team 2, squad].
    Subscription {
        channel_ids: [u32; 9],
    },
    Friendship(FriendshipRequest),
    /// A line the player typed. The client has already shown it locally.
    Text {
        /// The language for the general channel, the match for the match
        /// and team channels, the squad for the squad channel, else 0.
        channel_id: u32,
        /// Only set for the private channel.
        receiver: Vec<u8>,
        channel: u8,
        body: Vec<u8>,
    },
    /// A message for another player that waits for an answer
    /// (`important_message_types`): a friend request or a squad invite.
    Important {
        target_name: Vec<u8>,
        message_type: u8,
        text: Vec<u8>,
    },
    /// The player's answer to an important message: 0 accept, 1 decline.
    ImportantAnswer {
        message_id: u32,
        answer: u8,
    },
    /// Anything else, skipped whole.
    Unknown {
        message_type: u8,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum FriendshipRequest {
    RemoveFriend { account_id: u32 },
    AddIgnored { account_id: u32 },
    AddIgnoredByName { name: Vec<u8> },
    RemoveIgnored { account_id: u32 },
    FindPlayers { name: Vec<u8> },
    QueryFriendList,
    QueryIgnoreList,
    UpdateFriendsStatus,
}

impl FriendshipRequest {
    pub fn action(&self) -> friendship_actions_enum {
        use friendship_actions_enum as actions;

        match self {
            Self::RemoveFriend { .. } => actions::remove_friend,
            Self::AddIgnored { .. } => actions::add_ignorable,
            Self::AddIgnoredByName { .. } => actions::add_ignorable_by_name,
            Self::RemoveIgnored { .. } => actions::remove_ignorable,
            Self::FindPlayers { .. } => actions::find_players,
            Self::QueryFriendList => actions::query_friend_list,
            Self::QueryIgnoreList => actions::query_ignore_list,
            Self::UpdateFriendsStatus => actions::update_friends_status,
        }
    }
}

/// A friend's status in the friend list (`lobby_state`, `ingame`, `offline`,
/// `away`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FriendStatus {
    InLobby = 0,
    Offline = 2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerMessage {
    /// The answer to `SignInInfo`: the player's name in the chat.
    SignInInfo { local_name: Vec<u8> },
    /// A line for the client's chat, from another player.
    Text {
        sender_type: u8,
        sender_account_id: u32,
        sender_name: Vec<u8>,
        channel: u8,
        body: Vec<u8>,
    },
    /// The friend list: id, name and status, each. It replaces the client's.
    FriendList(Vec<ListedAccount>),
    /// The ignore list: id and name, each.
    IgnoreList(Vec<ListedAccount>),
    /// Players found by name: id and name, each.
    FoundPlayers(Vec<ListedAccount>),
    /// The friends' statuses: id and status, each. The client reads them only
    /// when they are as many as its friends, else it asks for the list again.
    FriendStatuses(Vec<ListedAccount>),
    /// The answer to a friendship change.
    FriendshipResult {
        action: friendship_actions_enum,
        accepted: bool,
    },
    /// A message the client keeps until the player answers it (`0xC6`).
    Important {
        sender_type: u8,
        sender_account_id: u32,
        sender_name: Vec<u8>,
        message_type: u8,
        text: Vec<u8>,
        message_id: u32,
    },
}

/// An account in a list the client reads: a `u32` id, then whichever of
/// the name and the status byte the list carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedAccount {
    pub account_id: u32,
    pub name: Vec<u8>,
    pub status: FriendStatus,
}

/// A list's count is a `u16`; the client resizes its list to it.
fn write_accounts(
    packet: &mut impl Packet,
    action: friendship_actions_enum,
    accounts: &[ListedAccount],
    name: bool,
    status: bool,
) {
    packet.write(messaging_server_message_types_enum::messaging_friendship_status as u8);
    packet.write(action as u8);
    packet.write(accounts.len() as u16);
    for account in accounts {
        packet.write(account.account_id);
        if name {
            write_string(packet, &account.name, NAME_CAPACITY);
        }
        if status {
            packet.write(account.status as u8);
        }
    }
}

impl NetworkRequest for ClientMessage {}
impl NetworkResponse for ServerMessage {}

impl Deserialize for ClientMessage {
    fn deserialize(out_buffer: &mut &[u8]) -> Result<Self, DeserializeError> {
        // Don't advance `out_buffer` until the whole frame is there.
        let mut buffer = *out_buffer;
        let frame_len = match advance_buffer::<u8>(&mut buffer)? {
            0 => advance_buffer::<u16>(&mut buffer)? as usize,
            len => len as usize,
        };
        let frame = buffer
            .get(..frame_len)
            .ok_or(DeserializeError::NotEnoughInput)?;
        let message = parse_frame(frame)?;
        *out_buffer = &buffer[frame_len..];
        Ok(message)
    }
}

fn parse_frame(mut frame: &[u8]) -> Result<ClientMessage, DeserializeError> {
    use messaging_client_message_types_enum as types;

    let frame = &mut frame;
    // A frame that ends early is malformed, not a message still on its way.
    let short = |error| match error {
        DeserializeError::NotEnoughInput => DeserializeError::incorrect_input(),
        error => error,
    };
    // An empty frame too: taken for "more to come", it had the reader wait
    // and grow its buffer for ever.
    let message_type = advance_buffer::<u8>(frame).map_err(short)?;
    let message = match message_type {
        _ if message_type == types::messaging_client_sign_in_info as u8 => {
            ClientMessage::SignInInfo {
                session_id: advance_buffer(frame).map_err(short)?,
                client_type: advance_buffer(frame).map_err(short)?,
            }
        }
        _ if message_type == types::messaging_client_subscription as u8 => {
            ClientMessage::Subscription {
                channel_ids: advance_buffer(frame).map_err(short)?,
            }
        }
        _ if message_type == types::messaging_friendship_action as u8 => {
            ClientMessage::Friendship(parse_friendship(frame).map_err(short)?)
        }
        _ if message_type == types::messaging_client_message as u8 => ClientMessage::Text {
            channel_id: advance_buffer(frame).map_err(short)?,
            receiver: advance_string(frame).map_err(short)?,
            channel: advance_buffer(frame).map_err(short)?,
            body: advance_string(frame).map_err(short)?,
        },
        _ if message_type == types::messaging_important_message as u8 => {
            // Always 0.
            advance_buffer::<u32>(frame).map_err(short)?;
            ClientMessage::Important {
                target_name: advance_string(frame).map_err(short)?,
                message_type: advance_buffer(frame).map_err(short)?,
                text: advance_string(frame).map_err(short)?,
            }
        }
        _ if message_type == types::messaging_important_message_read as u8 => {
            ClientMessage::ImportantAnswer {
                message_id: advance_buffer(frame).map_err(short)?,
                answer: advance_buffer(frame).map_err(short)?,
            }
        }
        _ => ClientMessage::Unknown { message_type },
    };
    Ok(message)
}

fn parse_friendship(frame: &mut &[u8]) -> Result<FriendshipRequest, DeserializeError> {
    use friendship_actions_enum as actions;

    let action = advance_buffer::<u8>(frame)?;
    let request = match action {
        _ if action == actions::remove_friend as u8 => FriendshipRequest::RemoveFriend {
            account_id: advance_buffer(frame)?,
        },
        _ if action == actions::add_ignorable as u8 => FriendshipRequest::AddIgnored {
            account_id: advance_buffer(frame)?,
        },
        _ if action == actions::add_ignorable_by_name as u8 => {
            FriendshipRequest::AddIgnoredByName {
                name: advance_string(frame)?,
            }
        }
        _ if action == actions::remove_ignorable as u8 => FriendshipRequest::RemoveIgnored {
            account_id: advance_buffer(frame)?,
        },
        _ if action == actions::find_players as u8 => FriendshipRequest::FindPlayers {
            name: advance_string(frame)?,
        },
        _ if action == actions::query_friend_list as u8 => FriendshipRequest::QueryFriendList,
        _ if action == actions::query_ignore_list as u8 => FriendshipRequest::QueryIgnoreList,
        _ if action == actions::update_friends_status as u8 => {
            FriendshipRequest::UpdateFriendsStatus
        }
        // 0 and 1 (adding a friend) go through an important message instead.
        _ => {
            return Err(DeserializeError::incorrect_input_from(format!(
                "unknown friendship action {action}"
            )));
        }
    };
    Ok(request)
}

fn advance_string(buffer: &mut &[u8]) -> Result<Vec<u8>, DeserializeError> {
    let len = advance_buffer::<u8>(buffer)?;
    Ok(advance_by::<u8>(len as usize, buffer)?.to_vec())
}

/// Cuts `value` to what fits in a `capacity`-byte buffer with its NUL.
fn write_string(packet: &mut impl Packet, value: &[u8], capacity: usize) {
    let value = &value[..value.len().min(capacity - 1)];
    packet.write(value.len() as u8);
    packet.append_bytes(value);
}

impl Serialize for ServerMessage {
    fn serialize(self, packet: &mut impl Packet) {
        use friendship_actions_enum as actions;
        use messaging_server_message_types_enum as types;

        match self {
            Self::SignInInfo { local_name } => {
                packet.write(types::messaging_server_sign_in_info as u8);
                write_string(packet, &local_name, NAME_CAPACITY);
            }
            Self::Text {
                sender_type,
                sender_account_id,
                sender_name,
                channel,
                body,
            } => {
                packet.write(types::messaging_server_message as u8);
                packet.write(sender_type);
                packet.write(sender_account_id);
                write_string(packet, &sender_name, NAME_CAPACITY);
                packet.write(channel);
                write_string(packet, &body, TEXT_CAPACITY);
            }
            Self::FriendList(accounts) => {
                write_accounts(packet, actions::query_friend_list, &accounts, true, true)
            }
            Self::IgnoreList(accounts) => {
                write_accounts(packet, actions::query_ignore_list, &accounts, true, false)
            }
            Self::FoundPlayers(accounts) => {
                write_accounts(packet, actions::find_players, &accounts, true, false)
            }
            Self::FriendStatuses(accounts) => write_accounts(
                packet,
                actions::update_friends_status,
                &accounts,
                false,
                true,
            ),
            Self::FriendshipResult { action, accepted } => {
                packet.write(types::messaging_friendship_status as u8);
                packet.write(action as u8);
                packet.write(if accepted { FRIENDSHIP_ACCEPTED } else { 0 });
            }
            Self::Important {
                sender_type,
                sender_account_id,
                sender_name,
                message_type,
                text,
                message_id,
            } => {
                packet.write(types::messaging_important_message as u8);
                packet.write(sender_type);
                packet.write(sender_account_id);
                write_string(packet, &sender_name, NAME_CAPACITY);
                packet.write(message_type);
                write_string(packet, &text, TEXT_CAPACITY);
                packet.write(message_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vostok::network_packet::TcpPacket;

    fn framed(message: ServerMessage) -> Vec<u8> {
        let mut packet = TcpPacket::new();
        message.serialize(&mut packet);
        packet.get_buffer().to_vec()
    }

    fn parse(mut bytes: &[u8]) -> Result<ClientMessage, DeserializeError> {
        ClientMessage::deserialize(&mut bytes)
    }

    #[test]
    fn reads_what_the_stock_client_sends_after_signing_in() {
        // The sign-in, then after 0xCA: the subscription and the queries
        // for the friend and ignore lists (spec §1.5).
        let mut bytes = vec![6, 0xC3, 0x00, 0xDD, 0, 0, 5];
        bytes.push(37);
        bytes.push(0xC5);
        for channel_id in [0, 2, u32::MAX, 0, 1, 0, 0, 0, 0] {
            bytes.extend(channel_id.to_le_bytes());
        }
        bytes.extend([2, 0xC4, 7, 2, 0xC4, 8]);
        let buffer = &mut bytes.as_slice();

        assert_eq!(
            ClientMessage::deserialize(buffer),
            Ok(ClientMessage::SignInInfo {
                session_id: 0xDD00,
                client_type: 5
            })
        );
        assert_eq!(
            ClientMessage::deserialize(buffer),
            Ok(ClientMessage::Subscription {
                channel_ids: [0, 2, u32::MAX, 0, 1, 0, 0, 0, 0]
            })
        );
        assert_eq!(
            ClientMessage::deserialize(buffer),
            Ok(ClientMessage::Friendship(
                FriendshipRequest::QueryFriendList
            ))
        );
        assert_eq!(
            ClientMessage::deserialize(buffer),
            Ok(ClientMessage::Friendship(
                FriendshipRequest::QueryIgnoreList
            ))
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn reads_a_typed_line_in_a_long_frame() {
        let body = vec![b'x'; 255];
        let mut frame = vec![0xC1];
        frame.extend(2_u32.to_le_bytes());
        frame.extend([0, 1, 255]);
        frame.extend(&body);
        let mut bytes = vec![0];
        bytes.extend((frame.len() as u16).to_le_bytes());
        bytes.extend(&frame);

        assert_eq!(
            parse(&bytes),
            Ok(ClientMessage::Text {
                channel_id: 2,
                receiver: vec![],
                channel: 1,
                body
            })
        );
        assert_eq!(
            parse(&bytes[..bytes.len() - 1]),
            Err(DeserializeError::NotEnoughInput),
            "the rest of the frame is still on its way"
        );
    }

    #[test]
    fn reads_a_friend_request_and_its_answer() {
        let mut frame = vec![0xC2, 0, 0, 0, 0, 3];
        frame.extend(b"bob");
        frame.extend([0, 2]);
        frame.extend(b"hi");
        let mut bytes = vec![frame.len() as u8];
        bytes.extend(&frame);
        bytes.extend([6, 0xC6, 7, 0, 0, 0, 1]);
        let buffer = &mut bytes.as_slice();

        assert_eq!(
            ClientMessage::deserialize(buffer),
            Ok(ClientMessage::Important {
                target_name: b"bob".to_vec(),
                message_type: 0,
                text: b"hi".to_vec(),
            })
        );
        assert_eq!(
            ClientMessage::deserialize(buffer),
            Ok(ClientMessage::ImportantAnswer {
                message_id: 7,
                answer: 1
            })
        );
    }

    #[test]
    fn skips_unknown_messages_whole() {
        let buffer = &mut [3, 0xC7, 1, 2, 2, 0xC4, 9].as_slice();

        assert_eq!(
            ClientMessage::deserialize(buffer),
            Ok(ClientMessage::Unknown { message_type: 0xC7 })
        );
        assert_eq!(
            ClientMessage::deserialize(buffer),
            Ok(ClientMessage::Friendship(
                FriendshipRequest::UpdateFriendsStatus
            ))
        );
    }

    #[test]
    fn writes_what_the_stock_client_reads() {
        assert_eq!(
            framed(ServerMessage::SignInInfo {
                local_name: b"tester".to_vec()
            }),
            [8, 0xCA, 6, b't', b'e', b's', b't', b'e', b'r']
        );
        assert_eq!(
            framed(ServerMessage::Text {
                sender_type: 5,
                sender_account_id: 0x0102_0304,
                sender_name: b"ab".to_vec(),
                channel: 1,
                body: b"hi".to_vec(),
            }),
            [13, 0xC9, 5, 4, 3, 2, 1, 2, b'a', b'b', 1, 2, b'h', b'i']
        );
        // The empty lists the spec answers the sign-in queries with.
        assert_eq!(
            framed(ServerMessage::FriendList(Vec::new())),
            [4, 0xCB, 7, 0, 0]
        );
        assert_eq!(
            framed(ServerMessage::IgnoreList(Vec::new())),
            [4, 0xCB, 8, 0, 0]
        );
        let ab = ListedAccount {
            account_id: 0x0102_0304,
            name: b"ab".to_vec(),
            status: FriendStatus::Offline,
        };
        assert_eq!(
            framed(ServerMessage::FriendList(vec![ab.clone()])),
            [12, 0xCB, 7, 1, 0, 4, 3, 2, 1, 2, b'a', b'b', 2]
        );
        assert_eq!(
            framed(ServerMessage::IgnoreList(vec![ab.clone()])),
            [11, 0xCB, 8, 1, 0, 4, 3, 2, 1, 2, b'a', b'b']
        );
        assert_eq!(
            framed(ServerMessage::FoundPlayers(vec![ab.clone()])),
            [11, 0xCB, 6, 1, 0, 4, 3, 2, 1, 2, b'a', b'b']
        );
        assert_eq!(
            framed(ServerMessage::FriendStatuses(vec![ab])),
            [9, 0xCB, 9, 1, 0, 4, 3, 2, 1, 2]
        );
        assert_eq!(
            framed(ServerMessage::FriendshipResult {
                action: friendship_actions_enum::remove_ignorable,
                accepted: true,
            }),
            [3, 0xCB, 5, b'4']
        );
        assert_eq!(
            framed(ServerMessage::FriendshipResult {
                action: friendship_actions_enum::add_friend,
                accepted: false,
            }),
            [3, 0xCB, 0, 0]
        );
        assert_eq!(
            framed(ServerMessage::Important {
                sender_type: 5,
                sender_account_id: 9,
                sender_name: b"ab".to_vec(),
                message_type: 0,
                text: b"hi".to_vec(),
                message_id: 3,
            }),
            [
                17, 0xCC, 5, 9, 0, 0, 0, 2, b'a', b'b', 0, 2, b'h', b'i', 3, 0, 0, 0
            ]
        );
    }

    #[test]
    fn cuts_strings_to_the_clients_buffers() {
        let bytes = framed(ServerMessage::SignInInfo {
            local_name: vec![b'n'; 80],
        });
        assert_eq!(bytes[2], 63);
        assert_eq!(bytes.len(), 3 + 63);
    }
}
