//! The address of the server the updater asks: `https://host[:port][/path]`.
//!
//! The files it brings are programs, and the only thing that says they are
//! the server's own is the HTTPS connection. So plain `http://` is taken
//! only for this computer itself (a server under test), never for a host on
//! a network.

use alloc::string::String;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Url {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    /// What every request path starts with: empty, or `/some/path` (no `/`
    /// at its end).
    pub prefix: String,
}

/// Reads a base address. `None` for anything else than the forms above.
pub fn parse(text: &str) -> Option<Url> {
    let text = text.trim();
    let (secure, rest) = if let Some(rest) = text.strip_prefix("https://") {
        (true, rest)
    } else {
        (false, text.strip_prefix("http://")?)
    };
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => {
            if port.is_empty() || port.len() > 5 || !port.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let port: u32 = port.parse().ok()?;
            if port == 0 || port > 65535 {
                return None;
            }
            (host, port as u16)
        }
        None => (authority, if secure { 443 } else { 80 }),
    };
    if host.is_empty()
        || host.len() > 253
        || host.starts_with(['.', '-'])
        || host.ends_with(['.', '-'])
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
    {
        return None;
    }
    if !secure && !(host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1") {
        return None;
    }
    let prefix = path.trim_end_matches('/');
    if prefix.len() > 200
        || !prefix
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'.' | b'_' | b'-' | b'~'))
    {
        return None;
    }
    Some(Url {
        secure,
        host: host.to_ascii_lowercase(),
        port,
        prefix: prefix.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_forms_that_are_taken() {
        let url = parse("https://skhid.link").unwrap();
        assert_eq!(
            (url.secure, url.host.as_str(), url.port, url.prefix.as_str()),
            (true, "skhid.link", 443, "")
        );
        let url = parse(" https://Test.Skhid.Link:8443/srp/ \r\n").unwrap();
        assert_eq!(
            (url.host.as_str(), url.port, url.prefix.as_str()),
            ("test.skhid.link", 8443, "/srp")
        );
        let url = parse("http://127.0.0.1:8088").unwrap();
        assert_eq!(
            (url.secure, url.host.as_str(), url.port),
            (false, "127.0.0.1", 8088)
        );
        assert_eq!(parse("http://localhost").unwrap().port, 80);
    }

    #[test]
    fn everything_else_is_refused() {
        for bad in [
            "",
            "skhid.link",
            "ftp://skhid.link",
            "https://",
            "https:///x",
            "https://a b",
            "https://user@skhid.link",
            "https://skhid.link:0",
            "https://skhid.link:65536",
            "https://skhid.link:",
            "https://skhid.link:80a",
            "https://-a.b",
            "https://a.b.",
            "https://skhid.link/?x=1",
            "https://skhid.link/a b",
            "https://skhid.link/#x",
            // Plain HTTP to a host on a network: the files would be anybody's.
            "http://skhid.link",
            "http://192.168.1.10:8088",
            "http://127.0.0.2",
            "http://localhost.evil.example",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }
}
