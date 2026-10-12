//! The whole lobby over real localhost UDP: its socket, its thread and its
//! timers, with scripted clients on sockets of their own. Every wait has a
//! deadline.

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use session::v026g::GameAccounts;
use session::{LockExt, SessionStore};
use survarium::v026g::lobby::client_model::{Decoded, FriendshipPush, Permitted};
use survarium::v026g::lobby::requests::INVENTORY_SLOT;
use survarium::v026g::lobby::{ClientState, channel};
use survarium::v026g::prices;

use super::client::{TestClient, request};
use super::{TestHost, denial, find, sent_to_match, signed_in};
use crate::v026g::{Inbox, Lobby, Settings};

/// No wait is longer.
const DEADLINE: Duration = Duration::from_secs(8);

struct Server {
    addr: SocketAddr,
    store: Arc<SessionStore>,
    accounts: Arc<GameAccounts>,
    host: Arc<TestHost>,
    inbox: Arc<Inbox>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl Server {
    /// A lobby on a free port whose queue gathers for `gather`.
    fn start(gather: Duration, link_timeout: Duration) -> Self {
        let store = Arc::new(SessionStore::with_gather_window(gather));
        let accounts = Arc::new(GameAccounts::new());
        let host = Arc::new(TestHost::default());
        let inbox = Inbox::new();
        let settings = Settings {
            tick: Duration::from_millis(50),
            link_timeout,
            // The session of a silent link waits as long again.
            session_grace: link_timeout,
            ..Settings::local("127.0.0.1:0")
        };
        let lobby = Lobby::bind(
            settings,
            store.clone(),
            accounts.clone(),
            Arc::new(session::v026g::rewards::Results::new()),
            host.clone(),
            inbox.clone(),
        )
        .unwrap();
        let addr = lobby.local_addr().unwrap();
        let thread = std::thread::spawn(move || lobby.run());
        Self {
            addr,
            store,
            accounts,
            host,
            inbox,
            thread: Some(thread),
        }
    }

    fn quick() -> Self {
        Self::start(Duration::ZERO, Duration::from_secs(30))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.inbox.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Peer {
    socket: UdpSocket,
    server: SocketAddr,
    client: TestClient,
    account_id: u32,
    session_id: u32,
}

impl Peer {
    /// A client with a session, not yet connected.
    fn new(server: &Server, name: &str) -> Self {
        let (account_id, session_id) = signed_in(&server.store, name);
        Self::with_session(server, account_id, session_id)
    }

    fn with_session(server: &Server, account_id: u32, session_id: u32) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        Self {
            socket,
            server: server.addr,
            client: TestClient::new(),
            account_id,
            session_id,
        }
    }

    /// Send what is due and take what has come.
    fn pump(&mut self) {
        for datagram in self.client.poll(Instant::now()) {
            let _ = self.socket.send_to(&datagram, self.server);
        }
        let mut buffer = [0_u8; 2048];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, from)) if from == self.server => self.client.receive(&buffer[..len]),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
                Err(_) => break,
            }
        }
    }

    fn ask(&mut self, (id, body): (u8, Vec<u8>)) {
        self.client.send(id, &body);
    }

    fn take(&mut self) -> Vec<Decoded> {
        self.client.ids.clear();
        std::mem::take(&mut self.client.decoded)
    }
}

/// Run the peers until `done`, or fail with `what` at the deadline.
fn until(peers: &mut [&mut Peer], what: &str, done: impl Fn(&[&mut Peer]) -> bool) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        for peer in peers.iter_mut() {
            peer.pump();
        }
        if done(peers) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Run the peers for a while.
