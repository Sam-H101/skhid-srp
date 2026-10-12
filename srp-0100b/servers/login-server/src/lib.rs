use openssl::ssl::{Ssl, SslContext, SslFiletype, SslMethod, SslStream};
use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use session::{SESSION_IDLE_LIMIT, SessionStore};
use vostok::config;

const PKEY_PATH: &str = "./certs/survarium_login_server.key";
const CERT_PATH: &str = "./certs/survarium_login_server.crt";

/// If no prefix is provided the client will construct an incorrect URI like so:
///     "127.0.0.1&type=2&.."
/// Notice that there is no `/` and that the first arguments starts with `&` instead of `?`.
/// As a temporary (?) fix provide an unused argument to the client, since it accepts url prefix
/// as a part of the message.
const URL_PREFIX: &[u8] = b"/hello?unused=1";

#[repr(u8)]
#[rustfmt::skip]
#[expect(non_camel_case_types)]
#[expect(dead_code)]
enum login_server_message_types_enum {
    servers_connection_info_message_type           = 0x8,  // + | logged in | client doesn't start required servers
    password_request_message_type                  = 0x9,  // connection error
    invalid_user_name_or_password_message_type     = 0xA,  // +
    valid_user_name_message_type                   = 0xB,  // + | establishes ssh connection
    sign_in_attempt_interval_violated_message_type = 0xC,  // +
    sign_out_successful                            = 0xD,  // connection error
    occupied_user_name_message_type                = 0xE,  // connection error
    send_sign_up_info_message_type                 = 0xF,  // connection error
    sign_up_successful_message_type                = 0x10, // connection error
    user_banned_message_type                       = 0x11, // +
    user_restricted_by_access_level_message_type   = 0x12, // +
    sign_in_user_already_signed_in                 = 0x13, // + | got stuck on connecting
    sign_in_invalid_version                        = 0x14, // +
    login_server_invalid_message_type              = 0x1F, // connection error
}

#[repr(u8)]
#[rustfmt::skip]
#[expect(non_camel_case_types)]
#[expect(dead_code)]
enum login_client_message_types_enum {
    sign_up_message_type              = 0x0, // not used by the client
    sign_in_message_type              = 0x1,
    sign_out_message_type             = 0x2,
    login_client_invalid_message_type = 0x7, // not used by the client
}

/// How often sessions unseen for [`SESSION_IDLE_LIMIT`] are ended: the
/// original login server's `collect_interval`
/// (`login_server/sources/entry_point.cpp`).
const COLLECT_INTERVAL: Duration = Duration::from_secs(60);

/// @NOTE: Real solution should:
/// * Accept the config for addresses of other servers
/// * Use `async` instead of threads
/// * Have a database for different users and their session ids
/// * ...many more things
pub fn run(store: Arc<SessionStore>) -> std::io::Result<()> {
    // Once, so a missing or bad certificate stops the server here rather
    // than failing every sign-in.
    let tls = Arc::new(tls_context().map_err(|error| {
        io::Error::other(format!(
            "cannot load {CERT_PATH} and {PKEY_PATH} (run srp from the repo root): {error}"
        ))
    })?);
    let listener = TcpListener::bind(config::get().login_server.bind_addr())?;

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            // A failed `accept` shouldn't take down the whole server.
            Err(error) => {
                log::error!("Failed to accept connection: {error}");
                continue;
            }
        };
        let store = store.clone();
        let tls = tls.clone();
        std::thread::spawn(move || {
            if std::panic::catch_unwind(|| handle_client(store, &tls, stream)).is_err() {
                log::error!("Connection handler panicked; dropped the connection");
            }
        });
    }
    Ok(())
}

