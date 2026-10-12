mod lobby_server;
mod messaging_server;

use session::SessionStore;
use vostok::config;
use vostok::network_client::{NetworkError, TcpClient};
use vostok::serde::DeserializeError;

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

const _: () = {
    // `bytemuck` is used for serializing and deserializing messages.
    // And all messages are expected to use little endian.
    assert!(
        cfg!(target_endian = "little"),
        "This code only supports little-endian systems"
    );
};

/// What the lobby and the chat do with a session the store doesn't know, or
/// one that has ended (signed out, or unseen for too long).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownSessions {
    /// Refuse it, as a session the login server didn't open.
    Refuse,
    /// Serve it as a default account: a lobby run without a login server.
    UseDefaultAccount,
}

/// The lobby and chat server of srp, beside its login server.
pub fn run(store: Arc<SessionStore>) -> std::io::Result<()> {
    serve(store, UnknownSessions::Refuse)
}

/// A lobby and chat server on its own, with no login server to open
/// sessions: any session gets a default account.
pub fn run_standalone(store: Arc<SessionStore>) -> std::io::Result<()> {
    serve(store, UnknownSessions::UseDefaultAccount)
}

fn serve(store: Arc<SessionStore>, unknown: UnknownSessions) -> std::io::Result<()> {
    let state = Arc::new(lobby_server::ServerState::new_dummy());
    let chat = Arc::new(messaging_server::ChatRoom::default());
    {
        // Players come and go, and matches start and end, outside the chat.
        let (store, chat) = (store.clone(), chat.clone());
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                chat.announce_online(store.match_count());
            }
        });
    }

    let listener = TcpListener::bind(config::get().lobby_server.bind_addr())?;

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            // A failed `accept` shouldn't take down the whole server.
            Err(error) => {
                log::error!("Failed to accept connection: {error}");
                continue;
            }
        };
        let state = state.clone();
        let store = store.clone();
        let chat = chat.clone();
        std::thread::spawn(move || {
            if std::panic::catch_unwind(|| handle_connection(store, state, &chat, stream, unknown))
                .is_err()
            {
                log::error!("Connection handler panicked; dropped the connection");
            }
        });
    }

    Ok(())
}

