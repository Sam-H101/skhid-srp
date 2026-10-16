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
    base(text, false)
}

/// `parse`; `any_http`: plain HTTP to any host too.
fn base(text: &str, any_http: bool) -> Option<Url> {
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
    if !secure && !any_http && !(host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1") {
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

/// An absolute download URL, including a query (Drive's direct download uses one).
/// It cannot carry credentials, fragments, controls, or a non-HTTPS remote host.
pub fn download(text: &str) -> Option<Url> {
    if text.len() > 4096
        || text
            .bytes()
            .any(|c| !c.is_ascii_graphic() || matches!(c, b'\\' | b'#'))
    {
        return None;
    }
    let scheme_end = text.find("://")? + 3;
    let at = text[scheme_end..]
        .find(['/', '?'])
        .map(|at| scheme_end + at)
        .unwrap_or(text.len());
    let mut url = parse(&text[..at])?;
    let path = &text[at..];
    url.prefix = if path.starts_with('?') {
        alloc::format!("/{path}")
    } else {
        path.into()
    };
    Some(url)
}

/// The address of a patch file of `resources.db` (`patches`): `http://` or
/// `https://`, any host, a bare IP address too, and a path. Plain HTTP is
/// taken here: the file is checked against the SHA-256 of the list, which
/// comes from the update server over its own address. No credentials, no
/// fragment, no space or control character.
pub fn patch(text: &str) -> Option<Url> {
    if text.len() > 1024
        || text
            .bytes()
            .any(|c| !c.is_ascii_graphic() || matches!(c, b'\\' | b'#'))
    {
        return None;
    }
    let scheme_end = text.find("://")? + 3;
    let at = scheme_end + text[scheme_end..].find('/')?;
    let path = &text[at..];
    if path.len() < 2 {
        return None;
    }
    let mut url = base(&text[..at], true)?;
    url.prefix = path.into();
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_may_be_on_any_host_over_http_or_https() {
        let u = patch("http://203.0.113.5/patch_14").unwrap();
        assert_eq!(
            (u.secure, u.host.as_str(), u.port, u.prefix.as_str()),
            (false, "203.0.113.5", 80, "/patch_14")
        );
        let u = patch("https://Files.Example.org:8443/db/patch_14?v=2").unwrap();
        assert_eq!(
            (u.secure, u.host.as_str(), u.port, u.prefix.as_str()),
            (true, "files.example.org", 8443, "/db/patch_14?v=2")
        );
        assert!(patch("http://127.0.0.1:1234/p/patch_1").is_some());
        for bad in [
            "",
            "/patch_14",
            "patch_14",
            "203.0.113.5/patch_14",
            "ftp://203.0.113.5/patch_14",
            "file:///C:/patch_14",
            "file://host/patch_14",
            "javascript://x/patch_14",
            "HTTP://host/patch_14",
            "http://203.0.113.5",
            "http://203.0.113.5/",
            "http://user@host/patch_14",
            "http://user:pw@host/patch_14",
            "http://host/patch_14#x",
            "http://host/a b",
            "http://host/a\\b",
            "http://host/a\r\nX: y",
            "http://host:0/patch_14",
            "http://host:99999/patch_14",
            "http://[::1]/patch_14",
            "http:///patch_14",
            " http://host/patch_14",
        ] {
            assert!(patch(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn download_queries_preserve_the_exact_request() {
        let u = download(
            "https://drive.usercontent.google.com/download?id=abc&export=download&confirm=t",
        )
        .unwrap();
        assert_eq!(u.host, "drive.usercontent.google.com");
        assert_eq!(u.prefix, "/download?id=abc&export=download&confirm=t");
        assert_eq!(download("https://host?x=1").unwrap().prefix, "/?x=1");
        for bad in [
            "http://example.com/file",
            "https://user@host/file",
            "https://host/a#b",
            "https://host/a\\b",
            "https://host/a\r\nX:bad",
            "https://host/a b",
            "ftp://host/file",
        ] {
            assert!(download(bad).is_none(), "{bad}");
        }
        assert!(download("http://127.0.0.1:1234/file?test=1").is_some());
    }

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
