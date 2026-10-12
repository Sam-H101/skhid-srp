//! The chat ("messaging") server. The stock client opens a second
//! connection to the lobby port for it; the first byte tells the two
//! protocols apart (see `handle_connection`).
//!
//! It relays typed lines between the connected players: general chat to
//! everyone, private (`/name text`) to that player, and the match channel to
//! the players subscribed to the same match. It keeps each account's friend
//! and ignore lists (in memory, with the account), finds players by name,
//! and tells which friends are online: those in the chat.
//!
//! The client also reads server notices from it, told apart by their
//! sender type (`network_client` lobby messaging, 0x5C96D9): from the stats
//! server, `#pc:[n]` sets the lobby's online counter
//! (`lobby_menu::on_stats_message_arrived`); from the matchmaker, `#q:[n]`
//! the place on the queue screen (`on_match_message_arrived`).

mod message;

use message::{ClientMessage, FriendshipRequest, ListedAccount, NAME_CAPACITY, ServerMessage};

use crate::UnknownSessions;
use session::{LockExt, SessionStore};
use vostok::network_client::TcpClient;
use vostok::network_packet::TcpPacket;
use vostok::serde::Serialize;

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[repr(u8)]
#[rustfmt::skip]
#[expect(non_camel_case_types)]
#[expect(dead_code)]
pub enum messaging_server_message_types_enum {
    messaging_server_connection_successful = 0xC8, // 1
    messaging_server_message               = 0xC9,
    messaging_server_sign_in_info          = 0xCB, // ???
    messaging_friendship_status            = 0xCC,
    messaging_server_invalid_message_type  = 0xFF,
}

#[repr(u8)]
#[rustfmt::skip]
#[expect(non_camel_case_types)]
#[expect(dead_code)]
pub enum messaging_client_message_types_enum {
    messaging_client_message              = 0xC1,       // 193
    messaging_client_sign_in_info         = 0xC3, // 1  // 195
    messaging_friendship_action           = 0xC4,       // 196
    messaging_client_subscription         = 0xC5, // 2  // 197
    messaging_client_invalid_message_type = 0xC7,
}

#[repr(u8)]
#[rustfmt::skip]
#[expect(non_camel_case_types)]
#[expect(dead_code)]
pub enum messaging_client_type_enum {
    unknown_client_type                = 0x0,
    login_server_client_type           = 0x1,
    lobby_server_client_type           = 0x2,
    match_server_client_type           = 0x3,
    message_server_client_type         = 0x4,
    account_client_type                = 0x5,
    administrative_client_type         = 0x6,
    match_maker_server_client_type     = 0x7,
    stats_processor_server_client_type = 0x8,
}

#[repr(u8)]
#[rustfmt::skip]
#[expect(non_camel_case_types)]
#[expect(dead_code)]
pub enum messaging_message_channel_enum {
    server_message_channel = 0x0,
    player_general_channel = 0x1,
    player_system_channel  = 0x2,
    player_clan_channel    = 0x3,
    player_private_channel = 0x4,
    player_match_channel   = 0x5,
    player_team1_channel   = 0x6,
    player_team2_channel   = 0x7,
    player_squad_channel   = 0x8,
    max_channel_num        = 0x9,
}

#[repr(u8)]
#[rustfmt::skip]
#[expect(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum friendship_actions_enum {
    add_friend            = 0x0,
    remove_friend         = 0x1,
    add_ignorable         = 0x2,
    remove_ignorable      = 0x3,
    find_players          = 0x4,
    query_friend_list     = 0x5,
    query_ignore_list     = 0x6,
    update_friends_status = 0x7,
}

/// Who a chat line goes to, by its `messaging_message_channel_enum`.
fn receives(channel: u8, channel_id: u32, receiver: &[u8], member: &Member) -> bool {
    use messaging_message_channel_enum as channels;

    match channel {
        _ if channel == channels::player_general_channel as u8 => true,
        _ if channel == channels::player_private_channel as u8 => {
            member.name.eq_ignore_ascii_case(receiver)
        }
        // The client sends its match id here, and nothing outside a match.
        _ if channel == channels::player_match_channel as u8 => {
            channel_id != 0 && channel_id != u32::MAX && member.match_channel_id == channel_id
        }
        // Clan, team and squad lines: the client does not send them (no
        // clans, squads or match chat id), or srp has no such group.
        _ => false,
    }
}

