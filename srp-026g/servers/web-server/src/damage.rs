//! The damage calculator, `/damage` on the 0.26g site: one page and the files
//! it loads, built by `srp-026g/scripts/damage-calc/build_page.py --site` into
//! `../damage/` from the game data of the latest content patch (the script's
//! README says how). Every file is part of the binary. The page names each
//! with `?v=<its hash>`, so they are sent to be kept for good.

use std::io::Write;
use std::net::{Shutdown, TcpStream};

use crate::{ContentType, Lang, Response};

/// The page's address.
pub(crate) const PATH: &str = "/damage";

/// The page's Content-Security-Policy: the site's, with its own script,
/// images (its files and the icons inside the script) and fonts.
pub(crate) const POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; \
    script-src 'self'; img-src 'self' data:; font-src 'self'; form-action 'self'";

/// Where the fonts are, which every page of the site uses (both servers).
pub(crate) const FONTS: &str = "/damage/fonts/";

/// The page's `@font-face` rules (the fonts' addresses with their hashes),
/// for the site's other pages.
pub(crate) fn font_faces() -> &'static str {
    static FACES: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FACES.get_or_init(|| {
        PAGE.lines()
            .filter(|line| line.starts_with("@font-face"))
            .collect()
    })
}

/// The page, with `{{LANG}}` and `<!--SITE-HEADER-->` to fill.
const PAGE: &str = include_str!("../damage/damage.html");

macro_rules! files {
    ($($path:literal => $content_type:literal,)*) => {
        &[$((
            concat!("/damage/", $path),
            $content_type,
            include_bytes!(concat!("../damage/", $path)) as &[u8],
        ),)*]
    };
}

/// The files the page loads: address, content type, bytes.
const FILES: &[(&str, &str, &[u8])] = files! {
    "calc.js" => "text/javascript; charset=utf-8",
    "fonts/carlito-bold.woff2" => "font/woff2",
    "fonts/pt-mono-regular.woff2" => "font/woff2",
    "fonts/pt-sans-bold.woff2" => "font/woff2",
    "fonts/pt-sans-italic.woff2" => "font/woff2",
    "fonts/pt-sans-narrow-bold.woff2" => "font/woff2",
    "fonts/pt-sans-narrow-regular.woff2" => "font/woff2",
    "fonts/pt-sans-regular.woff2" => "font/woff2",
    "maps/level_01.webp" => "image/webp",
    "maps/level_02.webp" => "image/webp",
    "maps/level_03.webp" => "image/webp",
    "maps/level_04.webp" => "image/webp",
    "maps/level_05.webp" => "image/webp",
    "maps/level_06.webp" => "image/webp",
    "maps/level_07.webp" => "image/webp",
    "maps/level_08.webp" => "image/webp",
    "maps/v021d_level_03.webp" => "image/webp",
    "maps/v021d_level_06.webp" => "image/webp",
};

/// The page's file at `path` (the target before any `?`): its content type
/// and bytes.
pub(crate) fn file(path: &str) -> Option<(&'static str, &'static [u8])> {
    FILES
        .iter()
        .find(|(address, _, _)| *address == path)
        .map(|&(_, content_type, bytes)| (content_type, bytes))
}

/// Sends one of the page's files, to be kept: its address changes with it.
pub(crate) fn send(stream: &mut TcpStream, head_only: bool, content_type: &str, bytes: &[u8]) {
    let head = format!(
        "HTTP/1.0 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: public, max-age=31536000, immutable\r\n\
         X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    let _ = stream.write_all(head.as_bytes());
    if !head_only {
        let _ = stream.write_all(bytes);
    }
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}

/// The page in `lang`, under `header` (the site's).
pub(crate) fn page(lang: Lang, header: &str) -> Response {
    Response {
        status: "200 OK",
        content_type: ContentType::Html,
        body: PAGE
            .replacen("{{LANG}}", lang.code(), 1)
            .replacen("<!--SITE-HEADER-->", header, 1),
    }
}

/// The site's header as the calculator shows it: the site stylesheet's rules
/// for it (`crate::STYLE`), under `.site`, so neither page's rules reach the
/// other's. The calculator's own `.tag` is undone for the header's line.
pub(crate) const HEADER_STYLE: &str = ".site{font:16px/1.55 system-ui,'Segoe UI',sans-serif;color:#e3e3e0;\
margin:-20px calc(-1*max(16px,env(safe-area-inset-right,0px))) 22px calc(-1*max(16px,env(safe-area-inset-left,0px)))}\
.site header{display:flex;flex-wrap:wrap;align-items:center;gap:.4rem 1.2rem;padding:.9rem 1.4rem;background:#0b0c0d;border-bottom:2px solid #d4a017;box-shadow:0 2px 18px #0008}\
.site .brand{font:700 1.6rem Impact,'Arial Narrow',sans-serif;letter-spacing:.28em;color:#d4a017;text-decoration:none}\
.site .tag{display:block;font:400 .7rem/1.55 system-ui,'Segoe UI',sans-serif;letter-spacing:.12em;text-transform:none;border:0;padding:0;border-radius:0;color:#8a8f94;flex:1;min-width:12rem}\
.site nav a{margin-right:.9rem;color:#e3e3e0;font-size:.85rem;text-decoration:none;text-transform:uppercase;letter-spacing:.08em}\
.site nav a:hover,.site nav a[aria-current]{color:#d4a017}\
.site .lang{color:#d4a017;font-size:.85rem;text-decoration:none;border:1px solid #d4a01766;padding:.15rem .6rem;border-radius:2px}\
.site .badge{background:#d64531;color:#fff;font:700 .75rem system-ui,sans-serif;letter-spacing:.14em;padding:.2rem .55rem;border-radius:2px}\
.site .note.test{max-width:1240px;margin:1rem auto 0;font-size:1rem;color:#e3e3e0;background:#1d2125;border-left:3px solid #d64531;padding:.7rem .9rem;border-radius:2px}\
.site a{color:#d4a017}";