fn run_for(peers: &mut [&mut Peer], time: Duration) {
    let end = Instant::now() + time;
    while Instant::now() < end {
        for peer in peers.iter_mut() {
            peer.pump();
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn has(peer: &Peer, pick: impl Fn(&Decoded) -> bool) -> bool {
    peer.client.decoded.iter().any(pick)
}

/// Connect and wait for the whole sign-in push (the squad info is its
/// last message).
fn connect(peer: &mut Peer) {
    peer.client.sign_in(peer.session_id, true);
    until(&mut [peer], "the sign-in pushes", |peers| {
        let ids = &peers[0].client.ids;
        ids.contains(&0x85)
            && ids.contains(&0xA1)
            && ids.iter().filter(|id| **id == 0xA3).count() == 2
    });
}

fn assert_clean(peers: &[&Peer]) {
    for peer in peers {
        assert_eq!(peer.client.faults, Vec::<String>::new());
        assert_eq!(peer.client.waiting_in_order(), 0);
    }
}

#[test]
fn a_client_signs_in_over_udp_and_shops() {
    let server = Server::quick();
    let mut alice = Peer::new(&server, "Alice");
    // Gear is sold by the factions, from a reputation level on.
    {
        let account = server.accounts.get_or_create(alice.account_id, "Alice");
        let mut account = account.locked();
        for faction in 1..=4 {
            account.add_reputation(faction, 40_000);
        }
    }
    connect(&mut alice);
    assert!(alice.client.model.is_signed_in());
    assert_eq!(alice.client.ids[0], 0x80);
    let ordered: Vec<u8> = alice
        .client
        .ids
        .iter()
        .copied()
        .filter(|id| !matches!(id, 0x80 | 0xA1))
        .collect();
    assert_eq!(
        ordered,
        [
            0x8F, 0x90, 0x8B, 0x96, 0x8E, 0x8D, 0x94, 0x8A, 0x95, 0x89, 0x87, 0x8C, 0x92, 0x9C,
            0x9D, 0x9B, 0x85, 0x91, 0xA3, 0xA3, 0x99
        ]
    );
    assert_eq!(alice.client.model.inventory().len(), 5);
    let decoded = alice.take();
    assert!(decoded.iter().any(|message| matches!(
        message,
        Decoded::Account(own) if own.body.name == "Alice" && own.active_kit == 1
    )));
    assert!(
        decoded
            .iter()
            .any(|message| matches!(message, Decoded::Prices(list) if !list.is_empty()))
    );
    assert_eq!(server.store.players_online(), 1);

    // A purchase it cannot pay for: the game's text.
    let dear = prices::price_list()
        .iter()
        .max_by_key(|record| record.cost)
        .unwrap();
    alice.ask(request::buy(
        dear.dict_id,
        1,
        dear.faction_id,
        INVENTORY_SLOT,
        1,
    ));
    until(&mut [&mut alice], "the refusal", |peers| {
        denial(&peers[0].client.decoded, 0x47).is_some()
    });
    assert_eq!(
        denial(&alice.take(), 0x47).as_deref(),
        Some("st_not_enougth_money")
    );
    // With the money, it is bought, and the pushes follow.
    server
        .accounts
        .get(alice.account_id)
        .unwrap()
        .locked()
        .generic_money = dear.cost + 10;
    // The lobby notices a balance changed behind its back.
    let rich = dear.cost + 10;
    until(&mut [&mut alice], "the new balance", |peers| {
        has(
            peers[0],
            |message| matches!(message, Decoded::Money { generic, .. } if *generic == rich),
        )
    });
    alice.take();
    alice.ask(request::buy(
        dear.dict_id,
        1,
        dear.faction_id,
        INVENTORY_SLOT,
        1,
    ));
    until(&mut [&mut alice], "the purchase", |peers| {
        has(peers[0], |message| {
            matches!(message, Decoded::Money { generic: 10, .. })
        })
    });
    assert_eq!(alice.client.ids, [0x83, 0x89, 0x87, 0x8B]);
    assert!(matches!(
        alice.client.decoded[0],
        Decoded::Permitted(Permitted::Bought(_))
    ));
    assert!(alice.client.model.inventory().len() >= 3);
    // Skills and a move, over the wire.
    alice.take();
    alice.ask(request::move_item(1, 1, 18, 7, INVENTORY_SLOT, 1));
    until(&mut [&mut alice], "the move", |peers| {
        has(
            peers[0],
            |message| matches!(message, Decoded::Account(own) if own.body.kits[0].slots[7].is_none()),
        )
    });
    alice.ask(request::play(1));
    until(&mut [&mut alice], "Play refused", |peers| {
        denial(&peers[0].client.decoded, 0x2F).is_some()
    });
    assert_eq!(
        denial(&alice.take(), 0x2F).as_deref(),
        Some("st_profile_check_failed_weapon")
    );
    assert_clean(&[&alice]);
}

#[test]
fn an_unknown_or_ended_session_is_turned_away_over_udp() {
    let server = Server::quick();
    let mut stranger = Peer::with_session(&server, 0, 0x00BA_D000);
    stranger.client.sign_in(stranger.session_id, true);
    until(&mut [&mut stranger], "the refusal", |peers| {
        peers[0].client.disconnected
    });
    assert!(stranger.client.decoded.is_empty());

    let (account_id, session_id) = signed_in(&server.store, "Alice");
    assert!(server.store.end_session(session_id));
    let mut ended = Peer::with_session(&server, account_id, session_id);
    ended.client.sign_in(session_id, true);
    until(&mut [&mut ended], "the refusal", |peers| {
        peers[0].client.disconnected
    });
    assert!(ended.client.decoded.is_empty());
    assert_eq!(server.store.players_online(), 0);
    assert_clean(&[&stranger, &ended]);
}

#[test]
fn a_lone_player_waits_and_two_go_to_the_same_match() {
    let server = Server::quick();
    let mut alice = Peer::new(&server, "Alice");
    let mut bob = Peer::new(&server, "Bob");
    connect(&mut alice);
    connect(&mut bob);
    alice.take();

    // Alone, also with a gather window of nothing: no match.
    alice.ask(request::play(1));
    run_for(&mut [&mut alice, &mut bob], Duration::from_millis(600));
    let decoded = alice.take();
    let waits = decoded
        .iter()
        .filter(|message| {
            matches!(
                message,
                Decoded::ClientState {
                    state: ClientState::InQueue {
                        place: 1,
                        queue_total: 1,
                        ..
                    },
                    ..
                }
            )
        })
        .count();
    assert!(waits >= 3, "{waits} queue updates");
    assert!(sent_to_match(&decoded).is_none());
    assert!(server.host.made.locked().is_empty());

    // Leaving the queue: back in the lobby.
    alice.ask(request::leave(1, false));
    until(&mut [&mut alice, &mut bob], "the lobby state", |peers| {
        has(peers[0], |message| {
            matches!(
                message,
                Decoded::ClientState {
                    state: ClientState::Lobby,
                    ..
                }
            )
        })
    });
    assert!(server.store.matchmaking().busy_sessions().is_empty());
    alice.take();
    bob.take();

    // Both press Play: one match, on opposite teams.
    alice.ask(request::play(1));
    bob.ask(request::play(1));
    until(&mut [&mut alice, &mut bob], "the match", |peers| {
        peers
            .iter()
            .all(|peer| sent_to_match(&peer.client.decoded).is_some())
    });
    let (alice_match, alice_team) = sent_to_match(&alice.take()).unwrap();
    let (bob_match, bob_team) = sent_to_match(&bob.take()).unwrap();
    assert_eq!(alice_match, bob_match);
    assert_ne!(alice_team, bob_team);
    let made = server.host.made.locked().clone();
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].0, alice_match);
    let sessions: Vec<u32> = made[0]
        .1
        .players
        .iter()
        .map(|player| player.session_id)
        .collect();
    assert!(sessions.contains(&alice.session_id) && sessions.contains(&bob.session_id));
    assert_clean(&[&alice, &bob]);
}

#[test]
fn two_clients_chat_and_become_friends() {
    let server = Server::quick();
    let mut alice = Peer::new(&server, "Alice");
    let mut bob = Peer::new(&server, "Bob");
    connect(&mut alice);
    connect(&mut bob);
    alice.ask(request::subscribe());
    bob.ask(request::subscribe());
    run_for(&mut [&mut alice, &mut bob], Duration::from_millis(100));
    alice.take();
    bob.take();

    alice.ask(request::chat(2, "", channel::GENERAL, "привет"));
    until(&mut [&mut alice, &mut bob], "the chat line", |peers| {
        has(peers[1], |message| {
            matches!(
                message,
                Decoded::ChatText { sender_name, channel: channel::GENERAL, body, .. }
                    if sender_name == "Alice" && body == "привет"
            )
        })
    });
    bob.ask(request::chat(0, "Alice", channel::PRIVATE, "hi"));
    until(&mut [&mut alice, &mut bob], "the private line", |peers| {
        has(peers[0], |message| {
            matches!(
                message,
                Decoded::ChatText { channel: channel::PRIVATE, body, .. } if body == "hi"
            )
        })
    });

    // A friend request, accepted.
    alice.ask(request::important("Bob", 0, ""));
    until(&mut [&mut alice, &mut bob], "the friend request", |peers| {
        has(peers[1], |message| {
            matches!(message, Decoded::Important { kind: 0, .. })
        })
    });
    let message_id = find(&bob.take(), |message| match message {
        Decoded::Important {
            kind: 0,
            message_id,
            ..
        } => Some(*message_id),
        _ => None,
    })
    .unwrap();
    bob.ask(request::answer(message_id, 0));
    let friend_of = |peer: &Peer, account_id: u32, status: u8| {
        has(peer, |message| {
            matches!(
                message,
                Decoded::Friendship(FriendshipPush::Friends(friends))
                    if friends.len() == 1
                        && friends[0].account_id == account_id
                        && friends[0].status == status
            )
        })
    };
    let (alice_id, bob_id) = (alice.account_id, bob.account_id);
    until(&mut [&mut alice, &mut bob], "the friend lists", |peers| {
        friend_of(peers[0], bob_id, 0) && friend_of(peers[1], alice_id, 0)
    });
    // Bob leaves: Alice's client is told he is offline.
    bob.client.leave();
    until(
        &mut [&mut alice, &mut bob],
        "the friend's status",
        |peers| {
            has(peers[0], |message| {
                matches!(
                    message,
                    Decoded::Friendship(FriendshipPush::Statuses(statuses))
                        if statuses[..] == [(bob_id, 2)]
                )
            })
        },
    );
    assert!(server.store.has_ended(bob.session_id));
    assert_clean(&[&alice, &bob]);
}

#[test]
fn a_squad_of_two_queues_together_over_udp() {
    let server = Server::quick();
    let mut alice = Peer::new(&server, "Alice");
    let mut bob = Peer::new(&server, "Bob");
    let mut carol = Peer::new(&server, "Carol");
    connect(&mut alice);
    connect(&mut bob);
    connect(&mut carol);
    let important = |peer: &Peer, kind: u8| {
        find(&peer.client.decoded, |message| match message {
            Decoded::Important {
                kind: got,
                message_id,
                ..
            } if *got == kind => Some(*message_id),
            _ => None,
        })
    };
    alice.ask(request::important("Bob", 2, "invite"));
    until(&mut [&mut alice, &mut bob], "the invite", |peers| {
        important(peers[1], 2).is_some()
    });
    let invite = important(&bob, 2).unwrap();
    // The client's order: "membership accepted", then the answer.
    bob.ask(request::important("Alice", 3, "membership accepted"));
    bob.ask(request::answer(invite, 0));
    until(&mut [&mut alice, &mut bob], "the acceptance", |peers| {
        important(peers[0], 3).is_some()
    });
    alice.ask(request::add_squad_member("Bob"));
    let in_squad = |peer: &Peer| {
        has(peer, |message| {
            matches!(message, Decoded::SquadInfo { squad_id, members, .. }
                if *squad_id != 0 && members.len() == 2)
        })
    };
    until(&mut [&mut alice, &mut bob], "the squad", |peers| {
        in_squad(peers[0]) && in_squad(peers[1])
    });

    // The soldier readies, then the commander; Carol makes up the match.
    carol.ask(request::play(1));
    bob.ask(request::play(1));
    run_for(
        &mut [&mut alice, &mut bob, &mut carol],
        Duration::from_millis(200),
    );
    assert!(server.host.made.locked().is_empty());
    alice.ask(request::play(1));
    until(
        &mut [&mut alice, &mut bob, &mut carol],
        "the match",
        |peers| {
            peers
                .iter()
                .all(|peer| sent_to_match(&peer.client.decoded).is_some())
        },
    );
    let alice_seat = sent_to_match(&alice.client.decoded).unwrap();
    let bob_seat = sent_to_match(&bob.client.decoded).unwrap();
    let carol_seat = sent_to_match(&carol.client.decoded).unwrap();
    assert_eq!(alice_seat, bob_seat);
    assert_eq!(carol_seat.0, alice_seat.0);
    assert_ne!(carol_seat.1, alice_seat.1);
    let made = server.host.made.locked().clone();
    let squad_of = |account_id: u32| {
        made[0]
            .1
            .players
            .iter()
            .find(|player| player.account_id == account_id)
            .unwrap()
            .profile
            .squad
    };
    assert_ne!(squad_of(alice.account_id), 0);
    assert_eq!(squad_of(alice.account_id), squad_of(bob.account_id));
    assert_eq!(squad_of(carol.account_id), 0);
    assert_clean(&[&alice, &bob, &carol]);
}

#[test]
fn a_link_that_goes_silent_frees_its_session() {
    let server = Server::start(Duration::ZERO, Duration::from_millis(1000));
    let mut alice = Peer::new(&server, "Alice");
    let mut bob = Peer::new(&server, "Bob");
    connect(&mut alice);
    connect(&mut bob);
    assert_eq!(server.store.players_online(), 2);
    // Alice's game is gone without a word; Bob's goes on.
    let alice_session = alice.session_id;
    drop(alice);
    let store = server.store.clone();
    until(&mut [&mut bob], "the session's end", |_| {
        store.has_ended(alice_session)
    });
    assert!(server.store.hold(alice_session).is_none());
    assert!(!server.store.has_ended(bob.session_id));
    until(&mut [&mut bob], "the online count", |peers| {
        peers[0]
            .client
            .online
            .last()
            .is_some_and(|counts| counts.0 == 1)
    });
    assert_clean(&[&bob]);
}

#[test]
fn hundreds_of_ordered_pushes_survive_a_lossy_udp_link() {
    let server = Server::quick();
    let mut alice = Peer::new(&server, "Alice");
    connect(&mut alice);
    alice.take();
    alice.client.loss_percent = 25;
    let pushes: Vec<_> = (0..320)
        .map(|index| survarium::v026g::lobby::money(index, 0))
        .collect();
    server.inbox.push(alice.account_id, pushes);
    let amounts = |peer: &Peer| -> Vec<u32> {
        peer.client
            .decoded
            .iter()
            .filter_map(|message| match message {
                Decoded::Money {
                    generic,
                    premium: 0,
                } => Some(*generic),
                _ => None,
            })
            .collect()
    };
    until(&mut [&mut alice], "320 ordered pushes", |peers| {
        amounts(peers[0]).len() == 320
    });
    assert_eq!(amounts(&alice), (0..320).collect::<Vec<u32>>());
    assert!(alice.client.dropped > 0);
    assert_clean(&[&alice]);
}