/// Everyone signed in to the chat, shared by the connection threads.
#[derive(Default)]
pub struct ChatRoom {
    next_member_id: AtomicU64,
    members: Mutex<HashMap<u64, Member>>,
    /// The players and matches last announced to everyone.
    announced: Mutex<Option<(usize, usize)>>,
    /// Stats notices for accounts not in the chat yet, sent when they join.
    waiting_notices: Mutex<HashMap<u32, Vec<String>>>,
}

struct Member {
    session_id: u32,
    account_id: u32,
    name: Vec<u8>,
    /// The match channel the client subscribed to (0 outside a match).
    match_channel_id: u32,
    writer: Arc<Mutex<TcpStream>>,
}

/// Removes a member from the room however its connection ends.
struct Membership<'a> {
    room: &'a ChatRoom,
    id: u64,
}

impl Drop for Membership<'_> {
    fn drop(&mut self) {
        self.room.members.locked().remove(&self.id);
    }
}

impl ChatRoom {
    fn join(&self, member: Member) -> Membership<'_> {
        let id = self.next_member_id.fetch_add(1, Ordering::Relaxed);
        self.members.locked().insert(id, member);
        Membership { room: self, id }
    }

    fn subscribe_to_match(&self, id: u64, match_channel_id: u32) {
        if let Some(member) = self.members.locked().get_mut(&id) {
            member.match_channel_id = match_channel_id;
        }
    }

    /// Whether the account has a client in the chat.
    fn is_online(&self, account_id: u32) -> bool {
        self.members
            .locked()
            .values()
            .any(|member| member.account_id == account_id)
    }

    /// The players signed in to the chat, each account once.
    fn players_online(&self) -> usize {
        let members = self.members.locked();
        let accounts: std::collections::HashSet<u32> =
            members.values().map(|member| member.account_id).collect();
        accounts.len()
    }

    /// Tell everyone the players online and the matches being played, if
    /// either changed since they last heard.
    pub fn announce_online(&self, matches: usize) {
        let counts = (self.players_online(), matches);
        {
            let mut announced = self.announced.locked();
            if *announced == Some(counts) {
                return;
            }
            *announced = Some(counts);
        }
        let writers: Vec<_> = self
            .members
            .locked()
            .values()
            .map(|member| member.writer.clone())
            .collect();
        for writer in &writers {
            if let Err(error) = send(writer, online_notice(counts.0, counts.1)) {
                log::debug!("Online count not delivered: {error}");
            }
        }
    }

    /// Send an account a stats notice (a match reward), now if it is in the
    /// chat, or when it joins.
    pub fn tell_account(&self, account_id: u32, text: String) {
        let writers: Vec<_> = self
            .members
            .locked()
            .values()
            .filter(|member| member.account_id == account_id)
            .map(|member| member.writer.clone())
            .collect();
        if writers.is_empty() {
            self.waiting_notices
                .locked()
                .entry(account_id)
                .or_default()
                .push(text);
            return;
        }
        for writer in &writers {
            if let Err(error) = send(writer, chat_notice(text.clone())) {
                log::debug!("Stats notice not delivered: {error}");
            }
        }
    }

    /// Show a waiting player its place on the queue screen.
    pub fn tell_queue_place(&self, session_id: u32, place: usize) {
        let writers: Vec<_> = self
            .members
            .locked()
            .values()
            .filter(|member| member.session_id == session_id)
            .map(|member| member.writer.clone())
            .collect();
        for writer in &writers {
            let notice = notice(
                messaging_client_type_enum::match_maker_server_client_type,
                format!("#q:[{place}]"),
            );
            if let Err(error) = send(writer, notice) {
                log::debug!("Queue place not delivered: {error}");
            }
        }
    }

    /// Send a typed line to everyone it is for; returns how many got it.
    fn relay(
        &self,
        from: u64,
        channel_id: u32,
        receiver: &[u8],
        channel: u8,
        body: &[u8],
    ) -> usize {
        let (message, writers) = {
            let members = self.members.locked();
            let Some(sender) = members.get(&from) else {
                return 0;
            };
            let message = ServerMessage::Text {
                sender_type: messaging_client_type_enum::account_client_type as u8,
                sender_account_id: sender.account_id,
                sender_name: sender.name.clone(),
                channel,
                body: body.to_vec(),
            };
            let writers: Vec<_> = members
                .iter()
                .filter(|(id, member)| {
                    **id != from && receives(channel, channel_id, receiver, member)
                })
                .map(|(_, member)| member.writer.clone())
                .collect();
            (message, writers)
        };
        for writer in &writers {
            // A recipient that is gone is noticed and removed by its own thread.
            if let Err(error) = send(writer, message.clone()) {
                log::debug!("Chat line not delivered: {error}");
            }
        }
        writers.len()
    }
}