/// Hear presence pings, and end sessions unseen for too long.
///
/// Once signed in, the client pings the login server's IP on UDP port 25100
/// (fixed: `_itoa_s(0x620C)` at 0x7A236C) every second: a datagram of its
/// session id alone, 4 bytes little-endian (`login_client_impl::ping`,
/// 0x79847A). It never waits for an answer; after 10 failed sends in a row
/// it stops (0x798459). The original login server kept the time of each
/// session's last ping and every minute signed out those unseen for ten
/// (`login_server.cpp`, `query_processor.cpp`); so does this. Lobby, chat
/// and match connections count as sightings too (see `SessionStore`), so a
/// client without pings (a script) keeps its session while it plays.
pub fn run_presence(store: Arc<SessionStore>) -> io::Result<()> {
    let address = config::get().login_pings.bind_addr();
    let mut socket = None;
    let mut unknown = HashSet::new();
    let mut next_collect = Instant::now() + COLLECT_INTERVAL;
    loop {
        if socket.is_none() {
            socket = match UdpSocket::bind(&address) {
                Ok(bound) => {
                    log::info!("Hearing presence pings on UDP {address}");
                    Some(bound)
                }
                Err(error) => {
                    log::error!(
                        "Cannot hear presence pings on UDP {address} ({error}); \
                         retrying in {} s",
                        COLLECT_INTERVAL.as_secs()
                    );
                    None
                }
            };
        }
        let wait = next_collect.saturating_duration_since(Instant::now());
        match &socket {
            Some(socket) if !wait.is_zero() => {
                socket.set_read_timeout(Some(wait))?;
                let mut buffer = [0; 16];
                match socket.recv_from(&mut buffer) {
                    Ok((len, from)) => hear_ping(&store, &buffer[..len], from, &mut unknown),
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) => {}
                    // On Windows a datagram socket also reports other hosts'
                    // errors (an oversized datagram, a reset); none ends it.
                    Err(error) => log::debug!("Presence ping unreadable: {error}"),
                }
            }
            Some(_) => {}
            None => std::thread::sleep(wait),
        }
        if Instant::now() >= next_collect {
            next_collect = Instant::now() + COLLECT_INTERVAL;
            let ended = store.end_idle_sessions(Instant::now(), SESSION_IDLE_LIMIT);
            if !ended.is_empty() {
                log::info!(
                    "Ended {} session(s) unseen for {} s: {ended:#x?}",
                    ended.len(),
                    SESSION_IDLE_LIMIT.as_secs()
                );
            }
        }
    }
}

/// One presence ping: a session id, 4 bytes. A ping for an unknown or ended
/// session is logged once per session (the client keeps pinging).
fn hear_ping(store: &SessionStore, datagram: &[u8], from: SocketAddr, unknown: &mut HashSet<u32>) {
    let Ok(session_id) = <[u8; 4]>::try_from(datagram).map(u32::from_le_bytes) else {
        log::debug!(
            "Ignored a {}-byte presence datagram from {from}",
            datagram.len()
        );
        return;
    };
    if store.touch(session_id) {
        log::trace!("Presence ping of session {session_id:#x} from {from}");
        return;
    }
    if unknown.len() >= 1024 {
        unknown.clear();
    }
    if unknown.insert(session_id) {
        log::debug!("Presence ping from {from} for unknown or ended session {session_id:#x}");
    }
}

fn handle_client(store: Arc<SessionStore>, tls: &SslContext, mut stream: TcpStream) {
    let mut request_type = [0_u8];
    if let Err(error) = stream.read_exact(&mut request_type) {
        log::info!("Login connection closed before its request: {error}");
        return;
    }

    let request_type = request_type[0];
    log::debug!("Received message of type: {request_type}");

    match () {
        _ if request_type == login_client_message_types_enum::sign_in_message_type as u8 => {
            handle_sign_in(store, tls, stream);
        }
        _ if request_type == login_client_message_types_enum::sign_out_message_type as u8 => {
            handle_sign_out(&store, tls, stream);
        }
        _ => log::warn!("Unknown login request type {request_type}; dropped the connection"),
    }
}

/// Read until `buffer` holds a whole message, as `complete` measures it from
/// what has arrived (`None`: can't tell yet), and return its length. Whatever
/// else came in the same reads is read too and ignored.
fn read_message(
    stream: &mut impl Read,
    buffer: &mut [u8],
    complete: impl Fn(&[u8]) -> Option<usize>,
) -> io::Result<usize> {
    let mut filled = 0;
    loop {
        if let Some(len) = complete(&buffer[..filled])
            && len <= filled
        {
            return Ok(len);
        }
        if filled == buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "message too long",
            ));
        }
        match stream.read(&mut buffer[filled..])? {
            0 => return Err(io::ErrorKind::UnexpectedEof.into()),
            read => filled += read,
        }
    }
}

