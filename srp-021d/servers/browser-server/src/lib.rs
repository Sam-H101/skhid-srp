//! The browser server: tells the client where its lobby is.
//!
//! The client asks `GET /hello?...` over HTTP/1.0 and reads **one** response.
//! Its HTTP client keeps one input buffer for all its requests, so anything
//! after the first response (a second answer to a pipelined request, which
//! it sends when an earlier request found the server down) stays in that
//! buffer; its next status line is then garbage, which it neither reports nor
//! recovers from, and it never finds the lobby again until it is restarted.
//! So this answers the first request of a connection only and closes.
//!
//! The game asks nothing but `/hello`. Any other request is a person with a
//! browser: with `SRP_WEB_DOMAIN` set it is sent on to the web page's
//! `https://` address, otherwise it gets a 404.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use session::SessionStore;
use vostok::config;

/// A request head is a few hundred bytes.
const MAX_HEAD: usize = 8192;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// How long unread bytes of a closed connection are drained, so closing it
/// does not reset it and the answer is lost.
const DRAIN: Duration = Duration::from_millis(500);

/// Where a browser is sent: `https://` and the web page's domain. The
/// domain is the operator's, never taken from the request.
fn web_domain() -> Option<String> {
    let domain = std::env::var("SRP_WEB_DOMAIN").ok()?;
    let domain = domain.trim();
    let valid = !domain.is_empty()
        && domain
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-:[]".contains(&byte));
    valid.then(|| domain.to_owned())
}

/// The redirect for `target` (the request's path and query) to `domain`, or
/// `None` if the target is not something to carry over.
fn redirect(domain: &str, target: &str) -> Option<String> {
    let plain = target.bytes().all(|byte| (0x21..0x7f).contains(&byte));
    (plain && target.starts_with('/') && !target.starts_with("//"))
        .then(|| format!("https://{domain}{target}"))
}

/// Run the browser server forever, a thread per connection. The session
/// store is unused: the browser server only hands the client the lobby
/// address.
pub fn run(_store: Arc<SessionStore>) -> io::Result<()> {
    let listener = TcpListener::bind(config::get().browser_server.bind_addr())?;
    log::info!("Browser server on {}", listener.local_addr()?);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let spawned = thread::Builder::new()
                    .name("browser-connection".into())
                    .spawn(move || serve(stream));
                if let Err(error) = spawned {
                    log::warn!("Can't serve a connection: {error}");
                }
            }
            Err(error) => {
                log::warn!("Accept failed: {error}");
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Ok(())
}

fn serve(mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(READ_TIMEOUT));
    let (Ok(peer), Ok(reached)) = (stream.peer_addr(), stream.local_addr()) else {
        return;
    };
    let Some(request_line) = read_request_line(&mut stream) else {
        return;
    };
    let target = request_line.split_whitespace().nth(1).unwrap_or("");
    let path = target.split('?').next().unwrap_or("");
    let is_hello = request_line.starts_with("GET ") && path == "/hello";
    log::debug!("Received: {request_line:?} from {peer}");

    let is_browser = matches!(request_line.split(' ').next(), Some("GET" | "HEAD"));
    let location = (!is_hello && is_browser)
        .then(web_domain)
        .flatten()
        .and_then(|domain| redirect(&domain, target));

    let (status, body) = if is_hello {
        // Where the client finds the lobby: see `ServerConfig::host_for`.
        let config = config::get();
        let lobby = &config.lobby_server;
        let host = lobby.host_for(&config.browser_server, peer.ip(), reached.ip());
        ("200 OK", format!("{host}:{}", lobby.port))
    } else if location.is_some() {
        ("308 Permanent Redirect", String::new())
    } else {
        ("404 Not Found", String::new())
    };
    log::info!("{peer} {request_line:?} {}", &status[..3]);
    let location = location
        .map(|to| format!("Location: {to}\r\n"))
        .unwrap_or_default();
    let response = format!(
        "HTTP/1.0 {status}\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    if stream.write_all(response.as_bytes()).is_err() || stream.flush().is_err() {
        return;
    }
    let _ = stream.shutdown(Shutdown::Write);
    // Whatever else the client sent (more requests) is not answered.
    let _ = stream.set_read_timeout(Some(DRAIN));
    let mut sink = [0_u8; 1024];
    while matches!(stream.read(&mut sink), Ok(n) if n > 0) {}
}

/// The first line of the request, once its head has arrived whole.
fn read_request_line(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        if head.len() > MAX_HEAD {
            return None;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => head.extend_from_slice(&chunk[..n]),
        }
    }
    let line = head.split(|&byte| byte == b'\r').next()?;
    Some(String::from_utf8_lossy(line).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn ask(request: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || serve(listener.accept().unwrap().0));
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(request.as_bytes()).unwrap();
        let mut answer = String::new();
        client.read_to_string(&mut answer).unwrap();
        server.join().unwrap();
        answer
    }

    const HELLO: &str = "GET /hello?unused=1&type=2 HTTP/1.0\r\nHost: x\r\n\r\n";

    #[test]
    fn a_hello_gets_the_lobby_address() {
        let answer = ask(HELLO);
        let port = config::get().lobby_server.port;
        assert!(answer.starts_with("HTTP/1.0 200 OK\r\n"), "{answer}");
        assert!(
            answer.ends_with(&format!("\r\n\r\n127.0.0.1:{port}")),
            "{answer}"
        );
    }

    #[test]
    fn a_second_pipelined_request_is_not_answered() {
        let answer = ask(&format!("{HELLO}{HELLO}"));
        assert_eq!(answer.matches("HTTP/1.0").count(), 1, "{answer}");
    }

    #[test]
    fn a_browser_is_sent_to_https_and_the_game_is_not() {
        assert_eq!(
            redirect("skhid.link", "/download?x=1").as_deref(),
            Some("https://skhid.link/download?x=1")
        );
        // Nothing that could point somewhere else or split the header.
        assert_eq!(redirect("skhid.link", "//evil.example/"), None);
        assert_eq!(redirect("skhid.link", "evil.example"), None);
        assert_eq!(redirect("skhid.link", "/a\u{7f}b"), None);
    }

    #[test]
    fn anything_else_is_not_found() {
        assert!(ask("GET /other HTTP/1.0\r\n\r\n").starts_with("HTTP/1.0 404"));
    }
}