/// A server notice, which the client reads by its sender type.
fn notice(sender: messaging_client_type_enum, body: String) -> ServerMessage {
    ServerMessage::Text {
        sender_type: sender as u8,
        sender_account_id: 0,
        sender_name: Vec::new(),
        channel: messaging_message_channel_enum::server_message_channel as u8,
        body: body.into_bytes(),
    }
}

/// A stats notice shown in the chat, on the system channel the client uses
/// for its own notices.
fn chat_notice(text: String) -> ServerMessage {
    ServerMessage::Text {
        sender_type: messaging_client_type_enum::stats_processor_server_client_type as u8,
        sender_account_id: 0,
        sender_name: Vec::new(),
        channel: messaging_message_channel_enum::player_system_channel as u8,
        body: text.into_bytes(),
    }
}

/// The online counts from the stats server. The client shows `#pc` in the
/// lobby's status bar; this build reads no `#mc`, sent for a client that
/// does. Without a `Player [` the client adds nothing to the chat.
fn online_notice(players: usize, matches: usize) -> ServerMessage {
    notice(
        messaging_client_type_enum::stats_processor_server_client_type,
        format!("#pc:[{players}] #mc:[{matches}]"),
    )
}

fn send(writer: &Mutex<TcpStream>, message: ServerMessage) -> std::io::Result<()> {
    let mut packet = TcpPacket::new();
    message.serialize(&mut packet);
    writer.locked().write_all(packet.get_buffer())
}

/// The most accounts on a friend or ignore list. The client keeps any
/// number; this keeps a list within one frame.
const MAX_LISTED: usize = 200;
/// The most players a search finds.
const MAX_FOUND: usize = 50;

/// The answer to a friendship request from `account_id`'s client
/// (`messaging_client::on_packet_received`, 0x5C9780). Adding a friend or
/// an ignored player is one-sided and at once: the client has no
/// invitations. After an accepted change (`'4'`) the client asks for the
/// list again; anything else it only logs. It asks for its friends'
/// statuses 10 s after each friend list or statuses answer.
fn friendship_reply(
    store: &SessionStore,
    room: &ChatRoom,
    account_id: u32,
    request: &FriendshipRequest,
) -> ServerMessage {
    let action = request.action();
    let Some(account) = store.account(account_id) else {
        // A standalone lobby's default account has no lists.
        return match request {
            FriendshipRequest::QueryFriendList => ServerMessage::FriendList(Vec::new()),
            FriendshipRequest::QueryIgnoreList => ServerMessage::IgnoreList(Vec::new()),
            FriendshipRequest::FindPlayers { .. } => ServerMessage::FoundPlayers(Vec::new()),
            FriendshipRequest::UpdateFriendsStatus => ServerMessage::FriendStatuses(Vec::new()),
            _ => ServerMessage::FriendshipResult {
                action,
                accepted: false,
            },
        };
    };
    let listed = |ids: &[u32]| -> Vec<ListedAccount> {
        ids.iter()
            .filter_map(|id| {
                let name = store.account(*id)?.locked().name.clone().into_bytes();
                Some(ListedAccount {
                    account_id: *id,
                    name,
                    online: room.is_online(*id),
                })
            })
            .collect()
    };
    // Someone else's account, and not one more than the list holds.
    let addable = |target: u32, list: &[u32]| {
        target != account_id
            && store.account(target).is_some()
            && (list.contains(&target) || list.len() < MAX_LISTED)
    };
    match request {
        FriendshipRequest::AddFriend { account_id: target } => {
            let mut account = account.locked();
            let accepted = addable(*target, &account.friends);
            if accepted && !account.friends.contains(target) {
                account.friends.push(*target);
            }
            ServerMessage::FriendshipResult { action, accepted }
        }
        FriendshipRequest::AddIgnored { account_id: target } => {
            let mut account = account.locked();
            let accepted = addable(*target, &account.ignored);
            if accepted && !account.ignored.contains(target) {
                account.ignored.push(*target);
            }
            ServerMessage::FriendshipResult { action, accepted }
        }
        FriendshipRequest::RemoveFriend { account_id: target } => {
            account.locked().friends.retain(|id| id != target);
            ServerMessage::FriendshipResult {
                action,
                accepted: true,
            }
        }
        FriendshipRequest::RemoveIgnored { account_id: target } => {
            account.locked().ignored.retain(|id| id != target);
            ServerMessage::FriendshipResult {
                action,
                accepted: true,
            }
        }
        FriendshipRequest::FindPlayers { name } => ServerMessage::FoundPlayers(
            store
                .find_accounts(name, account_id, MAX_FOUND)
                .into_iter()
                .map(|(found, name)| ListedAccount {
                    account_id: found,
                    name: name.into_bytes(),
                    online: false,
                })
                .collect(),
        ),
        FriendshipRequest::QueryFriendList => {
            let friends = account.locked().friends.clone();
            ServerMessage::FriendList(listed(&friends))
        }
        FriendshipRequest::QueryIgnoreList => {
            let ignored = account.locked().ignored.clone();
            ServerMessage::IgnoreList(listed(&ignored))
        }
        FriendshipRequest::UpdateFriendsStatus => {
            let friends = account.locked().friends.clone();
            ServerMessage::FriendStatuses(listed(&friends))
        }
    }
}