/// A sign-in: the email (u8 length) and the 6-byte game version.
fn sign_in_length(buffer: &[u8]) -> Option<usize> {
    Some(1 + usize::from(*buffer.first()?) + 6)
}

/// The password, behind its u8 length.
fn password_length(buffer: &[u8]) -> Option<usize> {
    Some(1 + usize::from(*buffer.first()?))
}

fn handle_sign_in(store: Arc<SessionStore>, tls: &SslContext, mut stream: TcpStream) {
    let mut buffer = [0; 1024];
    let peer = match stream.peer_addr() {
        Ok(peer) => peer,
        Err(error) => {
            log::info!("Sign-in dropped: {error}");
            return;
        }
    };
    // Where the client finds the browser server: see `ServerConfig::host_for`.
    let config = config::get();
    let browser_host = match stream.local_addr() {
        Ok(reached) => {
            config
                .browser_server
                .host_for(&config.login_server, peer.ip(), reached.ip())
        }
        Err(_) => config.browser_server.public_host.clone(),
    };
    let len = match read_message(&mut stream, &mut buffer, sign_in_length) {
        Ok(len) => len,
        Err(error) => {
            log::warn!("Unreadable sign-in from {peer}: {error}");
            return;
        }
    };
    let email_len = usize::from(buffer[0]);
    let email = String::from_utf8_lossy(&buffer[1..1 + email_len]).into_owned();
    let game_version = String::from_utf8_lossy(&buffer[1 + email_len..len]).into_owned();

    log::info!("User email: {email}");
    log::info!("Client version: {game_version}");

    // Resolve (or create) the account for this client and open a session. The
    // account is keyed on the client IP + email; an empty email is accepted.
    let account_id = store.get_or_create_account(peer.ip(), &email);
    let session_id = store.create_session(account_id);
    log::info!("Sign-in: account={account_id:#x} session={session_id:#x}");

    let message_type = login_server_message_types_enum::valid_user_name_message_type as u8;
    if let Err(error) = stream.write_all(&[message_type]) {
        log::warn!("Sign-in for session {session_id:#x} dropped: {error}");
        return;
    }

    let Some(mut ssl_stream) = accept_tls(tls, stream) else {
        return;
    };
    // The password is not checked, and never logged: the client sends the
    // one its player typed.
    let mut buffer = [0; 1024];
    if let Err(error) = read_message(&mut ssl_stream, &mut buffer, password_length) {
        log::warn!("Sign-in for session {session_id:#x} dropped before the password: {error}");
        return;
    }

    let mut buffer = vec![];
    buffer.push(login_server_message_types_enum::servers_connection_info_message_type as u8);

    // Host only: the client talks HTTP to the browser server on the
    // implicit port 80 and appends `URL_PREFIX` itself.
    log::debug!("Session {session_id:#x} goes to the browser server at {browser_host}");
    buffer.push(browser_host.len() as u8);
    buffer.extend(browser_host.as_bytes());
    buffer.push(URL_PREFIX.len() as u8);
    buffer.extend(URL_PREFIX);

    buffer.extend(session_id.to_le_bytes());

    if let Err(error) = ssl_stream.write_all(&buffer) {
        log::warn!("Sign-in for session {session_id:#x} dropped before the reply: {error}");
    }
}

/// A sign-out: the session id in the clear, then the password over TLS, and
/// no answer (`login_client_impl::sign_out`, 0x7A1CDC). The client sends one
/// when it quits, and when it gives up reconnecting to the lobby. Once the
/// password has come, the session ends, as the original login server signed
/// it out (`client_session_sign_out.cpp`); the password isn't checked.
fn handle_sign_out(store: &SessionStore, tls: &SslContext, mut stream: TcpStream) {
    // Exactly the id: the client starts its TLS handshake right behind it,
    // with no answer to wait for, so a longer read would take the start of
    // the handshake too.
    let mut session_id = [0; 4];
    let session_id = match stream.read_exact(&mut session_id) {
        Ok(()) => u32::from_le_bytes(session_id),
        Err(error) => {
            log::info!("Unreadable sign-out: {error}");
            return;
        }
    };
    log::debug!("Sign-out of session {session_id:#x}");

    let Some(mut ssl_stream) = accept_tls(tls, stream) else {
        return;
    };
    let mut buffer = [0; 1024];
    if let Err(error) = read_message(&mut ssl_stream, &mut buffer, password_length) {
        log::info!("Sign-out of session {session_id:#x} dropped before the password: {error}");
        return;
    }
    if store.end_session(session_id) {
        log::info!("Session {session_id:#x} signed out");
    } else {
        log::info!("Sign-out of unknown or ended session {session_id:#x}");
    }
}