pub fn handle_connection(
    store: Arc<SessionStore>,
    lobby_server: Arc<lobby_server::ServerState>,
    chat: &messaging_server::ChatRoom,
    stream: TcpStream,
    unknown: UnknownSessions,
) {
    // The lobby and chat ("messaging") protocols share this port; the first
    // message tells them apart. The chat writes through `stream` while
    // `tcp_client` reads from its clone.
    let reader = match stream.try_clone() {
        Ok(reader) => reader,
        Err(error) => {
            log::error!("Lobby connection unusable: {error}");
            return;
        }
    };
    let mut tcp_client = TcpClient::new(reader);
    match tcp_client.peek::<lobby_server::client::Message>() {
        Ok(_) => lobby_server.run(store, chat, tcp_client, unknown),
        Err(NetworkError::DeserializeError(DeserializeError::UnknownMessageType(_))) => {
            messaging_server::run(&store, chat, tcp_client, stream, unknown)
        }
        Err(error) if error.is_disconnect() => {
            log::info!("Lobby connection closed before its first message");
        }
        Err(error) => {
            log::error!("Unreadable first lobby message ({error}); dropped the connection");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::{Duration, Instant};

    /// The lobby and chat on a free local port.
    fn start(store: Arc<SessionStore>, unknown: UnknownSessions) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(lobby_server::ServerState::new_dummy());
        let chat = Arc::new(messaging_server::ChatRoom::default());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let (store, state, chat) = (store.clone(), state.clone(), chat.clone());
                std::thread::spawn(move || {
                    handle_connection(store, state, &chat, stream.unwrap(), unknown)
                });
            }
        });
        addr
    }

    fn connect(addr: SocketAddr, first: &[u8]) -> TcpStream {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = vec![first.len() as u8];
        bytes.extend(first);
        stream.write_all(&bytes).unwrap();
        stream
    }

    fn lobby_sign_in(addr: SocketAddr, session_id: u32) -> TcpStream {
        let mut sign_in = vec![0x26];
        sign_in.extend(session_id.to_le_bytes());
        connect(addr, &sign_in)
    }

    fn chat_sign_in(addr: SocketAddr, session_id: u32) -> TcpStream {
        let mut sign_in = vec![0xC3];
        sign_in.extend(session_id.to_le_bytes());
        sign_in.push(5);
        connect(addr, &sign_in)
    }

    /// The next frame, or `None` once the server has closed the connection.
    fn frame(stream: &mut TcpStream) -> Option<Vec<u8>> {
        let mut len = [0_u8];
        if stream.read(&mut len).unwrap() == 0 {
            return None;
        }
        let mut frame = vec![0; usize::from(len[0])];
        stream.read_exact(&mut frame).unwrap();
        Some(frame)
    }

    fn signed_in(store: &SessionStore, email: &str) -> u32 {
        let account_id = store.get_or_create_account(IpAddr::V4(Ipv4Addr::LOCALHOST), email);
        store.create_session(account_id)
    }

    #[test]
    fn an_unknown_or_ended_session_is_refused_before_sign_in() {
        let store = Arc::new(SessionStore::new());
        let addr = start(store.clone(), UnknownSessions::Refuse);

        let mut lobby = lobby_sign_in(addr, 0xBAD);
        assert_eq!(frame(&mut lobby), Some(vec![0x31]));
        assert_eq!(frame(&mut lobby), None);
        let mut chat = chat_sign_in(addr, 0xBAD);
        assert_eq!(frame(&mut chat), None);

        let session = signed_in(&store, "player");
        let mut lobby = lobby_sign_in(addr, session);
        assert_eq!(frame(&mut lobby), Some(vec![0x30]));
        let mut chat = chat_sign_in(addr, session);
        assert_eq!(frame(&mut chat), Some(b"\xCB\x06player".to_vec()));

        // Signed out: the open connections go on, new ones are refused.
        assert!(store.end_session(session));
        let mut again = lobby_sign_in(addr, session);
        assert_eq!(frame(&mut again), Some(vec![0x31]));
        assert_eq!(frame(&mut again), None);
        let mut chat_again = chat_sign_in(addr, session);
        assert_eq!(frame(&mut chat_again), None);
    }

    #[test]
    fn a_standalone_lobby_serves_an_unknown_session() {
        let addr = start(
            Arc::new(SessionStore::new()),
            UnknownSessions::UseDefaultAccount,
        );
        let mut lobby = lobby_sign_in(addr, 0xBAD);
        assert_eq!(frame(&mut lobby), Some(vec![0x30]));
        let mut chat = chat_sign_in(addr, 0xBAD);
        assert_eq!(frame(&mut chat), Some(b"\xCB\x0Adummy_name".to_vec()));
    }

    #[test]
    fn an_open_connection_keeps_its_session_from_idling_out() {
        let store = Arc::new(SessionStore::new());
        let addr = start(store.clone(), UnknownSessions::Refuse);
        let in_lobby = signed_in(&store, "lobby");
        let in_chat = signed_in(&store, "chat");
        let mut lobby = lobby_sign_in(addr, in_lobby);
        assert_eq!(frame(&mut lobby), Some(vec![0x30]));
        let mut chat = chat_sign_in(addr, in_chat);
        assert!(frame(&mut chat).is_some());

        let later = Instant::now() + Duration::from_secs(3600);
        assert!(
            store
                .end_idle_sessions(later, session::SESSION_IDLE_LIMIT)
                .is_empty()
        );

        // Its end is the last sighting.
        drop(lobby);
        let deadline = Instant::now() + Duration::from_secs(2);
        while store.end_idle_sessions(later, session::SESSION_IDLE_LIMIT) != [in_lobby] {
            assert!(
                Instant::now() < deadline,
                "the lobby never let its session go"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(store.account_for_session(in_chat).is_some());
    }
}