/// Serve one chat connection until the client leaves. `reader` has already
/// buffered the first message; `writer` is the same socket.
pub fn run(
    store: &SessionStore,
    room: &ChatRoom,
    mut reader: TcpClient,
    writer: TcpStream,
    unknown: UnknownSessions,
) {
    let session_id = match reader.read::<ClientMessage>() {
        Ok(ClientMessage::SignInInfo { session_id, .. }) => session_id,
        Ok(message) => {
            log::warn!("Chat connection opened with {message:?} instead of a sign-in; closed it");
            return;
        }
        Err(error) if error.is_disconnect() => {
            log::info!("Chat connection closed before its sign-in");
            return;
        }
        Err(error) => {
            log::warn!("Unreadable chat sign-in ({error}); closed the connection");
            return;
        }
    };

    // The name the lobby shows for the account. The connection holds its
    // session while it lasts. The chat client knows no refusal: it logs any
    // answer but its sign-in (0xCB, `messaging_client::
    // sign_in_on_packet_received`, 0x799B42), and on a closed connection
    // says "Lost connection to messaging server" and reconnects every 3 s
    // (`messaging_client::on_error`, 0x766450; `network_client::tick`,
    // 0x5C63A8). So a session the login server didn't open, or one that has
    // ended, is just closed; the lobby, which refuses it too, sends the
    // client back to the login screen. A lobby without a login server uses
    // the lobby's default account instead.
    let hold = store.hold(session_id);
    let account = hold
        .as_ref()
        .and_then(|_| store.account_for_session(session_id));
    let (account_id, mut name) = match account {
        Some(account) => {
            let account = account.locked();
            (account.id, account.name.clone().into_bytes())
        }
        None if unknown == UnknownSessions::Refuse => {
            log::warn!("Chat session {session_id:#x} is unknown or has ended; closed it");
            return;
        }
        None => {
            log::warn!("Unknown chat session {session_id:#x}; using a default account");
            (1, b"dummy_name".to_vec())
        }
    };
    name.truncate(NAME_CAPACITY - 1);
    let display_name = String::from_utf8_lossy(&name).into_owned();

    let writer = Arc::new(Mutex::new(writer));
    let sign_in = ServerMessage::SignInInfo {
        local_name: name.clone(),
    };
    if let Err(error) = send(&writer, sign_in) {
        log::info!("Session {session_id:#x} left the chat before signing in: {error}");
        return;
    }
    let membership = room.join(Member {
        session_id,
        account_id,
        name,
        match_channel_id: 0,
        writer: writer.clone(),
    });
    log::info!("Session {session_id:#x} joined the chat as {display_name}");
    // The others hear of the newcomer with the next announcement.
    let online = online_notice(room.players_online(), store.match_count());
    if let Err(error) = send(&writer, online) {
        log::debug!("Online count not delivered: {error}");
    }
    let waiting = room.waiting_notices.locked().remove(&account_id);
    for text in waiting.into_iter().flatten() {
        if let Err(error) = send(&writer, chat_notice(text)) {
            log::debug!("Stats notice not delivered: {error}");
        }
    }

    loop {
        let message = match reader.read::<ClientMessage>() {
            Ok(message) => message,
            Err(error) if error.is_disconnect() => {
                log::info!("Session {session_id:#x} left the chat");
                return;
            }
            Err(error) => {
                log::warn!(
                    "Unreadable chat message from session {session_id:#x} ({error}); \
                     closed the connection"
                );
                return;
            }
        };
        match message {
            ClientMessage::SignInInfo { .. } => {
                log::warn!("Session {session_id:#x} signed in to the chat again; ignored");
            }
            ClientMessage::Subscription { channel_ids } => {
                let match_channel_id =
                    channel_ids[messaging_message_channel_enum::player_match_channel as usize];
                log::debug!("Session {session_id:#x} chat channels: {channel_ids:x?}");
                room.subscribe_to_match(membership.id, match_channel_id);
            }
            ClientMessage::Friendship(request) => {
                log::debug!("Session {session_id:#x} friendship request: {request:?}");
                let reply = friendship_reply(store, room, account_id, &request);
                if let Err(error) = send(&writer, reply) {
                    log::debug!("Friendship reply not delivered: {error}");
                }
            }
            ClientMessage::Text {
                channel_id,
                receiver,
                channel,
                body,
            } => {
                let recipients = room.relay(membership.id, channel_id, &receiver, channel, &body);
                log::info!(
                    "Chat from {display_name} on channel {channel} reached {recipients} player(s)"
                );
                log::debug!("Chat line: {}", String::from_utf8_lossy(&body));
            }
            ClientMessage::Unknown { message_type } => {
                log::warn!("Unknown chat message type {message_type:#x}; skipped");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::{IpAddr, Ipv4Addr, TcpListener};
    use std::time::{Duration, Instant};

    /// A chat server on a free local port, serving every connection with `run`.
    struct Server {
        store: Arc<SessionStore>,
        room: Arc<ChatRoom>,
        addr: std::net::SocketAddr,
    }

    impl Server {
        fn start() -> Self {
            let store = Arc::new(SessionStore::new());
            let room = Arc::new(ChatRoom::default());
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let (thread_store, thread_room) = (store.clone(), room.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let stream = stream.unwrap();
                    let (store, room) = (thread_store.clone(), thread_room.clone());
                    std::thread::spawn(move || {
                        let reader = TcpClient::new(stream.try_clone().unwrap());
                        run(&store, &room, reader, stream, UnknownSessions::Refuse);
                    });
                }
            });
            Self { store, room, addr }
        }

        /// Sign in as a fresh account named `email`, the way the client does.
        fn sign_in(&self, email: &str) -> Client {
            let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
            let account_id = self.store.get_or_create_account(ip, email);
            let session_id = self.store.create_session(account_id);
            let mut stream = TcpStream::connect(self.addr).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut sign_in = vec![6, 0xC3];
            sign_in.extend(session_id.to_le_bytes());
            sign_in.push(5);
            stream.write_all(&sign_in).unwrap();
            let mut client = Client { stream, account_id };
            // The account name: the part of the email before the `@`.
            let name = email.split('@').next().unwrap();
            let mut expected = vec![0xCB, name.len() as u8];
            expected.extend(name.as_bytes());
            assert_eq!(client.frame(), expected);
            client
        }

        fn wait_for_members(&self, count: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            while self.room.members.locked().len() != count {
                assert!(
                    Instant::now() < deadline,
                    "the room never had {count} members"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    struct Client {
        stream: TcpStream,
        account_id: u32,
    }

    impl Client {
        fn send(&mut self, frame: &[u8]) {
            let mut bytes = vec![frame.len() as u8];
            bytes.extend(frame);
            self.stream.write_all(&bytes).unwrap();
        }

        fn say(&mut self, channel: u8, channel_id: u32, receiver: &str, body: &str) {
            let mut frame = vec![0xC1];
            frame.extend(channel_id.to_le_bytes());
            frame.push(receiver.len() as u8);
            frame.extend(receiver.as_bytes());
            frame.push(channel);
            frame.push(body.len() as u8);
            frame.extend(body.as_bytes());
            self.send(&frame);
        }

        fn subscribe_to_match(&mut self, match_id: u32) {
            let mut frame = vec![0xC5];
            for channel_id in [0, u32::MAX, u32::MAX, 0, 0, match_id, 0, 0, 0] {
                frame.extend(channel_id.to_le_bytes());
            }
            self.send(&frame);
        }

        /// The next frame other than an online count.
        fn frame(&mut self) -> Vec<u8> {
            loop {
                let frame = self.any_frame();
                if !is_online_notice(&frame) {
                    return frame;
                }
            }
        }

        fn any_frame(&mut self) -> Vec<u8> {
            let mut len = [0_u8];
            self.stream.read_exact(&mut len).unwrap();
            let mut frame = vec![0; len[0] as usize];
            self.stream.read_exact(&mut frame).unwrap();
            frame
        }

        /// The next frame other than an online count, if one arrives within
        /// a short wait.
        fn maybe_frame(&mut self) -> Option<Vec<u8>> {
            self.stream
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            let frame = loop {
                let mut len = [0_u8];
                match self.stream.read_exact(&mut len) {
                    Ok(()) => {
                        let mut frame = vec![0; len[0] as usize];
                        self.stream.read_exact(&mut frame).unwrap();
                        if !is_online_notice(&frame) {
                            break Some(frame);
                        }
                    }
                    Err(_) => break None,
                }
            };
            self.stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            frame
        }
    }

    fn is_online_notice(frame: &[u8]) -> bool {
        frame.starts_with(&[0xC9, 8])
    }

    /// A stats notice as the client receives it.
    fn stats_notice(body: &str) -> Vec<u8> {
        let mut frame = vec![0xC9, 8, 0, 0, 0, 0, 0, 0];
        frame.push(body.len() as u8);
        frame.extend(body.as_bytes());
        frame
    }

    #[test]
    fn a_newcomer_hears_the_online_counts_and_everyone_hears_changes() {
        let server = Server::start();
        let mut alice = server.sign_in("alice");
        assert_eq!(alice.any_frame(), stats_notice("#pc:[1] #mc:[0]"));
        let mut bob = server.sign_in("bob");
        assert_eq!(bob.any_frame(), stats_notice("#pc:[2] #mc:[0]"));
        server.wait_for_members(2);

        server.room.announce_online(1);
        assert_eq!(alice.any_frame(), stats_notice("#pc:[2] #mc:[1]"));
        assert_eq!(bob.any_frame(), stats_notice("#pc:[2] #mc:[1]"));
        // Nothing new, nothing sent.
        server.room.announce_online(1);
        assert_eq!(alice.maybe_frame(), None);
    }

    #[test]
    fn a_reward_waits_for_its_account_to_join_the_chat() {
        let server = Server::start();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let account_id = server.store.get_or_create_account(ip, "late");
        server
            .room
            .tell_account(account_id, "Player [ late ] #e:[10]".to_owned());

        let mut late = server.sign_in("late");
        let body = "Player [ late ] #e:[10]";
        let mut expected = vec![0xC9, 8, 0, 0, 0, 0, 0, 2];
        expected.push(body.len() as u8);
        expected.extend(body.as_bytes());
        assert_eq!(late.any_frame(), stats_notice("#pc:[1] #mc:[0]"));
        assert_eq!(late.any_frame(), expected);
    }

    #[test]
    fn the_queue_place_reaches_only_its_player() {
        let server = Server::start();
        let mut alice = server.sign_in("alice");
        let mut bob = server.sign_in("bob");
        server.wait_for_members(2);
        let alice_session = server
            .room
            .members
            .locked()
            .values()
            .find(|member| member.name == b"alice")
            .unwrap()
            .session_id;

        server.room.tell_queue_place(alice_session, 3);

        let body = "#q:[3]";
        let mut expected = vec![0xC9, 7, 0, 0, 0, 0, 0, 0];
        expected.push(body.len() as u8);
        expected.extend(body.as_bytes());
        assert_eq!(alice.frame(), expected);
        assert_eq!(bob.maybe_frame(), None);
    }

    /// What a client receives for a line from `sender`.
    fn line_from(sender: &Client, name: &str, channel: u8, body: &str) -> Vec<u8> {
        let mut frame = vec![0xC9, 5];
        frame.extend(sender.account_id.to_le_bytes());
        frame.push(name.len() as u8);
        frame.extend(name.as_bytes());
        frame.push(channel);
        frame.push(body.len() as u8);
        frame.extend(body.as_bytes());
        frame
    }

    #[test]
    fn signs_in_with_the_account_name_and_starts_with_empty_lists() {
        let server = Server::start();
        let mut client = server.sign_in("tester@test.invalid");

        client.send(&[0xC4, 5]);
        assert_eq!(client.frame(), [0xCC, 5, 0, 0]);
        client.send(&[0xC4, 6]);
        assert_eq!(client.frame(), [0xCC, 6, 0, 0]);
        client.send(&[0xC4, 7]);
        assert_eq!(client.frame(), [0xCC, 7, 0, 0]);
    }

    fn with_id(action: u8, account_id: u32) -> Vec<u8> {
        let mut frame = vec![0xC4, action];
        frame.extend(account_id.to_le_bytes());
        frame
    }

    /// A list entry: id, name, and the online byte if `online` is given.
    fn entry(account_id: u32, name: &str, online: Option<bool>) -> Vec<u8> {
        let mut bytes = account_id.to_le_bytes().to_vec();
        bytes.push(name.len() as u8);
        bytes.extend(name.as_bytes());
        bytes.extend(online.map(u8::from));
        bytes
    }

    fn list(action: u8, entries: &[Vec<u8>]) -> Vec<u8> {
        let mut frame = vec![0xCC, action];
        frame.extend((entries.len() as u16).to_le_bytes());
        frame.extend(entries.concat());
        frame
    }

    #[test]
    fn a_friend_is_added_at_once_and_shows_online_while_in_the_chat() {
        let server = Server::start();
        let mut alice = server.sign_in("alice");
        let bob = server.sign_in("bob");
        let carol_id = server
            .store
            .get_or_create_account(IpAddr::V4(Ipv4Addr::LOCALHOST), "carol");
        server.wait_for_members(2);

        alice.send(&with_id(0, bob.account_id));
        assert_eq!(alice.frame(), [0xCC, 0, b'4']);
        alice.send(&with_id(0, carol_id));
        assert_eq!(alice.frame(), [0xCC, 0, b'4']);
        // Again: still accepted, listed once.
        alice.send(&with_id(0, bob.account_id));
        assert_eq!(alice.frame(), [0xCC, 0, b'4']);
        alice.send(&[0xC4, 5]);
        assert_eq!(
            alice.frame(),
            list(
                5,
                &[
                    entry(bob.account_id, "bob", Some(true)),
                    entry(carol_id, "carol", Some(false)),
                ]
            )
        );

        drop(bob);
        server.wait_for_members(1);
        alice.send(&[0xC4, 7]);
        let mut statuses = vec![0xCC, 7, 2, 0];
        statuses.extend(alice_friend_status(&server, "bob", false));
        statuses.extend(carol_id.to_le_bytes());
        statuses.push(0);
        assert_eq!(alice.frame(), statuses);

        alice.send(&with_id(1, carol_id));
        assert_eq!(alice.frame(), [0xCC, 1, b'4']);
        alice.send(&[0xC4, 5]);
        let bob_id = server
            .store
            .get_or_create_account(IpAddr::V4(Ipv4Addr::LOCALHOST), "bob");
        assert_eq!(alice.frame(), list(5, &[entry(bob_id, "bob", Some(false))]));
    }

    fn alice_friend_status(server: &Server, email: &str, online: bool) -> Vec<u8> {
        let id = server
            .store
            .get_or_create_account(IpAddr::V4(Ipv4Addr::LOCALHOST), email);
        let mut bytes = id.to_le_bytes().to_vec();
        bytes.push(u8::from(online));
        bytes
    }

    #[test]
    fn oneself_and_unknown_accounts_cannot_be_added() {
        let server = Server::start();
        let mut alice = server.sign_in("alice");
        alice.send(&with_id(0, alice.account_id));
        assert_eq!(alice.frame(), [0xCC, 0, 0]);
        alice.send(&with_id(2, 0xBAD));
        assert_eq!(alice.frame(), [0xCC, 2, 0]);
        alice.send(&[0xC4, 5]);
        assert_eq!(alice.frame(), [0xCC, 5, 0, 0]);
    }

    #[test]
    fn the_ignore_list_keeps_names_without_statuses() {
        let server = Server::start();
        let mut alice = server.sign_in("alice");
        let bob = server.sign_in("bob");
        alice.send(&with_id(2, bob.account_id));
        assert_eq!(alice.frame(), [0xCC, 2, b'4']);
        alice.send(&[0xC4, 6]);
        assert_eq!(
            alice.frame(),
            list(6, &[entry(bob.account_id, "bob", None)])
        );
        // Ignoring doesn't touch the friend list.
        alice.send(&[0xC4, 5]);
        assert_eq!(alice.frame(), [0xCC, 5, 0, 0]);
        alice.send(&with_id(3, bob.account_id));
        assert_eq!(alice.frame(), [0xCC, 3, b'4']);
        alice.send(&[0xC4, 6]);
        assert_eq!(alice.frame(), [0xCC, 6, 0, 0]);
    }

    #[test]
    fn players_are_found_by_part_of_their_name_but_not_oneself() {
        let server = Server::start();
        let mut anna = server.sign_in("anna");
        let hannah = server.sign_in("Hannah");
        server.sign_in("bob");
        let mut search = vec![0xC4, 4, 3];
        search.extend(b"ANN");
        anna.send(&search);
        assert_eq!(
            anna.frame(),
            list(4, &[entry(hannah.account_id, "Hannah", None)])
        );
    }

    #[test]
    fn the_lists_stay_with_the_account() {
        let server = Server::start();
        let mut alice = server.sign_in("alice");
        let bob = server.sign_in("bob");
        alice.send(&with_id(0, bob.account_id));
        assert_eq!(alice.frame(), [0xCC, 0, b'4']);
        drop(alice);

        let mut again = server.sign_in("alice");
        again.send(&[0xC4, 5]);
        assert_eq!(
            again.frame(),
            list(5, &[entry(bob.account_id, "bob", Some(true))])
        );
    }

    #[test]
    fn a_closed_connection_leaves_the_room() {
        let server = Server::start();
        let client = server.sign_in("leaver@test.invalid");
        server.wait_for_members(1);

        drop(client);

        server.wait_for_members(0);
    }

    #[test]
    fn general_lines_reach_everyone_else() {
        let server = Server::start();
        let mut alice = server.sign_in("alice");
        let mut bob = server.sign_in("bob");
        let mut carol = server.sign_in("carol");
        server.wait_for_members(3);

        alice.say(1, 0, "", "hi all");

        let expected = line_from(&alice, "alice", 1, "hi all");
        assert_eq!(bob.frame(), expected);
        assert_eq!(carol.frame(), expected);
        assert_eq!(alice.maybe_frame(), None, "the client shows its own line");
    }

    #[test]
    fn private_lines_reach_only_the_named_player() {
        let server = Server::start();
        let mut alice = server.sign_in("alice");
        let mut bob = server.sign_in("bob");
        let mut carol = server.sign_in("carol");
        server.wait_for_members(3);

        alice.say(4, 0, "Bob", "psst");

        assert_eq!(bob.frame(), line_from(&alice, "alice", 4, "psst"));
        assert_eq!(carol.maybe_frame(), None);
    }

    #[test]
    fn match_lines_reach_only_the_same_match() {
        let server = Server::start();
        let mut alice = server.sign_in("alice");
        let mut bob = server.sign_in("bob");
        let mut carol = server.sign_in("carol");
        alice.subscribe_to_match(0x123);
        bob.subscribe_to_match(0x123);
        carol.subscribe_to_match(0x456);
        server.wait_for_members(3);
        // Subscriptions are handled in order with the lines that follow,
        // but bob's and carol's come from other connections.
        std::thread::sleep(Duration::from_millis(100));

        alice.say(5, 0x123, "", "gg");

        assert_eq!(bob.frame(), line_from(&alice, "alice", 5, "gg"));
        assert_eq!(carol.maybe_frame(), None);
    }
}