/// The login server's TLS setup, with the game's own key and certificate.
fn tls_context() -> Result<SslContext, openssl::error::ErrorStack> {
    let mut builder = SslContext::builder(SslMethod::tls())?;
    builder.set_security_level(0);
    builder.set_private_key_file(PKEY_PATH, SslFiletype::PEM)?;
    builder.set_certificate_chain_file(CERT_PATH)?;
    Ok(builder.build())
}

/// Upgrade a login connection to TLS. `None` when the handshake fails, which is
/// how a client that rejects the certificate shows up here: it just closes the
/// connection.
fn accept_tls(tls: &SslContext, stream: TcpStream) -> Option<SslStream<TcpStream>> {
    let peer = stream.peer_addr().ok();
    let ssl = match Ssl::new(tls) {
        Ok(ssl) => ssl,
        Err(error) => {
            log::error!("TLS setup for {peer:?} failed: {error}");
            return None;
        }
    };

    match ssl.accept(stream) {
        Ok(ssl_stream) => Some(ssl_stream),
        Err(error) => {
            // The client checks the certificate against its own
            // `resources/ssl/survarium_login_server.crt` on its first attempt
            // only. The game's original certificate expired in 2014, so that
            // file has to be replaced with `CERT_PATH` (see WORK.md).
            log::warn!(
                "TLS handshake with {peer:?} failed ({error}); if the client logs \
                 \"certificate verify failed\", copy {CERT_PATH} over its \
                 resources/ssl/survarium_login_server.crt"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hands out its bytes a few at a time, as a socket may.
    struct Trickle<'a>(&'a [u8]);

    impl Read for Trickle<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let n = self.0.len().min(buffer.len()).min(3);
            buffer[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    #[test]
    fn a_sign_in_is_read_whole_however_it_arrives() {
        let mut sign_in = vec![5];
        sign_in.extend(b"a@b.c0.100b");
        let mut buffer = [0; 64];
        let len = read_message(&mut Trickle(&sign_in), &mut buffer, sign_in_length).unwrap();
        assert_eq!(&buffer[..len], sign_in.as_slice());
    }

    #[test]
    fn a_presence_ping_keeps_its_session_and_nothing_else_does() {
        let store = SessionStore::new();
        let account = store.get_or_create_account([127, 0, 0, 1].into(), "pinger");
        let session = store.create_session(account);
        let from = SocketAddr::from(([127, 0, 0, 1], 50000));
        let mut unknown = HashSet::new();

        std::thread::sleep(Duration::from_millis(60));
        hear_ping(&store, &session.to_le_bytes(), from, &mut unknown);
        let limit = Duration::from_millis(50);
        assert!(store.end_idle_sessions(Instant::now(), limit).is_empty());

        // Too long, too short, someone else's: no sighting.
        std::thread::sleep(Duration::from_millis(60));
        let mut long = session.to_le_bytes().to_vec();
        long.push(0);
        hear_ping(&store, &long, from, &mut unknown);
        hear_ping(&store, &session.to_le_bytes()[..3], from, &mut unknown);
        hear_ping(&store, &0xBAD_u32.to_le_bytes(), from, &mut unknown);
        assert_eq!(unknown, HashSet::from([0xBAD]));
        assert_eq!(store.end_idle_sessions(Instant::now(), limit), [session]);
    }

    #[test]
    fn a_cut_or_oversized_message_is_an_error_not_a_panic() {
        let mut buffer = [0; 64];
        assert!(read_message(&mut Trickle(&[200, 1, 2]), &mut buffer, sign_in_length).is_err());
        let mut small = [0; 4];
        assert!(read_message(&mut Trickle(&[9; 16]), &mut small, password_length).is_err());
    }
}
