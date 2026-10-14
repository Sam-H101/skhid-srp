//! A small website for the people running srp.
//!
//! - `/` is a home page: what the project is and where it came from.
//! - `/exchange` lets a player exchange silver for gold: user name and password,
//!   then how much gold ([`SILVER_PER_GOLD`] silver each).
//! - `/launcher/…` and `/api/client-files` (the 0.26g page only) are for the
//!   patched game launcher and its updater ([`launcher`]): the page it shows
//!   in its window, and which archive `/download` hands out now.
//! - `/resources/<sha256>` (the 0.26g page only) is a blob of the updater's
//!   resource step, from the folder `resources` beside the download file
//!   ([`resource_file`]).
//! - The admin page changes the silver and gold of any account, for testing.
//!   It exists only when `SRP_ADMIN_TOKEN` and `SRP_ADMIN_PATH` (a hard to
//!   guess name, served at `/<name>`) are set, and only for the addresses in
//!   `SRP_ADMIN_IPS` (default: this machine). Anyone else gets the same
//!   "not found" as for a page that does not exist.
//!
//! It listens on `SRP_WEB_BIND:SRP_WEB_PORT`, by default 127.0.0.1:8088: the
//! page is plain HTTP, so a public host puts HTTPS in front of it (the
//! compose file's Caddy) instead of opening the port. Every request carries
//! its own credentials, so there are no cookies and nothing to forge across
//! sites.
//!
//! The page is of one game version, by how it is started: [`run`] is the
//! 0.21d page (balances of `session::Account::game`, `-client=`), and
//! [`run_026g`] the 0.26g one ([`v026g`]: the 0.26g accounts, the lobby told
//! of every change, `-server=`). `srp` starts the 0.26g one.

mod launcher;
mod stats;
mod v026g;

pub use v026g::{Game, LiveMatch, LivePhase, LiveStanding, LobbyLink, MatchCounter};

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use session::{Ban, LockExt, Net, SessionStore, SignInError};

/// 10 000 silver for 100 gold.
pub const SILVER_PER_GOLD: u32 = 100;
/// The most gold one exchange buys.
/// What the home page tells a new player, fixed at start-up.
struct Connect {
    /// The login server as the client is told to reach it (`host:port`).
    address: String,
    /// `SRP_DOWNLOAD_URL`: where the patched game can be had.
    download_url: Option<String>,
    /// `SRP_DOWNLOAD_SHA256`: its checksum.
    sha256: Option<String>,
    /// `SRP_DOWNLOAD_FILE`: a file this server hands out itself at
    /// `/download` (the only file it ever serves; no path comes from a request).
    download_file: Option<PathBuf>,
    /// `SRP_OTHER_SERVER_URL`: the page of the server for the other game
    /// version (0.21d and 0.26g link to each other); `https://` only.
    other_url: Option<String>,
    /// `SRP_GAME_URL` (the 0.26g page): where the original, unchanged game
    /// can be had; `https://` only. The page's own download is then only
    /// the few files that go over it.
    game_url: Option<String>,
    /// `SRP_TEST_SERVER=on`: every page says this is the test server. The
    /// link of `SRP_OTHER_SERVER_URL` is then the players' server.
    test_server: bool,
}

/// Downloads of that file at one time; more get "busy, try again". The
/// blobs of `/resources/` have as many slots again, of their own: they never
/// take the archive's.
const MAX_DOWNLOADS: u32 = 5;
/// A download slower than this for a minute is cut, so slow readers cannot
/// hold the slots.
const MIN_DOWNLOAD_RATE: u64 = 32 * 1024;
const DOWNLOAD_GRACE: Duration = Duration::from_secs(60);

static CONNECT: OnceLock<Connect> = OnceLock::new();

fn env_text(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

const MAX_EXCHANGE_GOLD: u32 = 10_000;
const MAX_REQUEST: usize = 4096;
const MAX_CONNECTIONS: u32 = 64;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest timed ban, in hours (ten years); longer is for good.
const MAX_BAN_HOURS: u64 = 24 * 365 * 10;
/// The furthest a maintenance can be planned: a week.
const MAX_MAINTENANCE_MINUTES: u64 = 7 * 24 * 60;
/// Wrong admin tokens in a row before the admin page waits.
const MAX_TOKEN_FAILURES: u32 = 5;
const TOKEN_LOCKOUT: Duration = Duration::from_secs(60);
/// The `Content-Security-Policy` of every page but the launcher's
/// ([`launcher::policy`]).
const POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'";

/// The page's languages: Ukrainian and English only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lang {
    En,
    Uk,
}

impl Lang {
    /// `?lang=` or the form's `lang`, else the browser's `Accept-Language`
    /// (Ukrainian when it comes first), else English. (`ua` is what the
    /// game's launcher calls Ukrainian in the addresses it opens.)
    fn pick(chosen: &str, accept: &str) -> Self {
        match chosen {
            "uk" | "ua" => return Self::Uk,
            "en" => return Self::En,
            _ => {}
        }
        let first = accept.split(',').next().unwrap_or("").trim().to_lowercase();
        if first.starts_with("uk") {
            Self::Uk
        } else {
            Self::En
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Uk => "uk",
        }
    }

    fn tr(self, en: &str, uk: &str) -> String {
        match self {
            Self::En => en.to_owned(),
            Self::Uk => uk.to_owned(),
        }
    }
}

/// What the home page counts.
#[derive(Clone, Copy)]
struct Stats {
    registered: usize,
    online: usize,
    /// The minutes to a planned maintenance.
    maintenance: Option<u32>,
    registrations_open: bool,
    /// The 0.26g page.
    v026g: bool,
    /// The download holds the launcher (0.26g): the page tells players to
    /// start that, and keeps the start script and the command line for
    /// those who want them.
    launcher: bool,
    /// The download's launcher brings the original game itself (0.26g):
    /// the page tells players to unpack into an empty folder, and names
    /// the game's own archive only as the way out when that fails.
    installs_game: bool,
}

enum Form {
    None,
    Home(Stats),
    Exchange,
    Admin(AdminView),
    /// The 0.26g page's "not found" and exchange.
    None26,
    Exchange26,
}

impl Form {
    fn is_026g(&self) -> bool {
        match self {
            Self::None | Self::Exchange => false,
            Self::None26 | Self::Exchange26 => true,
            Self::Home(stats) => stats.v026g,
            Self::Admin(view) => view.g26,
        }
    }
}

/// The admin page as shown: before the token (all empty), or after it.
#[derive(Default)]
struct AdminView {
    /// The token, once it was right: the form keeps it, so each action
    /// needs it only once.
    token: String,
    /// The tab to show.
    tab: String,
    /// The bans, as tables (after the token only).
    bans: String,
    /// What is planned, as a sentence (after the token only).
    maintenance: String,
    /// Whether new accounts are taken, for the form's buttons.
    registrations_open: bool,
    /// The 0.26g page: the account, item and text tabs too.
    g26: bool,
    /// The operator's text that stands (0.26g, after the token only).
    admin_text: Option<(String, String)>,
}

/// Frees a download slot when the transfer ends, however it ends.
struct Slot<'a>(&'a AtomicU32);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The file of `/resources/<name>` beside the download file `download`:
/// `<its folder>/resources/<name>`, for a name of 64 lower-case hex digits
/// (the SHA-256 of the blob; `client/updater`, "Resources") and no other.
/// No other part of a request's path ever names a file.
fn resource_file(path: &str, download: &Path) -> Option<PathBuf> {
    let name = path.strip_prefix("/resources/")?;
    if name.len() != 64 || !name.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    Some(download.parent()?.join("resources").join(name))
}

/// The blob a request asks for: a `GET` or `HEAD` of `/resources/<name>`
/// on the 0.26g page (`v026g`), when there is a download file. Anything
/// else is the page's to answer, as before there were blobs.
fn resource_request(v026g: bool, request: &Request, download: Option<&Path>) -> Option<PathBuf> {
    if !v026g || !matches!(request.method.as_str(), "GET" | "HEAD") {
        return None;
    }
    resource_file(&request.path, download?)
}

/// The file to hand out at `/download`, if one is set and there.
fn available_download() -> Option<&'static PathBuf> {
    CONNECT
        .get()?
        .download_file
        .as_ref()
        .filter(|file| file.is_file())
}

/// A `Range` header against a file of `size` bytes: `Some(None)` when there is
/// none (the whole file), `Some(Some((first, last)))` for one range (inclusive),
/// `None` when it cannot be met. Only a single `bytes=` range is understood;
/// anything else gets the whole file.
fn parse_range(header: &str, size: u64) -> Option<Option<(u64, u64)>> {
    let Some(spec) = header.trim().strip_prefix("bytes=") else {
        return Some(None);
    };
    if spec.contains(',') {
        return Some(None);
    }
    let (first, last) = spec.split_once('-')?;
    if size == 0 {
        return None;
    }
    let (first, last) = if first.is_empty() {
        // The last N bytes.
        let count: u64 = last.trim().parse().ok().filter(|count| *count > 0)?;
        (size.saturating_sub(count), size - 1)
    } else {
        let first: u64 = first.trim().parse().ok()?;
        let last = if last.trim().is_empty() {
            size - 1
        } else {
            last.trim().parse::<u64>().ok()?.min(size - 1)
        };
        (first, last)
    };
    (first <= last && first < size).then_some(Some((first, last)))
}

struct Web {
    store: Arc<SessionStore>,
    token: Option<String>,
    /// `/<SRP_ADMIN_PATH>`; the admin page is off without it.
    admin_path: Option<String>,
    /// Who may see the admin page.
    admin_ips: Vec<Net>,
    /// Take the client's address from `X-Forwarded-For` (the last entry:
    /// the one a reverse proxy added), for a proxy in front.
    trust_proxy: bool,
    token_failures: Mutex<(u32, Option<Instant>)>,
    connections: AtomicU32,
    downloads: AtomicU32,
    /// Downloads of `/resources/` blobs now.
    resource_downloads: AtomicU32,
    /// `Some`: the page of the 0.26g stack.
    game: Option<Game>,
}

/// Who may use the admin page, and how.
#[derive(Clone, Debug, Default)]
pub struct Admin {
    /// `SRP_ADMIN_TOKEN`; the admin page is off without it.
    pub token: Option<String>,
    /// `/<SRP_ADMIN_PATH>`; the admin page is off without it.
    pub path: Option<String>,
    /// `SRP_ADMIN_IPS`.
    pub ips: Vec<Net>,
    /// `SRP_WEB_TRUST_PROXY=on`.
    pub trust_proxy: bool,
}

/// The 0.21d page, as the environment asks for it.
pub fn run(store: Arc<SessionStore>) -> io::Result<()> {
    run_from_env(store, None)
}

/// The 0.26g page, as the environment asks for it (the same settings).
pub fn run_026g(store: Arc<SessionStore>, game: Game) -> io::Result<()> {
    run_from_env(store, Some(game))
}

fn run_from_env(store: Arc<SessionStore>, game: Option<Game>) -> io::Result<()> {
    let bind = std::env::var("SRP_WEB_BIND").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("SRP_WEB_PORT").unwrap_or_else(|_| "8088".into());
    let address = vostok::config::load().map_or_else(
        |_| "127.0.0.1:1234".into(),
        |config| config.login_server.public_addr(),
    );
    log::info!("Home page tells players to connect to {address}");
    let _ = CONNECT.set(Connect {
        address,
        download_url: env_text("SRP_DOWNLOAD_URL").filter(|url| url.starts_with("https://")),
        download_file: env_text("SRP_DOWNLOAD_FILE").map(PathBuf::from),
        sha256: env_text("SRP_DOWNLOAD_SHA256")
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())),
        other_url: env_text("SRP_OTHER_SERVER_URL").and_then(|url| other_url(&url)),
        game_url: env_text("SRP_GAME_URL").filter(|url| url.starts_with("https://")),
        test_server: env_text("SRP_TEST_SERVER")
            .is_some_and(|value| value.eq_ignore_ascii_case("on")),
    });
    let token = std::env::var("SRP_ADMIN_TOKEN")
        .ok()
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty());
    let admin_path = std::env::var("SRP_ADMIN_PATH")
        .ok()
        .map(|path| path.trim().trim_matches('/').to_owned())
        .filter(|path| !path.is_empty())
        .map(|path| format!("/{path}"));
    let admin_ips: Vec<Net> = match std::env::var("SRP_ADMIN_IPS") {
        Ok(list) => list.split(',').filter_map(Net::parse).collect(),
        Err(_) => ["127.0.0.1", "::1"]
            .into_iter()
            .filter_map(Net::parse)
            .collect(),
    };
    if let Some(file) = CONNECT
        .get()
        .and_then(|connect| connect.download_file.as_ref())
    {
        match std::fs::metadata(file) {
            Ok(meta) if meta.is_file() => {
                log::info!(
                    "Downloads: {} ({} bytes) at /download",
                    file.display(),
                    meta.len()
                );
            }
            _ => log::warn!(
                "SRP_DOWNLOAD_FILE {} is not a file; /download is off",
                file.display()
            ),
        }
    }
    let trust_proxy = std::env::var("SRP_WEB_TRUST_PROXY")
        .is_ok_and(|value| value.trim().eq_ignore_ascii_case("on"));
    let listener = TcpListener::bind(format!("{bind}:{port}"))?;
    serve_on(
        &listener,
        store,
        Admin {
            token,
            path: admin_path,
            ips: admin_ips,
            trust_proxy,
        },
        game,
    )
}

/// Serve the page on `listener` until it fails: the 0.26g page with a
/// `game`, else the 0.21d one. (What the home page tells of the address and
/// the download comes from the environment, through [`run`] / [`run_026g`]
/// only.)
pub fn serve_on(
    listener: &TcpListener,
    store: Arc<SessionStore>,
    admin: Admin,
    game: Option<Game>,
) -> io::Result<()> {
    let admin_state = match (&admin.token, &admin.path) {
        (Some(_), Some(_)) => format!("admin on for {} address(es)", admin.ips.len()),
        _ => "admin off: SRP_ADMIN_TOKEN and SRP_ADMIN_PATH are needed".to_owned(),
    };
    log::info!(
        "Web server on {} ({} page: exchange, {admin_state})",
        listener.local_addr()?,
        if game.is_some() { "0.26g" } else { "0.21d" }
    );
    let web = Arc::new(Web {
        store,
        token: admin.token,
        admin_path: admin.path,
        admin_ips: admin.ips,
        trust_proxy: admin.trust_proxy,
        token_failures: Mutex::new((0, None)),
        connections: AtomicU32::new(0),
        downloads: AtomicU32::new(0),
        resource_downloads: AtomicU32::new(0),
        game,
    });
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if web.connections.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
            web.connections.fetch_sub(1, Ordering::AcqRel);
            continue;
        }
        let spawned = thread::Builder::new().name("web-connection".into()).spawn({
            let web = web.clone();
            move || {
                web.serve(stream);
                web.connections.fetch_sub(1, Ordering::AcqRel);
            }
        });
        if spawned.is_err() {
            web.connections.fetch_sub(1, Ordering::AcqRel);
        }
    }
    Ok(())
}

struct Request {
    method: String,
    path: String,
    lang: Lang,
    /// `X-Forwarded-For`, as sent.
    forwarded: String,
    /// `Range`, as sent.
    range: String,
    /// Who is asking: the connection's address (see `Web::trust_proxy`).
    client: Option<IpAddr>,
    form: Vec<(String, String)>,
}

impl Request {
    fn field(&self, name: &str) -> &str {
        self.form
            .iter()
            .find(|(key, _)| key == name)
            .map_or("", |(_, value)| value.trim())
    }

    /// The field as sent, spaces kept (a password).
    fn raw(&self, name: &str) -> &str {
        self.form
            .iter()
            .find(|(key, _)| key == name)
            .map_or("", |(_, value)| value.as_str())
    }
}

#[derive(Clone, Copy)]
enum ContentType {
    Html,
    Json,
    Text,
}

impl ContentType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Html => "text/html; charset=utf-8",
            Self::Json => "application/json",
            Self::Text => "text/plain; charset=utf-8",
        }
    }
}

struct Response {
    status: &'static str,
    content_type: ContentType,
    body: String,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Web {
    fn serve(&self, mut stream: TcpStream) {
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        let peer = stream
            .peer_addr()
            .map(|address| address.to_string())
            .unwrap_or_default();
        let Some(mut request) = read_request(&mut stream) else {
            return;
        };
        let connection = stream.peer_addr().ok().map(|address| address.ip());
        request.client = if self.trust_proxy {
            request
                .forwarded
                .rsplit(',')
                .next()
                .and_then(|last| last.trim().parse().ok())
        } else {
            connection
        };
        if request.path == "/download" && matches!(request.method.as_str(), "GET" | "HEAD") {
            if let Some(file) = available_download() {
                self.download(&mut stream, &request, file);
                return;
            }
        }
        if let Some(file) = resource_request(
            self.game.is_some(),
            &request,
            available_download().map(PathBuf::as_path),
        ) {
            self.resource(&mut stream, &request, &file);
            return;
        }
        let response = self.route(&request, &peer);
        let policy = if self.game.is_some() && launcher::serves(&request.path) {
            launcher::policy()
        } else {
            POLICY
        };
        let head = format!(
            "HTTP/1.0 {}\r\nContent-Type: {}\r\n\
             Content-Length: {}\r\nCache-Control: no-store\r\n\
             X-Frame-Options: DENY\r\nX-Content-Type-Options: nosniff\r\n\
             Referrer-Policy: no-referrer\r\n\
             Content-Security-Policy: {policy}\r\nConnection: close\r\n\r\n",
            response.status,
            response.content_type.as_str(),
            response.body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(response.body.as_bytes());
        let _ = stream.flush();
        let _ = stream.shutdown(Shutdown::Write);
    }

    /// Hands out the download file: whole, or from a `Range` (so a broken
    /// download resumes). At most [`MAX_DOWNLOADS`] at once.
    fn download(&self, stream: &mut TcpStream, request: &Request, file: &Path) {
        self.send_file(stream, request, file, &self.downloads, "application/zip");
    }

    /// Hands out a blob of `/resources/` as the download file is handed
    /// out, in slots of its own; "not found" when there is no such file.
    fn resource(&self, stream: &mut TcpStream, request: &Request, file: &Path) {
        self.send_file(
            stream,
            request,
            file,
            &self.resource_downloads,
            "application/octet-stream",
        );
    }

    /// A file, whole or from a `Range`, while fewer than [`MAX_DOWNLOADS`]
    /// of `slots` are taken.
    fn send_file(
        &self,
        stream: &mut TcpStream,
        request: &Request,
        file: &Path,
        slots: &AtomicU32,
        content_type: &str,
    ) {
        let reply = |stream: &mut TcpStream, status: &str, extra: &str| {
            let _ = stream.write_all(
                format!(
                    "HTTP/1.0 {status}\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            );
        };
        if slots.fetch_add(1, Ordering::AcqRel) >= MAX_DOWNLOADS {
            slots.fetch_sub(1, Ordering::AcqRel);
            reply(stream, "503 Service Unavailable", "Retry-After: 60\r\n");
            return;
        }
        let _slot = Slot(slots);
        let Ok(mut open) = File::open(file) else {
            reply(stream, "404 Not Found", "");
            return;
        };
        // (A folder opens too, on some systems.)
        let Some(size) = open
            .metadata()
            .ok()
            .filter(|meta| meta.is_file())
            .map(|meta| meta.len())
        else {
            reply(stream, "404 Not Found", "");
            return;
        };
        let (first, last, partial) = match parse_range(&request.range, size) {
            Some(Some((first, last))) => (first, last, true),
            Some(None) => (0, size.saturating_sub(1), false),
            None => {
                reply(
                    stream,
                    "416 Range Not Satisfiable",
                    &format!("Content-Range: bytes */{size}\r\n"),
                );
                return;
            }
        };
        let length = if size == 0 { 0 } else { last - first + 1 };
        let name: String = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "._-".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let range = if partial {
            format!("Content-Range: bytes {first}-{last}/{size}\r\n")
        } else {
            String::new()
        };
        let head = format!(
            "HTTP/1.0 {}\r\nContent-Type: {content_type}\r\n\
             Content-Disposition: attachment; filename=\"{name}\"\r\n\
             Accept-Ranges: bytes\r\n{range}Content-Length: {length}\r\n\
             X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
            if partial {
                "206 Partial Content"
            } else {
                "200 OK"
            },
        );
        if stream.write_all(head.as_bytes()).is_err() || request.method == "HEAD" || length == 0 {
            return;
        }
        if open.seek(SeekFrom::Start(first)).is_err() {
            return;
        }
        let started = Instant::now();
        let mut sent = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        while sent < length {
            let want = buffer.len().min((length - sent) as usize);
            let Ok(read) = open.read(&mut buffer[..want]) else {
                return;
            };
            if read == 0 || stream.write_all(&buffer[..read]).is_err() {
                return;
            }
            sent += read as u64;
            let elapsed = started.elapsed();
            if elapsed > DOWNLOAD_GRACE && sent < MIN_DOWNLOAD_RATE * elapsed.as_secs() {
                return;
            }
        }
        let _ = stream.flush();
    }

    fn route(&self, request: &Request, peer: &str) -> Response {
        if let Some(game) = &self.game {
            if request.method == "GET" {
                match request.path.as_str() {
                    "/api/stats" => return stats::response(&self.store, game),
                    "/api/client-files" => return launcher::client_files(),
                    path if path.starts_with(launcher::FRESH) => {
                        return launcher::fresh(
                            launcher::language(path, request.lang),
                            &self.store,
                            game,
                        );
                    }
                    path if path.starts_with(launcher::FRESH_BEFORE) => {
                        return launcher::fresh_before(
                            launcher::language(path, request.lang),
                            &self.store,
                            game,
                        );
                    }
                    path if launcher::serves(path) => {
                        return launcher::page(
                            launcher::language(path, request.lang),
                            &self.store,
                            game,
                        );
                    }
                    _ => {}
                }
            }
            return self.route_026g(request, peer);
        }
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/") => page(
                request.lang,
                Form::Home(Stats {
                    registered: self.store.registered_accounts(),
                    online: self.store.players_present(),
                    maintenance: self.store.minutes_to_maintenance(),
                    registrations_open: self.store.registrations_open(),
                    v026g: false,
                    launcher: false,
                    installs_game: false,
                }),
                "",
            ),
            ("GET", "/exchange") => page(request.lang, Form::Exchange, ""),
            ("POST", "/exchange") => self.exchange(request, peer),
            ("GET", path) if self.is_admin_page(path, request) => {
                page(request.lang, Form::Admin(AdminView::default()), "")
            }
            ("POST", path) if self.is_admin_page(path, request) => self.admin(request, peer),
            _ => Response {
                status: "404 Not Found",
                content_type: ContentType::Html,
                body: page(
                    request.lang,
                    Form::None,
                    &request.lang.tr("Not found.", "Не знайдено."),
                )
                .body,
            },
        }
    }

    /// Whether `path` is the admin page and `request` may see it.
    fn is_admin_page(&self, path: &str, request: &Request) -> bool {
        self.token.is_some()
            && self.admin_path.as_deref() == Some(path)
            && request
                .client
                .is_some_and(|ip| self.admin_ips.iter().any(|net| net.contains(ip)))
    }

    fn exchange(&self, request: &Request, peer: &str) -> Response {
        let lang = request.lang;
        let user = request.field("user");
        let wrong_login = lang.tr(
            "Wrong user name or password.",
            "Неправильне ім'я користувача або пароль.",
        );
        let gold: u32 = match request.field("gold").parse() {
            Ok(gold) if (1..=MAX_EXCHANGE_GOLD).contains(&gold) => gold,
            _ => {
                return page(
                    lang,
                    Form::Exchange,
                    &lang.tr(
                        &format!("Gold must be 1 to {MAX_EXCHANGE_GOLD}."),
                        &format!("Золота має бути від 1 до {MAX_EXCHANGE_GOLD}."),
                    ),
                );
            }
        };
        let password = request.field("password").as_bytes();
        let checked = match request.client {
            Some(ip) => self.store.check_password_from(user, password, ip),
            None => self.store.check_password(user, password),
        };
        let account_id = match checked {
            Ok(id) => id,
            Err(SignInError::Banned) => {
                log::warn!("Exchange for banned account {user:?} from {peer} refused");
                return page(
                    lang,
                    Form::Exchange,
                    &lang.tr("This account is banned.", "Цей акаунт заблоковано."),
                );
            }
            Err(SignInError::TooManyAttempts | SignInError::TooManyFromAddress) => {
                return page(
                    lang,
                    Form::Exchange,
                    &lang.tr(
                        "Too many wrong passwords; wait a minute.",
                        "Забагато хибних паролів; зачекайте хвилину.",
                    ),
                );
            }
            Err(_) => {
                log::warn!("Exchange for {user:?} from {peer}: wrong user name or password");
                return page(lang, Form::Exchange, &wrong_login);
            }
        };
        let Some(account) = self.store.account(account_id) else {
            return page(lang, Form::Exchange, &wrong_login);
        };
        let cost = gold * SILVER_PER_GOLD;
        let result = {
            let mut account = locked(&account);
            let game = &mut account.game;
            match (
                game.generic_money.checked_sub(cost),
                game.premium_money.checked_add(gold),
            ) {
                (Some(silver), Some(total)) => {
                    game.generic_money = silver;
                    game.premium_money = total;
                    Ok((silver, total))
                }
                (None, _) => Err(lang.tr(
                    &format!(
                        "Not enough silver: {gold} gold costs {cost}, you have {}.",
                        game.generic_money
                    ),
                    &format!(
                        "Недостатньо срібла: {gold} золота коштує {cost}, у вас {}.",
                        game.generic_money
                    ),
                )),
                _ => Err(lang.tr(
                    "That would overflow the gold balance.",
                    "Баланс золота переповниться.",
                )),
            }
        };
        match result {
            Ok((silver, total)) => {
                self.store.mark_dirty(account_id);
                self.store.save_soon();
                log::info!(
                    "Exchange for {user:?} from {peer}: {cost} silver for {gold} gold \
                     (now {silver} silver, {total} gold)"
                );
                page(
                    lang,
                    Form::Exchange,
                    &lang.tr(
                        &format!(
                            "Done: {cost} silver for {gold} gold. Now {silver} silver and \
                             {total} gold. An open game shows it within a second."
                        ),
                        &format!(
                            "Готово: {cost} срібла за {gold} золота. Тепер {silver} срібла і \
                             {total} золота. Відкрита гра покаже це за секунду."
                        ),
                    ),
                )
            }
            Err(error) => page(lang, Form::Exchange, &error),
        }
    }

    fn admin(&self, request: &Request, peer: &str) -> Response {
        let lang = request.lang;
        // The tab the form was on says what to do: Enter in any field
        // submits the form, whichever button that presses.
        let g26 = self.game.is_some();
        let tab = match request.field("tab") {
            tab @ ("money" | "ban" | "unban" | "bans" | "maint" | "signups") => tab,
            tab @ ("account" | "item" | "text") if g26 => tab,
            _ => "money",
        };
        let refused = |message: String| {
            let view = AdminView {
                tab: tab.to_owned(),
                g26,
                ..AdminView::default()
            };
            page(lang, Form::Admin(view), &message)
        };
        if self.token_locked() {
            return refused(lang.tr(
                "Too many wrong tokens; wait a minute.",
                "Забагато хибних токенів; зачекайте хвилину.",
            ));
        }
        let given = request.field("token");
        let right = self.token.as_deref().unwrap_or_default();
        if !same(given.as_bytes(), right.as_bytes()) {
            self.token_failed();
            log::warn!("Admin with a wrong token from {peer}");
            return refused(lang.tr("Wrong token.", "Неправильний токен."));
        }
        *locked(&self.token_failures) = (0, None);
        let message = match tab {
            "ban" => self.admin_ban(request, peer),
            "unban" => self.admin_unban(request, peer),
            "bans" => String::new(),
            "maint" => self.admin_maintenance(request, peer),
            "signups" => self.admin_signups(request, peer),
            "account" => self.admin_account_026g(request, peer),
            "item" => self.admin_item_026g(request, peer),
            "text" => self.admin_text_026g(request, peer),
            _ if g26 => self.admin_money_026g(request, peer),
            _ => self.admin_money(request, peer),
        };
        let view = AdminView {
            token: given.to_owned(),
            tab: tab.to_owned(),
            bans: bans_table(lang, &self.store),
            maintenance: maintenance_line(lang, &self.store),
            registrations_open: self.store.registrations_open(),
            g26,
            admin_text: self.game.as_ref().and_then(|game| game.lobby.admin_text()),
        };
        page(lang, Form::Admin(view), &message)
    }

    fn admin_money(&self, request: &Request, peer: &str) -> String {
        let lang = request.lang;
        let user = request.field("user").trim();
        let no_such = lang.tr("No such user.", "Такого користувача немає.");
        let Some(account_id) = self.store.find_account(user) else {
            return no_such;
        };
        let (Some(silver), Some(gold)) =
            (delta(request.field("silver")), delta(request.field("gold")))
        else {
            return lang.tr(
                "Silver and gold are whole numbers, + or -, or empty.",
                "Срібло і золото — цілі числа, + або -, або порожньо.",
            );
        };
        let Some(account) = self.store.account(account_id) else {
            return no_such;
        };
        let (silver_now, gold_now) = {
            let mut account = locked(&account);
            let game = &mut account.game;
            game.generic_money = apply(game.generic_money, silver);
            game.premium_money = apply(game.premium_money, gold);
            (game.generic_money, game.premium_money)
        };
        self.store.mark_dirty(account_id);
        self.store.save_soon();
        log::info!(
            "Admin from {peer}: {user:?} silver {silver:+}, gold {gold:+} \
             (now {silver_now} silver, {gold_now} gold)"
        );
        lang.tr(
            &format!("{user}: now {silver_now} silver and {gold_now} gold."),
            &format!("{user}: тепер {silver_now} срібла і {gold_now} золота."),
        )
    }

    /// Open or close registration of new accounts.
    fn admin_signups(&self, request: &Request, peer: &str) -> String {
        let lang = request.lang;
        let open = match request.field("signups") {
            "open" => true,
            "closed" => false,
            _ => return String::new(),
        };
        self.store.set_registrations_open(open);
        log::warn!(
            "Admin from {peer}: registration of new accounts {}",
            if open { "opened" } else { "closed" }
        );
        if open {
            lang.tr(
                "New accounts are taken again.",
                "Нові акаунти знову приймаються.",
            )
        } else {
            lang.tr(
                "New accounts are refused; existing players sign in as before.",
                "Нові акаунти не приймаються; гравці з акаунтом входять як раніше.",
            )
        }
    }

    /// Plan the maintenance `minutes` from now, or cancel it. The clients
    /// are told within a few seconds: from an hour before they show the
    /// minutes left, 30 minutes before they lock Play, and at the time they
    /// quit.
    fn admin_maintenance(&self, request: &Request, peer: &str) -> String {
        let lang = request.lang;
        if !request.field("cancel").is_empty() {
            let was = self.store.maintenance_at().is_some();
            self.store.set_maintenance(None);
            if was {
                log::warn!("Admin from {peer}: cancelled the maintenance");
            }
            return if was {
                lang.tr("Maintenance cancelled.", "Технічні роботи скасовано.")
            } else {
                lang.tr(
                    "No maintenance was planned.",
                    "Технічних робіт не було заплановано.",
                )
            };
        }
        let minutes = request.field("minutes").trim();
        let Some(minutes) = minutes
            .parse::<u64>()
            .ok()
            .filter(|minutes| (1..=MAX_MAINTENANCE_MINUTES).contains(minutes))
        else {
            return lang.tr(
                &format!("Minutes: a whole number from 1 to {MAX_MAINTENANCE_MINUTES}."),
                &format!("Хвилини: ціле число від 1 до {MAX_MAINTENANCE_MINUTES}."),
            );
        };
        let at = unix_now() + minutes * 60;
        self.store.set_maintenance(Some(at));
        log::warn!("Admin from {peer}: maintenance planned in {minutes} min");
        lang.tr(
            &format!(
                "Maintenance starts in {minutes} min, at {} UTC.",
                clock_utc(at)
            ),
            &format!(
                "Технічні роботи почнуться через {minutes} хв, о {} UTC.",
                clock_utc(at)
            ),
        )
    }

    /// Ban an account (and, if asked, the address it last signed in from),
    /// or an address or range.
    fn admin_ban(&self, request: &Request, peer: &str) -> String {
        let lang = request.lang;
        let target = request.field("ban_user").trim();
        let hours = request.field("hours").trim();
        let until = match hours.parse::<u64>() {
            _ if hours.is_empty() => None,
            Ok(0) => None,
            Ok(hours) if hours <= MAX_BAN_HOURS => Some(unix_now() + hours * 3600),
            _ => {
                return lang.tr(
                    &format!("Hours: a whole number up to {MAX_BAN_HOURS}, or empty for good."),
                    &format!("Години: ціле число до {MAX_BAN_HOURS}, або порожньо — назавжди."),
                );
            }
        };
        let reason: String = request.field("reason").trim().chars().take(200).collect();
        let ban = Ban { until, reason };
        let how_long = match until {
            Some(_) => lang.tr(&format!("for {hours} h"), &format!("на {hours} год")),
            None => lang.tr("for good", "назавжди"),
        };
        if let Some(net) = Net::parse(target) {
            let kicked = self.store.ban_net(net, ban.clone());
            log::warn!(
                "Admin from {peer}: banned address {net} {how_long} ({:?}); \
                 {kicked} session(s) ended",
                ban.reason
            );
            return lang.tr(
                &format!("Address {net} banned {how_long}; {kicked} session(s) ended."),
                &format!("Адресу {net} заблоковано {how_long}; завершено сесій: {kicked}."),
            );
        }
        let Some(account_id) = self.store.find_account(target) else {
            return lang.tr(
                "No such user or address. A name several accounts share needs the user name they sign in with.",
                "Немає такого користувача чи адреси. Якщо ім'я мають кілька акаунтів, вкажіть ім'я для входу.",
            );
        };
        let kicked = self.store.ban_account(account_id, ban.clone());
        log::warn!(
            "Admin from {peer}: banned account {target:?} ({account_id}) {how_long} ({:?}); \
             {kicked} session(s) ended",
            ban.reason
        );
        let mut message = lang.tr(
            &format!("{target} banned {how_long}; {kicked} session(s) ended."),
            &format!("{target} заблоковано {how_long}; завершено сесій: {kicked}."),
        );
        if request.field("with_ip") == "on" {
            let last_ip = self
                .store
                .account(account_id)
                .and_then(|account| account.locked().last_ip);
            match last_ip {
                Some(ip) => {
                    let net = Net::single(ip);
                    let more = self.store.ban_net(net, ban);
                    log::warn!(
                        "Admin from {peer}: banned address {net} too; {more} more session(s) ended"
                    );
                    message += &lang.tr(
                        &format!(" Its address {net} too ({more} more session(s) ended)."),
                        &format!(" І його адресу {net} (ще завершено сесій: {more})."),
                    );
                }
                None => {
                    message += &lang.tr(
                        " Its address is not known, so no address was banned.",
                        " Його адреса невідома, тож адресу не заблоковано.",
                    );
                }
            }
        }
        message
    }

    fn admin_unban(&self, request: &Request, peer: &str) -> String {
        let lang = request.lang;
        let target = request.field("unban_user").trim();
        if let Some(net) = Net::parse(target) {
            return if self.store.unban_net(net) {
                log::warn!("Admin from {peer}: lifted the ban of address {net}");
                lang.tr(
                    &format!("Address {net} unbanned."),
                    &format!("Адресу {net} розблоковано."),
                )
            } else {
                lang.tr(
                    &format!("{net} was not banned (a range is lifted as it was banned)."),
                    &format!(
                        "{net} не була заблокована (діапазон знімається таким, яким його заблокували)."
                    ),
                )
            };
        }
        let Some(account_id) = self.store.find_account(target) else {
            return lang.tr(
                "No such user or address.",
                "Немає такого користувача чи адреси.",
            );
        };
        let mut message = if self.store.unban_account(account_id) {
            log::warn!("Admin from {peer}: lifted the ban of account {target:?} ({account_id})");
            lang.tr(
                &format!("{target} unbanned."),
                &format!("{target} розблоковано."),
            )
        } else {
            lang.tr(
                &format!("{target} was not banned."),
                &format!("{target} не був заблокований."),
            )
        };
        if request.field("unban_ip") == "on"
            && let Some(ip) = self
                .store
                .account(account_id)
                .and_then(|account| account.locked().last_ip)
            && self.store.unban_net(Net::single(ip))
        {
            log::warn!("Admin from {peer}: lifted the ban of address {ip}");
            message += &lang.tr(
                &format!(" Its address {ip} too."),
                &format!(" І його адресу {ip}."),
            );
        }
        message
    }

    fn token_locked(&self) -> bool {
        let mut failures = locked(&self.token_failures);
        match failures.1 {
            Some(last) if last.elapsed() >= TOKEN_LOCKOUT => {
                *failures = (0, None);
                false
            }
            _ => failures.0 >= MAX_TOKEN_FAILURES,
        }
    }

    fn token_failed(&self) {
        let mut failures = locked(&self.token_failures);
        failures.0 += 1;
        failures.1 = Some(Instant::now());
    }
}

/// A signed whole number, or nothing for 0.
fn delta(text: &str) -> Option<i64> {
    if text.is_empty() {
        return Some(0);
    }
    text.parse::<i64>()
        .ok()
        .filter(|number| number.abs() <= 1_000_000_000)
}

/// `balance` changed by `change`, not below 0.
fn apply(balance: u32, change: i64) -> u32 {
    u32::try_from((i64::from(balance) + change).max(0)).unwrap_or(u32::MAX)
}

/// Equal, in time that does not tell where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    let mut difference = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b) {
        difference |= usize::from(x ^ y);
    }
    difference == 0
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut data = Vec::new();
    let mut chunk = [0_u8; 1024];
    let head_end = loop {
        if let Some(at) = data.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        if data.len() > MAX_REQUEST {
            return None;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => data.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_owned();
    let target = first.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let path = path.to_owned();
    let accept = head
        .split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("accept-language"))
        .map_or("", |(_, value)| value.trim())
        .to_owned();
    let forwarded = head
        .split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("x-forwarded-for"))
        .map_or("", |(_, value)| value.trim())
        .to_owned();
    let range = head
        .split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("range"))
        .map_or("", |(_, value)| value.trim())
        .to_owned();
    let length: usize = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
        .unwrap_or(0);
    if length > MAX_REQUEST {
        return None;
    }
    while data.len() < head_end + length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => data.extend_from_slice(&chunk[..n]),
        }
    }
    let body = String::from_utf8_lossy(&data[head_end..head_end + length]).into_owned();
    let pairs = |text: &str| -> Vec<(String, String)> {
        text.split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(key, value)| (decode(key), decode(value)))
            .collect()
    };
    let form = pairs(&body);
    let chosen = form
        .iter()
        .chain(pairs(query).iter())
        .find(|(key, _)| key == "lang")
        .map_or("", |(_, value)| value.as_str())
        .to_owned();
    Some(Request {
        method,
        path,
        lang: Lang::pick(&chosen, &accept),
        forwarded,
        range,
        client: None,
        form,
    })
}

/// `application/x-www-form-urlencoded` text.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'+' => out.push(b' '),
            b'%' => {
                let byte = text
                    .get(at + 1..at + 3)
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                match byte {
                    Some(byte) => {
                        out.push(byte);
                        at += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `SRP_OTHER_SERVER_URL` as the pages use it: a plain `https://` address
/// without a query, or nothing.
fn other_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let host = url.strip_prefix("https://")?;
    let plain = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~/:".contains(&b));
    plain.then(|| url.to_owned())
}

/// The header's link to the server for the other game version (`version`),
/// in the page's language; empty without `SRP_OTHER_SERVER_URL`.
fn other_server_link(lang: Lang, version: &str, url: Option<&str>) -> String {
    url.map_or_else(String::new, |url| {
        format!(
            " <a href=\"{}/?lang={}\">Survarium {version}</a>",
            escape(url),
            lang.code()
        )
    })
}

/// The page's title: a test server says so (`SRP_TEST_SERVER`).
fn page_title(lang: Lang, version: &str, test: bool) -> String {
    match (test, lang) {
        (false, Lang::En) => format!("Skhid · Survarium {version} community server"),
        (false, Lang::Uk) => format!("Схід · ком'юніті сервер Survarium {version}"),
        (true, Lang::En) => format!("Skhid · Survarium {version} TEST server"),
        (true, Lang::Uk) => format!("Схід · ТЕСТОВИЙ сервер Survarium {version}"),
    }
}

/// The line beside the name in the header.
fn page_tag(lang: Lang, version: &str, test: bool) -> String {
    let version = version.to_uppercase();
    match (test, lang) {
        (false, Lang::En) => {
            format!("SKHID · COMMUNITY SERVER FOR SURVARIUM {version} · UNOFFICIAL")
        }
        (false, Lang::Uk) => format!("КОМ'ЮНІТІ СЕРВЕР SURVARIUM {version} · НЕОФІЦІЙНИЙ"),
        (true, Lang::En) => format!("SKHID · TEST SERVER FOR SURVARIUM {version} · UNOFFICIAL"),
        (true, Lang::Uk) => format!("ТЕСТОВИЙ СЕРВЕР SURVARIUM {version} · НЕОФІЦІЙНИЙ"),
    }
}

/// What a test server puts at the top of every page: what it is, and where
/// the players' server is (`SRP_OTHER_SERVER_URL`, when set).
fn test_server_notice(lang: Lang, players_url: Option<&str>) -> String {
    let players = players_url.map_or_else(String::new, |url| {
        format!(
            " {} <a href=\"{}/?lang={}\">{}</a>.",
            lang.tr("The players' server:", "Сервер для гравців:"),
            escape(url),
            lang.code(),
            escape(url.trim_start_matches("https://")),
        )
    });
    format!(
        "<p class=\"note test\"><b>{}</b> {}{players}</p>",
        lang.tr("Test server.", "Тестовий сервер."),
        lang.tr(
            "Changes are tried here before they reach the players: it can restart at any \
             moment, and accounts and progress here can be reset.",
            "Тут перевіряються зміни, перш ніж вони потраплять до гравців: сервер може \
             перезапуститися будь-якої миті, а облікові записи й прогрес можуть бути скинуті.",
        ),
    )
}

fn page(lang: Lang, form: Form, message: &str) -> Response {
    let connect = CONNECT.get();
    let test = connect.is_some_and(|connect| connect.test_server);
    let other_url = connect.and_then(|connect| connect.other_url.as_deref());
    page_as(lang, form, message, test, other_url)
}

/// `page` with what the settings say given to it.
fn page_as(lang: Lang, form: Form, message: &str, test: bool, other_url: Option<&str>) -> Response {
    let notice = if message.is_empty() {
        String::new()
    } else {
        format!("<p class=note>{}</p>", escape(message))
    };
    let code = lang.code();
    let (other, other_name) = match lang {
        Lang::En => ("uk", "Українська"),
        Lang::Uk => ("en", "English"),
    };
    let version = if form.is_026g() { "0.26g" } else { "0.21d" };
    let other_version = if form.is_026g() { "0.21d" } else { "0.26g" };
    // A test server names the other server in its notice, as the players'.
    let (other_link, badge, test_notice) = if test {
        (
            String::new(),
            format!(" <span class=badge>{}</span>", lang.tr("TEST", "ТЕСТ")),
            test_server_notice(lang, other_url),
        )
    } else {
        (
            other_server_link(lang, other_version, other_url),
            String::new(),
            String::new(),
        )
    };
    let body = match form {
        Form::Exchange => exchange_form(lang),
        Form::Exchange26 => v026g::exchange_form(lang),
        Form::Admin(view) => admin_form(lang, &view),
        Form::Home(stats) => home(lang, stats),
        Form::None | Form::None26 => String::new(),
    };
    let disclaimer = lang.tr(
        "Community server, unofficial. Not affiliated with or endorsed by Vostok Games. \
         Survarium is a trademark of its owners.",
        "Неофіційний ком'юніті сервер. Не пов'язаний із Vostok Games і не схвалений ними. \
         Survarium є торговельною маркою її власників.",
    );
    Response {
        status: "200 OK",
        content_type: ContentType::Html,
        body: format!(
            "<!doctype html><html lang={code}><meta charset=utf-8>\
             <meta name=viewport content=\"width=device-width,initial-scale=1\">\
             <title>{}</title><style>{STYLE}</style>\
             <header><span class=brand>СХІД</span>{badge}\
             <span class=tag>{}</span>\
             <nav><a href=\"/?lang={code}\">{}</a> <a href=\"/exchange?lang={code}\">{}</a>{other_link}</nav> <a class=lang href=\"?lang={other}\">{other_name}</a></header>\
             <main>{test_notice}{notice}{body}</main>\
             <footer>{}</footer></html>",
            escape(&page_title(lang, version, test)),
            page_tag(lang, version, test),
            lang.tr("Home", "Головна"),
            lang.tr("Exchange", "Обмін"),
            escape(&disclaimer),
        ),
    }
}

const STYLE: &str = ":root{color-scheme:dark}*{box-sizing:border-box}body{font:16px/1.55 system-ui,'Segoe UI',sans-serif;background:radial-gradient(circle at 50% -10%,#232a2e,#101214 60%) fixed;color:#e3e3e0;margin:0;min-height:100vh}header{display:flex;flex-wrap:wrap;align-items:center;gap:.4rem 1.2rem;padding:.9rem 1.4rem;background:#0b0c0d;border-bottom:2px solid #d4a017;box-shadow:0 2px 18px #0008}.brand{font:700 1.6rem Impact,'Arial Narrow',sans-serif;letter-spacing:.28em;color:#d4a017}.tag{font-size:.7rem;letter-spacing:.12em;color:#8a8f94;flex:1;min-width:12rem}nav a{margin-right:.9rem;color:#e3e3e0;font-size:.85rem;text-decoration:none;text-transform:uppercase;letter-spacing:.08em}nav a:hover{color:#d4a017}.lang{color:#d4a017;font-size:.85rem;text-decoration:none;border:1px solid #d4a01766;padding:.15rem .6rem;border-radius:2px}main{max-width:34rem;margin:2rem auto;padding:0 1rem}a{color:#d4a017}h1{font-size:1.3rem;text-transform:uppercase;letter-spacing:.08em;margin:.2rem 0 .8rem}h2{font-size:1rem;text-transform:uppercase;letter-spacing:.08em;color:#d4a017}.card{background:#171a1dcc;border:1px solid #2a2f34;border-top:2px solid #d4a017;border-radius:3px;padding:1rem 1.3rem;margin-bottom:1.2rem}ol{padding-left:1.3rem}.rules{padding-left:1.1rem}.opts{font-size:.85rem;color:#b4b8bb;padding-left:1.1rem}li{margin:.7rem 0}pre,code{background:#0b0c0d;border:1px solid #2a2f34;border-radius:2px;color:#e8c766;font:.85rem Consolas,monospace}pre{padding:.6rem .8rem;margin:.4rem 0;white-space:pre-wrap;word-break:break-all}.dl{display:flex;flex-direction:column;align-items:flex-start;gap:.5rem;margin:.6rem 0}.dl .button{margin:0}.hash{display:block;max-width:100%;font-size:.78rem;padding:.35rem .5rem}li>p{margin:.4rem 0}.card .note{margin:1rem 0 0}.stats{display:flex;gap:2rem;margin-top:1rem;padding-top:.8rem;border-top:1px solid #2a2f34}.stats div{display:flex;flex-direction:column}.stats b{font-size:1.6rem;color:#d4a017;line-height:1.1}.stats span{font-size:.75rem;color:#8a8f94;text-transform:uppercase;letter-spacing:.08em}code{padding:.1rem .3rem;word-break:break-all}label{display:block;margin:.9rem 0 .25rem;font-size:.85rem;color:#b4b8bb}input{width:100%;padding:.55rem .6rem;background:#0f1113;color:inherit;border:1px solid #3a3f44;border-radius:2px;font:inherit}input:focus{outline:0;border-color:#d4a017;box-shadow:0 0 0 2px #d4a01740}button,.button{display:inline-block;margin-top:1.2rem;padding:.65rem 1.5rem;background:#d4a017;color:#101214;border:0;border-radius:2px;font:700 .9rem system-ui,sans-serif;text-transform:uppercase;letter-spacing:.08em;text-decoration:none;cursor:pointer}button:hover,.button:hover{background:#e8b52a}.note{background:#1d2125;border-left:3px solid #d4a017;padding:.7rem .9rem;border-radius:2px}.note.test{border-left-color:#d64531;margin-bottom:1.2rem}.badge{background:#d64531;color:#fff;font:700 .75rem system-ui,sans-serif;letter-spacing:.14em;padding:.2rem .55rem;border-radius:2px}small{color:#8a8f94}label.check{display:flex;gap:.5rem;align-items:center;font-size:.85rem}label.check input{width:auto}.tabs{margin-top:1.2rem}.tabs>input{position:absolute;opacity:0;pointer-events:none}.tabs>label{display:inline-block;margin:0 .25rem 0 0;padding:.45rem .9rem;border:1px solid #2a2f34;border-bottom:0;border-radius:3px 3px 0 0;cursor:pointer;font-size:.78rem;text-transform:uppercase;letter-spacing:.08em;color:#b4b8bb;background:#0f1113}.tabs>input:checked+label{color:#101214;background:#d4a017;border-color:#d4a017}.tabs>input:focus-visible+label{outline:2px solid #e8c766}.panel{display:none;border:1px solid #2a2f34;border-top:2px solid #d4a017;padding:.2rem 1rem 1rem;background:#171a1dcc}#tab-money:checked~.p-money,#tab-ban:checked~.p-ban,#tab-unban:checked~.p-unban,#tab-bans:checked~.p-bans,#tab-maint:checked~.p-maint,#tab-signups:checked~.p-signups,#tab-account:checked~.p-account,#tab-item:checked~.p-item,#tab-text:checked~.p-text{display:block}button.quiet{background:0;color:#d4a017;border:1px solid #d4a01766}button.quiet:hover{background:#d4a01722}table{width:100%;border-collapse:collapse;font-size:.85rem}th,td{text-align:left;padding:.35rem .4rem;border-bottom:1px solid #2a2f34;vertical-align:top;word-break:break-word}th{color:#8a8f94;font-weight:600}details{margin-top:.8rem}summary{cursor:pointer;color:#d4a017;font-size:.8rem;text-transform:uppercase;letter-spacing:.08em}footer{max-width:34rem;margin:2rem auto;padding:1rem;border-top:1px solid #2a2f34;font-size:.75rem;color:#6d7378}";

/// A file's size as the download button shows it.
fn file_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.2} GB", bytes as f64 / 1e9)
    } else {
        format!("{:.1} MB", bytes as f64 / 1e6)
    }
}

/// `SRP_MATCH_TEAM_SIZE`: the most players of one side, 1 to half a match.
fn match_team_size() -> usize {
    let half = session::MAX_PLAYERS_PER_MATCH / 2;
    std::env::var("SRP_MATCH_TEAM_SIZE")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .filter(|team| (1..=half).contains(team))
        .unwrap_or(half)
}

/// Whether every game that can press Play plays in the growing queue (the
/// games of patch revision 5 or later): the lobby refuses the others
/// (`SRP_PATCH_REVISION_CHECK` is `refuse`, and `SRP_MIN_PATCH_REVISION` is
/// 5 or more). The lobby makes two queues of the games with and without the
/// late join; the page tells the growing one's rules only when no game is
/// left for the other.
fn only_growing_games() -> bool {
    let refuses = std::env::var("SRP_PATCH_REVISION_CHECK").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "refuse" | "on" | "1" | "true" | "yes"
        )
    });
    let minimum = std::env::var("SRP_MIN_PATCH_REVISION")
        .ok()
        .and_then(|value| value.trim().parse::<u8>().ok())
        .unwrap_or(0);
    refuses && minimum >= survarium::v026g::match_msgs::late_join::FIRST_REVISION
}

/// What the growing queue and the matches it makes are set to.
struct GrowingRules {
    /// The most players of one side.
    team: usize,
    /// The fewest players a new match starts with.
    min_players: usize,
    /// How long after that many wait a new match starts.
    gather: u64,
    /// A running match takes players until this much of it is left.
    cutoff: u64,
    /// How long a player who would make an even match uneven waits.
    patience: u64,
}

impl GrowingRules {
    fn from_env() -> Self {
        let (min_players, gather) = session::growing_start_rules_from_env();
        let patience = std::env::var("SRP_LATE_JOIN_PATIENCE_SECONDS")
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(session::matchmaking_policy::DEFAULT_LATE_JOIN_PATIENCE.as_secs());
        Self {
            team: match_team_size(),
            min_players,
            gather: gather.as_secs(),
            cutoff: session::matchmaking_policy::JOIN_CUTOFF.as_secs(),
            patience,
        }
    }
}

/// The list under "How matches start" for the growing queue: a match starts
/// small and takes players while it is played.
fn growing_rules(lang: Lang, rules: &GrowingRules, squads: bool) -> String {
    let GrowingRules {
        team,
        min_players,
        gather,
        cutoff,
        patience,
    } = *rules;
    let full = team * 2;
    let cutoff_min = cutoff / 60;
    let items = [
        lang.tr(
            &format!(
                "{min_players} players are enough to start a match: it starts {gather} s after that many are waiting. Whoever presses Play in that time goes into the same match."
            ),
            &format!(
                "Для початку матчу досить {min_players} гравців: він починається через {gather} с після того, як їх набралося стільки. Хто натисне «Грати» за цей час, потрапить у той самий матч."
            ),
        ),
        lang.tr(
            &format!("{full} players waiting: the match starts at once."),
            &format!("{full} гравців чекає: матч починається одразу."),
        ),
        lang.tr(
            &format!(
                "A match that is being played takes new players until {cutoff_min} minutes of it are left, up to {team} a side. A newcomer goes to the side that makes the teams more even."
            ),
            &format!(
                "Матч, що триває, приймає нових гравців, доки не лишиться {cutoff_min} хв до його кінця, до {team} на сторону. Новачок іде на ту сторону, де команди стають рівнішими."
            ),
        ),
        lang.tr(
            &format!(
                "A player who would make an even match uneven waits up to {patience} s for somebody to go in with."
            ),
            &format!(
                "Гравець, через якого рівний матч став би нерівним, чекає до {patience} с, щоб хтось зайшов разом із ним."
            ),
        ),
        if squads {
            lang.tr(
                &format!(
                    "Fewer than {min_players} players: no match starts until there are {min_players}."
                ),
                &format!(
                    "Менше ніж {min_players} гравці: матч не починається, доки не набереться {min_players}."
                ),
            )
        } else {
            lang.tr(
                &format!(
                    "Fewer than {min_players} players: no match starts until there are {min_players}. Squads are switched off: everybody queues alone."
                ),
                &format!(
                    "Менше ніж {min_players} гравці: матч не починається, доки не набереться {min_players}. Загони вимкнено: кожен стає в чергу сам."
                ),
            )
        },
        lang.tr(
            "The server makes the two sides as even as it can: by the players' gear, their level and their results in earlier matches. With an odd number of players, the stronger ones are the side with a player fewer.",
            "Сервер ділить гравців на дві сторони якомога рівніше: за спорядженням, рівнем і результатами попередніх матчів. Якщо гравців непарна кількість, сильніші грають на стороні, де на одного гравця менше.",
        ),
    ];
    items
        .iter()
        .map(|item| format!("<li>{item}</li>"))
        .collect()
}

fn home(lang: Lang, stats: Stats) -> String {
    let v026g = stats.v026g;
    let with_launcher = stats.launcher;
    let installs_game = with_launcher && stats.installs_game;
    let connect = CONNECT.get();
    let address = connect.map_or("127.0.0.1:1234", |connect| connect.address.as_str());
    let local = available_download().and_then(|file| std::fs::metadata(file).ok());
    let download_label = if v026g {
        lang.tr("Download the files", "Завантажити файли")
    } else {
        lang.tr("Download the game", "Завантажити гру")
    };
    let download = match connect.and_then(|connect| connect.download_url.as_deref()) {
        _ if local.is_some() => format!(
            "<a class=button href=\"/download\">{download_label} ({})</a>",
            file_size(local.as_ref().map_or(0, |meta| meta.len()))
        ),
        Some(url) => format!(
            "<a class=button href=\"{}\" rel=noopener>{download_label}</a>",
            escape(url),
        ),
        None => format!(
            "<em>{}</em>",
            lang.tr(
                "The download link is coming soon.",
                "Посилання на завантаження скоро з'явиться."
            )
        ),
    };
    let hash = match connect.and_then(|connect| connect.sha256.as_deref()) {
        Some(hash) => format!(
            "<small>SHA-256</small><code class=hash>{}<br>{}</code>",
            escape(&hash.to_ascii_lowercase()[..32]),
            escape(&hash.to_ascii_lowercase()[32..])
        ),
        None => String::new(),
    };
    // 0.26g: the game itself comes from elsewhere (step one); this server
    // hands out only the files that go over it.
    // With a download whose launcher brings the game there is no such
    // step: the link is the way out, in the small print of the download.
    let game_link = connect
        .and_then(|connect| connect.game_url.as_deref())
        .map(|url| {
            format!(
                "<a href=\"{}\" rel=noopener>{}</a>",
                escape(url),
                lang.tr("the original game", "оригінальну гру")
            )
        })
        .unwrap_or_else(|| lang.tr("the original game", "оригінальну гру").to_string());
    let original = if v026g && !installs_game {
        let link = match connect.and_then(|connect| connect.game_url.as_deref()) {
            Some(url) => format!(
                "<div class=dl><a class=button href=\"{}\" rel=noopener>{}</a></div>",
                escape(url),
                lang.tr("Get the game", "Отримати гру")
            ),
            None => format!(
                "<p><em>{}</em></p>",
                lang.tr("The link is coming soon.", "Посилання скоро з'явиться.")
            ),
        };
        format!(
            "<li>{}{link}</li>",
            lang.tr(
                "Get the original Survarium 0.26g (build 2777) and unpack it (a .7z archive: 7-Zip opens it).",
                "Отримайте оригінальну Survarium 0.26g (збірка 2777) і розпакуйте її (архів .7z: відкривається 7-Zip)."
            )
        )
    } else {
        String::new()
    };
    let mut notices = String::new();
    if let Some(minutes) = stats.maintenance {
        notices += &format!(
            "<p class=note><b>{}</b> {}</p>",
            lang.tr("Maintenance.", "Технічні роботи."),
            if minutes == 0 {
                lang.tr(
                    "The server is down for maintenance.",
                    "Сервер на технічних роботах.",
                )
            } else {
                lang.tr(
                    &format!("The game closes in {minutes} min; Play is locked in the last 30."),
                    &format!("Гра закриється через {minutes} хв; за 30 хв «Грати» вимикається."),
                )
            }
        );
    }
    if !stats.registrations_open {
        notices += &format!(
            "<p class=note>{}</p>",
            lang.tr(
                "New registrations are closed for now. Existing players can sign in and play.",
                "Нові реєстрації тимчасово закриті. Гравці з акаунтом можуть входити й грати.",
            )
        );
    }
    let stats = format!(
        "{notices}<div class=stats><div><b>{}</b><span>{}</span></div><div><b>{}</b><span>{}</span></div></div>",
        stats.online,
        lang.tr("players online", "гравців онлайн"),
        stats.registered,
        lang.tr("registered", "зареєстровано"),
    );
    let version = if v026g { "0.26g" } else { "0.21d" };
    // 0.26g: `-server=` (it has no `-client=`), and the exe that takes srp's
    // certificate (`scripts/client-patch-026g`).
    let server_key = if v026g { "-server" } else { "-client" };
    let mut options = format!(
        "<ul class=opts><li><code>{server_key}</code> {}</li><li><code>-localization</code> {}</li>",
        lang.tr("server address", "адреса сервера"),
        lang.tr(
            "language: english, russian, ukrainian",
            "мова: english, russian, ukrainian"
        ),
    );
    if v026g {
        options += &format!(
            "<li><code>-mt_allocator=768</code> {}</li><li><code>-minimum_resources_memory_size=768</code> {}</li>",
            lang.tr(
                "memory for the game (maps need more than the default), MB",
                "пам'ять для гри (мапам замало типової), МБ"
            ),
            lang.tr("memory for resources, MB", "пам'ять для ресурсів, МБ"),
        );
    }
    options += "</ul>";
    let launch = if v026g {
        format!(
            "<pre>survarium_skhid.exe -localization={} -server={} -mt_allocator=768 -minimum_resources_memory_size=768</pre><p><small>{}</small></p>",
            lang.tr("english", "ukrainian"),
            escape(address),
            lang.tr(
                "Run it from the game's <code>game\\binaries\\x86</code> folder (the download's <code>play_skhid.bat</code> does).",
                "Запускайте з теки гри <code>game\\binaries\\x86</code> (це робить <code>play_skhid.bat</code> із завантаження).",
            ),
        )
    } else {
        format!(
            "<pre>survarium.exe -localization={} -client={}</pre>",
            lang.tr("english", "ukrainian"),
            escape(address),
        )
    };
    let command_line = format!(
        "{}{launch}{options}",
        lang.tr(
            "Start it with this command line:",
            "Запустіть гру з такою командою:"
        ),
    );
    // With the launcher in the download a player starts that and types
    // nothing; the start script and the command line stay, folded away.
    let start = if with_launcher {
        format!(
            "{}<p>{}</p><p><small>{}</small></p><details><summary>{}</summary><p>{}</p>{command_line}</details>",
            if installs_game {
                lang.tr(
                    "Double-click <code>Skhid.bat</code> in that folder (Windows may show it as <b>Skhid</b>). The launcher opens. <b>The first time it gets the game itself</b>: it downloads Survarium 0.26g (2.9 GB, from Google Drive), checks it and installs it, which takes a while; no sign-in is asked for. Then choose your language at the launcher's top and press <b>PLAY</b>.",
                    "Двічі клацніть <code>Skhid.bat</code> у цій теці (Windows може показувати його як <b>Skhid</b>). Відкриється лаунчер. <b>Першого разу він сам отримує гру</b>: завантажує Survarium 0.26g (2,9 ГБ, з Google Drive), перевіряє та встановлює її, на що потрібен час; входити нікуди не треба. Потім оберіть мову вгорі вікна лаунчера й натисніть <b>PLAY</b>.",
                )
            } else {
                lang.tr(
                    "Double-click <code>Skhid.bat</code> in that folder (Windows may show it as <b>Skhid</b>). The launcher opens: choose your language at its top and press <b>PLAY</b>.",
                    "Двічі клацніть <code>Skhid.bat</code> у цій теці (Windows може показувати його як <b>Skhid</b>). Відкриється лаунчер: оберіть мову вгорі його вікна й натисніть <b>PLAY</b>.",
                )
            },
            lang.tr(
                "From then on this is all you do: each time it starts, the launcher looks for new files of ours and installs them by itself.",
                "Відтоді це все, що потрібно робити: під час кожного запуску лаунчер перевіряє, чи є нові наші файли, і сам їх встановлює.",
            ),
            if installs_game {
                lang.tr(
                    "If Windows warns about an unknown publisher, let it run: our files are not signed. On Linux, run <code>bash Skhid.sh launcher</code> in that folder the first time, so that the game is brought; <code>bash Skhid.sh</code> starts the game alone. <code>LINUX.txt</code> in the download says what it needs.",
                    "Якщо Windows попереджає про невідомого видавця, дозвольте запуск: наші файли не підписані. У Linux першого разу виконайте в цій теці <code>bash Skhid.sh launcher</code>, щоб було отримано гру; <code>bash Skhid.sh</code> запускає лише гру. Що для цього потрібно, сказано в <code>LINUX.txt</code> із завантаження.",
                )
            } else {
                lang.tr(
                    "If Windows warns about an unknown publisher, let it run: our files are not signed. On Linux, run <code>bash Skhid.sh</code> in that folder; <code>LINUX.txt</code> in the download says what it needs.",
                    "Якщо Windows попереджає про невідомого видавця, дозвольте запуск: наші файли не підписані. У Linux виконайте <code>bash Skhid.sh</code> у цій теці; що для цього потрібно, сказано в <code>LINUX.txt</code> із завантаження.",
                )
            },
            lang.tr("Without the launcher", "Без лаунчера"),
            lang.tr(
                "<code>play_skhid.bat</code> in <code>game\\binaries\\x86</code> starts the game alone, without looking for new files. If the game says it is outdated, start <code>Skhid.bat</code>: the launcher brings the new files.",
                "<code>play_skhid.bat</code> у теці <code>game\\binaries\\x86</code> запускає лише гру, без перевірки нових файлів. Якщо гра каже, що її версія застаріла, запустіть <code>Skhid.bat</code>: лаунчер сам отримає нові файли.",
            ),
        )
    } else {
        command_line
    };
    let window = session::gather_window_from_env()
        .unwrap_or(session::DEFAULT_GATHER_WINDOW)
        .as_secs();
    let fill = session::fill_from_env();
    let quiet = session::quiet_from_env().as_secs();
    let max = session::MAX_PLAYERS_PER_MATCH;
    // 0.26g's lobby can be told to refuse squads (`SRP_SQUADS`).
    let squads = !v026g
        || !matches!(
            std::env::var("SRP_SQUADS")
                .ok()
                .map(|value| value.trim().to_ascii_lowercase())
                .as_deref(),
            Some("off" | "0" | "false" | "no")
        );
    let rules = if v026g && only_growing_games() {
        growing_rules(lang, &GrowingRules::from_env(), squads)
    } else if v026g {
        let (min_players, extension, longest) = session::leveled_start_rules_from_env();
        let (extension, longest) = (extension.as_secs(), longest.as_secs());
        let team = match_team_size();
        let full = team * 2;
        let longest_min = longest / 60;
        format!(
            "<li>{}</li><li>{}</li><li>{}</li><li>{}</li><li>{}</li><li>{}</li>",
            lang.tr(
                &format!("{full} players: the match starts at once."),
                &format!("{full} гравців: матч починається одразу.")
            ),
            lang.tr(
                &format!(
                    "{fill} to {} players: it starts when nobody has pressed Play for {quiet} s.",
                    full - 1
                ),
                &format!(
                    "від {fill} до {} гравців: матч починається, коли {quiet} с ніхто не натискав «Грати».",
                    full - 1
                )
            ),
            lang.tr(
                &format!(
                    "{min_players} to {} players: a {window} s countdown starts when the {min_players}th player joins. Each time somebody presses Play, {extension} s are added to what is left.",
                    fill - 1
                ),
                &format!(
                    "від {min_players} до {} гравців: відлік {window} с починається, коли приєднується {min_players}-й гравець. Щоразу, коли хтось натискає «Грати», до залишку додається {extension} с.",
                    fill - 1
                )
            ),
            lang.tr(
                &format!("The wait never lasts longer than {longest_min} minutes from the start of the countdown."),
                &format!("Очікування ніколи не триває довше {longest_min} хв від початку відліку.")
            ),
            if squads {
                lang.tr(
                    &format!("Fewer than {min_players} players: no match starts until there are {min_players}. A squad counts as one side, so a squad alone waits too."),
                    &format!("Менше ніж {min_players} гравці: матч не починається, доки не набереться {min_players}. Загін — це одна сторона, тож загін сам теж чекає.")
                )
            } else {
                lang.tr(
                    &format!("Fewer than {min_players} players: no match starts until there are {min_players}. Squads are switched off: everybody queues alone."),
                    &format!("Менше ніж {min_players} гравці: матч не починається, доки не набереться {min_players}. Загони вимкнено: кожен стає в чергу сам.")
                )
            },
            lang.tr(
                "The server makes the two sides as even as it can: by the players' gear, their level and their results in earlier matches. With an odd number of players, the stronger ones are the side with a player fewer.",
                "Сервер ділить гравців на дві сторони якомога рівніше: за спорядженням, рівнем і результатами попередніх матчів. Якщо гравців непарна кількість, сильніші грають на стороні, де на одного гравця менше."
            ),
        )
    } else {
        format!(
            "<li>{}</li><li>{}</li><li>{}</li><li>{}</li>",
            lang.tr(
                &format!("{max} players: the match starts at once."),
                &format!("{max} гравців: матч починається одразу.")
            ),
            lang.tr(
                &format!(
                    "{fill} to {} players: it starts when nobody has pressed Play for {quiet} s, or when {window} s are up, whichever is first.",
                    max - 1
                ),
                &format!(
                    "від {fill} до {} гравців: матч починається, коли {quiet} с ніхто не натискав «Грати», або через {window} с, що настане раніше.",
                    max - 1
                )
            ),
            lang.tr(
                &format!("2 to {} players: they wait the full {window} s.", fill - 1),
                &format!("від 2 до {} гравців: чекають повні {window} с.", fill - 1)
            ),
            lang.tr(
                "One player alone: no match starts until another joins.",
                "Один гравець: матч не починається, доки не приєднається ще хтось."
            ),
        )
    };
    format!(
        "<section class=card><h1>{}</h1><p>{}</p><p>{}</p><p class=note><b>{}</b> {}</p>{stats}</section> <section class=card><h2>{}</h2><ol> {original}<li>{}<div class=dl>{download}{hash}</div></li> <li>{start}</li> <li>{}</li></ol></section> <section class=card><h2>{}</h2><ul class=rules>{rules}</ul></section> <p><a class=button href=\"/exchange?lang={}\">{}</a></p>",
        lang.tr("Skhid", "Схід"),
        if squads {
            lang.tr(
                &format!("Skhid (Ukrainian for \u{201c}east\u{201d}) is a small community server that lets the Survarium {version} client sign in, trade, form squads and play matches again."),
                &format!("Схід \u{2014} невеликий ком'юніті сервер, на якому клієнт Survarium {version} знову може входити, торгувати, збирати загони й грати матчі.")
            )
        } else {
            lang.tr(
                &format!("Skhid (Ukrainian for \u{201c}east\u{201d}) is a small community server that lets the Survarium {version} client sign in, trade and play matches again."),
                &format!("Схід \u{2014} невеликий ком'юніті сервер, на якому клієнт Survarium {version} знову може входити, торгувати й грати матчі.")
            )
        },
        lang.tr(
            "Built on the open source project SRP (Survarium Restoration Project), which gave this server its start: <a href=\"https://github.com/srp-survarium/srp\">github.com/srp-survarium/srp</a>.",
            "Створено на основі open source проєкту SRP (Survarium Restoration Project), який дав початок цьому серверу: <a href=\"https://github.com/srp-survarium/srp\">github.com/srp-survarium/srp</a>."
        ),
        lang.tr("Early test server.", "Тестовий сервер."),
        lang.tr(
            "It has bugs and may not work fully. Your progress, items and accounts can be lost or reset at any time, and the server may be stopped at any time. By playing you accept this.",
            "Можливі помилки та збої. Ваш прогрес, предмети й акаунти можуть зникнути або бути скинуті будь-коли, а сервер можуть зупинити будь-коли. Граючи, ви з цим погоджуєтесь."
        ),
        lang.tr("How to play", "Як зіграти"),
        if installs_game {
            format!(
                "{}<p><small>{}</small></p><p><small>{}</small></p>",
                lang.tr(
                    "Download our files and unpack them into a new, empty folder on a disk with about 10 GB free. <b>You do this only once:</b> the launcher in them gets the game, and later picks up updates on its own.",
                    "Завантажте наші файли й розпакуйте їх у нову порожню теку на диску, де вільно близько 10 ГБ. <b>Це робиться лише один раз:</b> лаунчер із цих файлів сам отримує гру, а далі сам отримує оновлення."
                ),
                lang.tr(
                    "Already have Survarium 0.26g (build 2777)? Unpack our files into its folder instead, the one that holds the <code>game</code> folder, and say yes when asked to replace files: nothing is downloaded again. Two of the game's files are replaced: <code>bugtrap.dll</code> (the crash reporter) and one certificate. Keep copies of them if you want the originals back; nothing else of the game changes.",
                    "Уже маєте Survarium 0.26g (збірка 2777)? Тоді розпакуйте наші файли в її теку, ту, в якій лежить тека <code>game</code>, і погодьтеся замінити файли: нічого не завантажується вдруге. Замінюються два файли гри: <code>bugtrap.dll</code> (звіти про збої) і один сертифікат. Збережіть їхні копії, якщо хочете повернути оригінали; більше в грі нічого не змінюється."
                ),
                lang.tr(
                    &format!("If the launcher cannot get the game (it says why), get {game_link} yourself, a .7z archive that 7-Zip opens, unpack it, and unpack our files into its folder as above."),
                    &format!("Якщо лаунчер не може отримати гру (він скаже чому), отримайте {game_link} самі (архів .7z, відкривається 7-Zip), розпакуйте її та розпакуйте наші файли в її теку, як сказано вище.")
                ),
            )
        } else if with_launcher {
            lang.tr(
                "Download our files and unpack them into the game's folder: the one that holds the <code>game</code> folder. Say yes when asked to replace files. <b>You do this only once:</b> later the launcher picks up updates on its own.<p><small>Two of the game's files are replaced: <code>bugtrap.dll</code> (the crash reporter) and one certificate. Keep copies of them if you want the originals back; nothing else of the game changes.</small></p>",
                "Завантажте наші файли й розпакуйте їх у теку гри: ту, в якій лежить тека <code>game</code>. Погодьтеся замінити файли. <b>Це робиться лише один раз:</b> далі лаунчер сам отримує оновлення.<p><small>Замінюються два файли гри: <code>bugtrap.dll</code> (звіти про збої) і один сертифікат. Збережіть їхні копії, якщо хочете повернути оригінали; більше в грі нічого не змінюється.</small></p>"
            )
        } else if v026g {
            lang.tr(
                "Download our files and unpack them over the game's <code>game</code> folder. They add <code>survarium_skhid.exe</code>, <code>play_skhid.bat</code> and, for Linux, <code>play_skhid.sh</code> beside the game's own exe, and replace two of the game's files: <code>bugtrap.dll</code> (the crash reporter) and one certificate. Keep copies of those two if you want the originals back; nothing else changes.",
                "Завантажте наші файли й розпакуйте їх поверх теки <code>game</code> гри. Вони додають <code>survarium_skhid.exe</code>, <code>play_skhid.bat</code> і, для Linux, <code>play_skhid.sh</code> поруч із exe гри та замінюють два файли гри: <code>bugtrap.dll</code> (звіти про збої) і один сертифікат. Збережіть копії цих двох файлів, якщо хочете повернути оригінали; більше нічого не змінюється."
            )
        } else {
            lang.tr("Download the patched game and unpack it.", "Завантажте пропатчену гру та розпакуйте її.")
        },
        lang.tr(
            "Sign in with any user name and password. A new name is registered on its first sign-in; remember the password.",
            "Увійдіть з будь-яким іменем і паролем. Нове ім'я реєструється під час першого входу; запам'ятайте пароль."
        ),
        lang.tr("How matches start", "Як починаються матчі"),
        lang.code(),
        if v026g {
            lang.tr("Exchange silver and gold", "Обміняти срібло й золото")
        } else {
            lang.tr("Exchange silver for gold", "Обміняти срібло на золото")
        },
    )
}

fn exchange_form(lang: Lang) -> String {
    let code = lang.code();
    format!(
        "<h1>{}</h1><form method=post action=/exchange autocomplete=off>\
         <input type=hidden name=lang value={code}>\
         <label for=u>{}</label><input id=u name=user required maxlength=99>\
         <label for=p>{}</label><input id=p name=password type=password required maxlength=47>\
         <label for=g>{}</label><input id=g name=gold type=number min=1 max=10000 required>\
         <small>{}</small><br><button>{}</button></form>",
        lang.tr("Silver for gold", "Срібло на золото"),
        lang.tr("User name", "Ім'я користувача"),
        lang.tr("Password", "Пароль"),
        lang.tr("Gold to buy", "Скільки золота купити"),
        lang.tr(
            "100 silver for each gold (10 000 silver = 100 gold).",
            "100 срібла за 1 золота (10 000 срібла = 100 золота)."
        ),
        lang.tr("Exchange", "Обміняти"),
    )
}

/// The admin page: the token once, then a tab for each action. The tabs are
/// radio buttons in the form (no script), so the token goes with whichever
/// tab is open, and the page comes back on that tab.
fn admin_form(lang: Lang, view: &AdminView) -> String {
    let code = lang.code();
    let tab = |name: &str, title: String| {
        let open = view.tab == name || (view.tab.is_empty() && name == "money");
        format!(
            "<input type=radio name=tab value={name} id=tab-{name}{}><label for=tab-{name}>{title}</label>",
            if open { " checked" } else { "" }
        )
    };
    let (signups_state, signups_to, signups_button) = if view.registrations_open {
        (
            lang.tr("New accounts are taken.", "Нові акаунти приймаються."),
            "closed",
            lang.tr("Stop new registrations", "Зупинити нові реєстрації"),
        )
    } else {
        (
            lang.tr(
                "New accounts are refused for now.",
                "Нові акаунти зараз не приймаються.",
            ),
            "open",
            lang.tr("Take new registrations", "Приймати нові реєстрації"),
        )
    };
    let maintenance = if view.token.is_empty() {
        String::new()
    } else {
        view.maintenance.clone()
    };
    let bans = if view.token.is_empty() {
        format!(
            "<p><small>{}</small></p>",
            lang.tr(
                "Shown once the token is right.",
                "Показуються після правильного токена."
            )
        )
    } else {
        view.bans.clone()
    };
    let (extra_tabs, extra_panels) = if view.g26 {
        admin_extras_026g(lang, view, &tab)
    } else {
        (String::new(), String::new())
    };
    format!(
        "<h1>{}</h1><form method=post action=\"\" autocomplete=off>\
         <input type=hidden name=lang value={code}>\
         <label for=t>{}</label><input id=t name=token type=password required value=\"{}\">\
         <div class=tabs>{}{}{}{}{}{}{extra_tabs}\
         <div class=\"panel p-money\">\
         <label for=u>{}</label><input id=u name=user maxlength=99>\
         <label for=s>{}</label><input id=s name=silver placeholder=+10000>\
         <label for=g>{}</label><input id=g name=gold placeholder=+500>\
         <button>{}</button></div>\
         <div class=\"panel p-ban\">\
         <label for=b>{}</label><input id=b name=ban_user maxlength=99>\
         <label for=h>{}</label><input id=h name=hours inputmode=numeric placeholder=24>\
         <label for=r>{}</label><input id=r name=reason maxlength=200>\
         <label class=check><input type=checkbox name=with_ip checked> {}</label>\
         <button>{}</button></div>\
         <div class=\"panel p-unban\">\
         <label for=n>{}</label><input id=n name=unban_user maxlength=99>\
         <label class=check><input type=checkbox name=unban_ip> {}</label>\
         <button>{}</button></div>\
         <div class=\"panel p-bans\">{bans}<button>{}</button></div>\
         <div class=\"panel p-signups\"><p class=note>{signups_state}</p>\
         <small>{}</small><br>\
         <button name=signups value={signups_to}>{signups_button}</button></div>\
         <div class=\"panel p-maint\">{maintenance}\
         <label for=m>{}</label><input id=m name=minutes inputmode=numeric placeholder=30>\
         <small>{}</small><br>\
         <button>{}</button> <button name=cancel value=1 class=quiet>{}</button></div>\
         {extra_panels}</div></form>",
        lang.tr("Admin", "Адміністратор"),
        lang.tr("Token", "Токен"),
        escape(&view.token),
        tab("money", lang.tr("Money", "Гроші")),
        tab("ban", lang.tr("Ban", "Бан")),
        tab("unban", lang.tr("Unban", "Розбан")),
        tab("bans", lang.tr("Bans", "Бани")),
        tab("maint", lang.tr("Maintenance", "Роботи")),
        tab("signups", lang.tr("Signups", "Реєстрація")),
        lang.tr("User name", "Ім'я користувача"),
        lang.tr("Silver, + or -", "Срібло, + або -"),
        lang.tr("Gold, + or -", "Золото, + або -"),
        lang.tr("Change", "Змінити"),
        lang.tr(
            "User name, or an address or range (1.2.3.4, 1.2.3.0/24)",
            "Ім'я, або адреса чи діапазон (1.2.3.4, 1.2.3.0/24)"
        ),
        lang.tr("Hours (empty: for good)", "Години (порожньо — назавжди)"),
        lang.tr(
            "Reason (only admins see it)",
            "Причина (бачать лише адміни)"
        ),
        lang.tr(
            "Also ban the address it last signed in from",
            "Також заблокувати адресу, з якої він входив востаннє"
        ),
        lang.tr("Ban", "Заблокувати"),
        lang.tr(
            "User name, or an address or range",
            "Ім'я, або адреса чи діапазон"
        ),
        lang.tr(
            "Also lift the ban of its last address",
            "Також розблокувати його останню адресу"
        ),
        lang.tr("Unban", "Розблокувати"),
        lang.tr("Refresh", "Оновити"),
        lang.tr(
            "Existing players keep signing in and playing either way. Registration is open by default; a restart goes back to that (or to SRP_REGISTRATION=closed).",
            "Гравці з акаунтом входять і грають у будь-якому разі. За замовчуванням реєстрація відкрита; перезапуск повертає це (або SRP_REGISTRATION=closed)."
        ),
        lang.tr("Starts in (minutes)", "Почнуться через (хвилин)"),
        lang.tr(
            "Players see the minutes left from 60 on, cannot press Play from 30, and the game closes at 0. Restarting srp cancels it.",
            "Гравці бачать залишок хвилин за годину, не можуть натиснути «Грати» за 30, а гра закривається на 0. Перезапуск srp скасовує."
        ),
        lang.tr("Plan", "Запланувати"),
        lang.tr("Cancel", "Скасувати"),
    )
}

/// The 0.26g admin's own tabs and panels: the account, an item, the text.
fn admin_extras_026g(
    lang: Lang,
    view: &AdminView,
    tab: &dyn Fn(&str, String) -> String,
) -> (String, String) {
    let tabs = format!(
        "{}{}{}",
        tab("account", lang.tr("Account", "Акаунт")),
        tab("item", lang.tr("Item", "Предмет")),
        tab("text", lang.tr("Text", "Текст")),
    );
    let standing = match &view.admin_text {
        Some((first, second)) => format!(
            "<p class=note>{}<br>{}<br>{}</p>",
            lang.tr("Shown now:", "Зараз показано:"),
            escape(first),
            escape(second)
        ),
        None if view.token.is_empty() => String::new(),
        None => format!(
            "<p><small>{}</small></p>",
            lang.tr("No text is shown.", "Текст не показано.")
        ),
    };
    let max = v026g::MAX_TEXT_LEN;
    let panels = format!(
        "<div class=\"panel p-account\">\
         <label for=au>{}</label><input id=au name=acc_user maxlength=99>\
         <label for=ph>{}</label><input id=ph name=premium_hours placeholder=+24>\
         <label for=lv>{}</label><input id=lv name=level inputmode=numeric>\
         <label for=xp>{}</label><input id=xp name=experience placeholder=+500>\
         <label for=fa>{}</label><input id=fa name=faction inputmode=numeric>\
         <label for=rp>{}</label><input id=rp name=reputation placeholder=+100>\
         <small>{}</small><br><button>{}</button></div>\
         <div class=\"panel p-item\">\
         <label for=iu>{}</label><input id=iu name=item_user maxlength=99>\
         <label for=it>{}</label><input id=it name=item maxlength=99>\
         <label for=ic>{}</label><input id=ic name=count inputmode=numeric placeholder=1>\
         <button>{}</button></div>\
         <div class=\"panel p-text\">{standing}\
         <label for=l1>{}</label><input id=l1 name=line1 maxlength={max}>\
         <label for=l2>{}</label><input id=l2 name=line2 maxlength={max}>\
         <small>{}</small><br>\
         <button>{}</button> <button name=clear_text value=1 class=quiet>{}</button></div>",
        lang.tr("User name", "Ім'я користувача"),
        lang.tr("Premium hours, + or -", "Години преміуму, + або -"),
        lang.tr("Set the level (0 to 50)", "Встановити рівень (0–50)"),
        lang.tr("Experience, + or -", "Досвід, + або -"),
        lang.tr("Faction (1 to 4)", "Фракція (1–4)"),
        lang.tr("Reputation with it, + or -", "Репутація в ній, + або -"),
        lang.tr(
            "Empty fields change nothing. An open game is sent the account again.",
            "Порожні поля нічого не змінюють. Відкрита гра отримає акаунт знову."
        ),
        lang.tr("Change", "Змінити"),
        lang.tr("User name", "Ім'я користувача"),
        lang.tr(
            "Item: its id in the 0.26g dictionary, or its exact name",
            "Предмет: id у словнику 0.26g або точна назва"
        ),
        lang.tr(
            "Count (rounds for ammo; up to 8 of anything else)",
            "Кількість (набої для боєприпасів; до 8 для решти)"
        ),
        lang.tr("Give", "Видати"),
        lang.tr("First line", "Перший рядок"),
        lang.tr("Second line", "Другий рядок"),
        lang.tr(
            "Everyone in the lobby sees it, and whoever signs in, until it is cleared or srp restarts.",
            "Його бачать усі в лобі й ті, хто входить, доки його не приберуть або srp не перезапустять."
        ),
        lang.tr("Show", "Показати"),
        lang.tr("Clear", "Прибрати"),
    );
    (tabs, panels)
}

/// `HH:MM` (UTC) of a Unix time.
fn clock_utc(at: u64) -> String {
    let seconds = at % 86_400;
    format!("{:02}:{:02}", seconds / 3600, seconds / 60 % 60)
}

/// What is planned, as a line for the admin and the home page.
fn maintenance_line(lang: Lang, store: &SessionStore) -> String {
    let Some(minutes) = store.minutes_to_maintenance() else {
        return format!(
            "<p><small>{}</small></p>",
            lang.tr(
                "No maintenance is planned.",
                "Технічних робіт не заплановано."
            )
        );
    };
    let at = store.maintenance_at().unwrap_or_default();
    let text = if minutes == 0 {
        lang.tr(
            "Maintenance has started: the game closes for everyone who signs in.",
            "Технічні роботи почалися: гра закривається в усіх, хто входить.",
        )
    } else {
        lang.tr(
            &format!(
                "Maintenance starts in {minutes} min, at {} UTC.",
                clock_utc(at)
            ),
            &format!(
                "Технічні роботи почнуться через {minutes} хв, о {} UTC.",
                clock_utc(at)
            ),
        )
    };
    format!("<p class=note>{}</p>", escape(&text))
}

/// The bans in force, as two tables.
fn bans_table(lang: Lang, store: &SessionStore) -> String {
    let (accounts, nets) = store.bans();
    let until = |ban: &Ban| match ban.until {
        Some(until) => {
            let hours = until.saturating_sub(unix_now()).div_ceil(3600);
            lang.tr(&format!("{hours} h left"), &format!("ще {hours} год"))
        }
        None => lang.tr("for good", "назавжди"),
    };
    let none = format!("<p><small>{}</small></p>", lang.tr("None.", "Немає."));
    let mut html = format!("<h2>{}</h2>", lang.tr("Accounts", "Акаунти"));
    if accounts.is_empty() {
        html += &none;
    } else {
        html += &format!(
            "<table><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr>",
            lang.tr("Account", "Акаунт"),
            lang.tr("Until", "Доки"),
            lang.tr("Reason", "Причина"),
            lang.tr("Last address", "Остання адреса"),
        );
        for (_, login, name, ban, ip) in &accounts {
            html += &format!(
                "<tr><td>{}<br><small>{}</small></td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(name),
                escape(login),
                until(ban),
                escape(&ban.reason),
                ip.map(|ip| ip.to_string()).unwrap_or_default(),
            );
        }
        html += "</table>";
    }
    html += &format!("<h2>{}</h2>", lang.tr("Addresses", "Адреси"));
    if nets.is_empty() {
        html += &none;
    } else {
        html += &format!(
            "<table><tr><th>{}</th><th>{}</th><th>{}</th></tr>",
            lang.tr("Address", "Адреса"),
            lang.tr("Until", "Доки"),
            lang.tr("Reason", "Причина"),
        );
        for ban in &nets {
            html += &format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                ban.net,
                until(&ban.ban),
                escape(&ban.ban.reason),
            );
        }
        html += "</table>";
    }
    html
}

/// The wall clock, Unix seconds.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web(token: Option<&str>) -> Web {
        Web {
            store: Arc::new(SessionStore::new()),
            token: token.map(str::to_owned),
            admin_path: Some("/hidden".into()),
            admin_ips: vec![
                Net::parse("127.0.0.1").unwrap(),
                Net::parse("10.1.0.0/16").unwrap(),
            ],
            trust_proxy: false,
            token_failures: Mutex::new((0, None)),
            connections: AtomicU32::new(0),
            downloads: AtomicU32::new(0),
            resource_downloads: AtomicU32::new(0),
            game: None,
        }
    }

    fn post(path: &str, form: &[(&str, &str)]) -> Request {
        Request {
            method: "POST".into(),
            path: path.into(),
            lang: Lang::En,
            forwarded: String::new(),
            range: String::new(),
            client: Some("127.0.0.1".parse().unwrap()),
            form: form
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    fn get(path: &str) -> Request {
        Request {
            method: "GET".into(),
            form: Vec::new(),
            ..post(path, &[])
        }
    }

    fn balances(web: &Web, user: &str) -> (u32, u32) {
        let id = web.store.account_id_by_name(user).unwrap();
        let account = web.store.account(id).unwrap();
        let account = locked(&account);
        (account.game.generic_money, account.game.premium_money)
    }

    #[test]
    fn silver_becomes_gold_for_the_right_password_only() {
        let web = web(None);
        web.store.sign_in("fighter", b"secret").unwrap();
        let (silver, gold) = balances(&web, "fighter");

        let wrong = web.exchange(
            &post(
                "/exchange",
                &[("user", "fighter"), ("password", "nope"), ("gold", "1")],
            ),
            "t",
        );
        assert!(wrong.body.contains("Wrong user name or password"));
        assert_eq!(balances(&web, "fighter"), (silver, gold));

        let unknown = web.exchange(
            &post(
                "/exchange",
                &[("user", "ghost"), ("password", "x"), ("gold", "1")],
            ),
            "t",
        );
        assert!(unknown.body.contains("Wrong user name or password"));
        assert!(web.store.account_id_by_name("ghost").is_none());

        let done = web.exchange(
            &post(
                "/exchange",
                &[("user", "Fighter"), ("password", "secret"), ("gold", "2")],
            ),
            "t",
        );
        assert!(done.body.contains("Done"), "{}", done.body);
        assert_eq!(balances(&web, "fighter"), (silver - 200, gold + 2));
    }

    #[test]
    fn an_exchange_cannot_spend_silver_that_is_not_there() {
        let web = web(None);
        web.store.sign_in("poor", b"secret").unwrap();
        let before = balances(&web, "poor");
        let answer = web.exchange(
            &post(
                "/exchange",
                &[("user", "poor"), ("password", "secret"), ("gold", "10000")],
            ),
            "t",
        );
        assert!(answer.body.contains("Not enough silver"), "{}", answer.body);
        assert_eq!(balances(&web, "poor"), before);
        let zero = web.exchange(
            &post(
                "/exchange",
                &[("user", "poor"), ("password", "secret"), ("gold", "0")],
            ),
            "t",
        );
        assert!(zero.body.contains("Gold must be"));
    }

    #[test]
    fn the_admin_needs_its_token_and_changes_balances() {
        let web = web(Some("tok"));
        web.store.sign_in("tester", b"secret").unwrap();
        let (silver, gold) = balances(&web, "tester");
        let refused = web.admin(
            &post(
                "/hidden",
                &[("token", "bad"), ("user", "tester"), ("gold", "+9")],
            ),
            "t",
        );
        assert!(refused.body.contains("Wrong token"));
        assert_eq!(balances(&web, "tester"), (silver, gold));

        let done = web.admin(
            &post(
                "/hidden",
                &[
                    ("token", "tok"),
                    ("user", "tester"),
                    ("gold", "+500"),
                    ("silver", "-999999999"),
                ],
            ),
            "t",
        );
        assert!(done.body.contains("now 0 silver"), "{}", done.body);
        assert_eq!(balances(&web, "tester"), (0, gold + 500));
    }

    #[test]
    fn the_admin_bans_from_its_tabs_with_the_token_given_once() {
        let web = web(Some("tok"));
        let id = web.store.sign_in("cheater@x.test", b"secret").unwrap();
        web.store.signed_in_from(id, "203.0.113.9".parse().unwrap());
        let banned = web.admin(
            &post(
                "/hidden",
                &[
                    ("token", "tok"),
                    ("tab", "ban"),
                    ("ban_user", "cheater"),
                    ("hours", ""),
                    ("reason", "<aimbot>"),
                    ("with_ip", "on"),
                ],
            ),
            "t",
        );
        assert!(
            banned.body.contains("cheater banned for good"),
            "{}",
            banned.body
        );
        assert!(banned.body.contains("203.0.113.9 too"));
        // The page comes back on its tab, with the token kept.
        assert!(banned.body.contains("value=ban id=tab-ban checked"));
        assert!(banned.body.contains("value=\"tok\""));
        // The list escapes what it shows.
        assert!(banned.body.contains("&lt;aimbot&gt;"));
        assert_eq!(
            web.store.sign_in("cheater@x.test", b"secret"),
            Err(SignInError::Banned)
        );
        assert!(web.store.ip_ban("203.0.113.9".parse().unwrap()).is_some());

        let lifted = web.admin(
            &post(
                "/hidden",
                &[
                    ("token", "tok"),
                    ("tab", "unban"),
                    ("unban_user", "cheater@x.test"),
                    ("unban_ip", "on"),
                ],
            ),
            "t",
        );
        assert!(
            lifted.body.contains("cheater@x.test unbanned"),
            "{}",
            lifted.body
        );
        assert_eq!(web.store.sign_in("cheater@x.test", b"secret"), Ok(id));
        assert!(web.store.ip_ban("203.0.113.9".parse().unwrap()).is_none());

        // A wrong token keeps nothing.
        let refused = web.admin(&post("/hidden", &[("token", "bad"), ("tab", "bans")]), "t");
        assert!(!refused.body.contains("value=\"bad\""));
    }

    #[test]
    fn the_admin_plans_maintenance_and_closes_registration() {
        let web = web(Some("tok"));
        let act = |fields: &[(&str, &str)]| {
            let mut all = vec![("token", "tok")];
            all.extend_from_slice(fields);
            web.admin(&post("/hidden", &all), "t")
        };
        let planned = act(&[("tab", "maint"), ("minutes", "45")]);
        assert!(
            planned.body.contains("starts in 45 min"),
            "{}",
            planned.body
        );
        assert_eq!(web.store.minutes_to_maintenance(), Some(45));
        let bad = act(&[("tab", "maint"), ("minutes", "0")]);
        assert!(bad.body.contains("a whole number from 1"));
        assert_eq!(web.store.minutes_to_maintenance(), Some(45));
        // The home page says so.
        let home = web.route(&get("/"), "t");
        assert!(home.body.contains("closes in 4"), "{}", home.body);
        let cancelled = act(&[("tab", "maint"), ("cancel", "1")]);
        assert!(cancelled.body.contains("Maintenance cancelled"));
        assert_eq!(web.store.maintenance_at(), None);

        assert!(web.store.registrations_open());
        let closed = act(&[("tab", "signups"), ("signups", "closed")]);
        assert!(closed.body.contains("refused"), "{}", closed.body);
        assert!(!web.store.registrations_open());
        assert!(
            web.route(&get("/"), "t")
                .body
                .contains("registrations are closed")
        );
        act(&[("tab", "signups"), ("signups", "open")]);
        assert!(web.store.registrations_open());
    }

    #[test]
    fn wrong_tokens_lock_the_admin_for_a_while() {
        let web = web(Some("tok"));
        web.store.sign_in("tester", b"secret").unwrap();
        for _ in 0..MAX_TOKEN_FAILURES {
            web.admin(&post("/hidden", &[("token", "bad")]), "t");
        }
        let locked = web.admin(
            &post(
                "/hidden",
                &[("token", "tok"), ("user", "tester"), ("gold", "1")],
            ),
            "t",
        );
        assert!(locked.body.contains("wait a minute"));
    }

    #[test]
    fn the_admin_page_is_for_its_addresses_and_its_path_only() {
        let web = web(Some("tok"));
        let get = |path: &str, client: &str| {
            web.route(
                &Request {
                    method: "GET".into(),
                    path: path.into(),
                    lang: Lang::En,
                    forwarded: String::new(),
                    range: String::new(),
                    client: client.parse().ok(),
                    form: Vec::new(),
                },
                "t",
            )
        };
        assert_eq!(get("/hidden", "127.0.0.1").status, "200 OK");
        assert_eq!(get("/hidden", "10.1.200.3").status, "200 OK");
        assert_eq!(get("/hidden", "203.0.113.5").status, "404 Not Found");
        assert_eq!(get("/hidden", "10.2.0.1").status, "404 Not Found");
        assert_eq!(get("/admin", "127.0.0.1").status, "404 Not Found");
        assert_eq!(get("/hidden", "::1").status, "404 Not Found");
    }

    #[test]
    fn networks_parse_and_match() {
        let net = Net::parse("192.168.0.0/16").unwrap();
        assert!(net.contains("192.168.9.9".parse().unwrap()));
        assert!(!net.contains("192.169.0.1".parse().unwrap()));
        assert!(
            Net::parse("0.0.0.0/0")
                .unwrap()
                .contains("8.8.8.8".parse().unwrap())
        );
        assert!(Net::parse("::1").unwrap().contains("::1".parse().unwrap()));
        assert!(Net::parse("1.2.3.4/33").is_none() && Net::parse("nope").is_none());
    }

    #[test]
    fn the_admin_page_does_not_exist_without_a_token() {
        let web = web(None);
        let request = Request {
            method: "GET".into(),
            path: "/hidden".into(),
            lang: Lang::En,
            forwarded: String::new(),
            range: String::new(),
            client: Some("127.0.0.1".parse().unwrap()),
            form: Vec::new(),
        };
        assert_eq!(web.route(&request, "t").status, "404 Not Found");
    }

    #[test]
    fn the_language_is_ukrainian_or_english_only() {
        assert_eq!(Lang::pick("uk", "en"), Lang::Uk);
        assert_eq!(Lang::pick("en", "uk"), Lang::En);
        assert_eq!(Lang::pick("de", "uk-UA,uk;q=0.9"), Lang::Uk);
        assert_eq!(Lang::pick("", "ru-RU,ru"), Lang::En);
        assert_eq!(Lang::pick("", ""), Lang::En);
    }

    fn live_growing_rules() -> GrowingRules {
        GrowingRules {
            team: 10,
            min_players: 2,
            gather: 120,
            cutoff: 120,
            patience: 60,
        }
    }

    #[test]
    fn the_growing_queue_is_told_as_it_is() {
        let en = growing_rules(Lang::En, &live_growing_rules(), false);
        assert!(en.contains(
            "2 players are enough to start a match: it starts 120 s after that many are waiting."
        ));
        assert!(en.contains("20 players waiting: the match starts at once."));
        assert!(en.contains("until 2 minutes of it are left, up to 10 a side"));
        assert!(en.contains("waits up to 60 s"));
        assert!(en.contains(
            "Fewer than 2 players: no match starts until there are 2. Squads are switched off"
        ));
        // Nothing of the standard queue's rules is left in it.
        assert!(!en.contains("countdown"));
        assert!(!en.contains("nobody has pressed Play"));
        assert!(!en.contains('4'), "{en}");

        let squads = growing_rules(Lang::En, &live_growing_rules(), true);
        assert!(!squads.contains("Squads are switched off"));

        let uk = growing_rules(Lang::Uk, &live_growing_rules(), false);
        assert!(uk.contains("Для початку матчу досить 2 гравців"));
        assert!(uk.contains("Загони вимкнено"));
        assert_eq!(en.matches("<li>").count(), uk.matches("<li>").count());
    }

    #[test]
    fn the_growing_queue_follows_its_settings() {
        let rules = GrowingRules {
            team: 8,
            min_players: 4,
            gather: 90,
            cutoff: 180,
            patience: 30,
        };
        let en = growing_rules(Lang::En, &rules, true);
        assert!(en.contains("4 players are enough to start a match: it starts 90 s after"));
        assert!(en.contains("16 players waiting"));
        assert!(en.contains("until 3 minutes of it are left, up to 8 a side"));
        assert!(en.contains("waits up to 30 s"));
    }

    #[test]
    fn a_test_server_says_so_on_every_page() {
        let plain = page_as(Lang::En, Form::Exchange, "", false, None).body;
        assert!(plain.contains("<title>Skhid · Survarium 0.21d community server</title>"));
        assert!(!plain.contains("class=badge>"));
        assert!(!plain.contains("Test server."));

        let url = Some("https://skhid.link");
        for form in [Form::Exchange, Form::Exchange26, Form::None26] {
            let test = page_as(Lang::En, form, "", true, url).body;
            assert!(test.contains(" TEST server</title>"), "{test}");
            assert!(test.contains("<span class=badge>TEST</span>"));
            assert!(test.contains("TEST SERVER FOR SURVARIUM 0."));
            assert!(test.contains("<b>Test server.</b>"));
            assert!(test.contains("accounts and progress here can be reset"));
            // The other server is the players', named in the notice and
            // not as the other game version in the header.
            assert!(test.contains(
                "The players' server: <a href=\"https://skhid.link/?lang=en\">skhid.link</a>."
            ));
            assert!(!test.contains(">Survarium 0.21d</a>"));
            assert!(!test.contains(">Survarium 0.26g</a>"));
        }
        let uk = page_as(Lang::Uk, Form::None26, "", true, None).body;
        assert!(uk.contains("<span class=badge>ТЕСТ</span>"));
        assert!(uk.contains("<b>Тестовий сервер.</b>"));
        assert!(!uk.contains("Сервер для гравців:"));
        // A server that is not one links the other version as before.
        let live = page_as(Lang::En, Form::None26, "", false, url).body;
        assert!(live.contains(" <a href=\"https://skhid.link/?lang=en\">Survarium 0.21d</a>"));
    }

    #[test]
    fn the_other_server_is_linked_only_by_a_plain_https_address() {
        assert_eq!(
            other_url(" https://026g.skhid.link/ ").as_deref(),
            Some("https://026g.skhid.link")
        );
        assert_eq!(other_url("http://skhid.link"), None);
        assert_eq!(other_url("https://"), None);
        assert_eq!(other_url("https://a.b/\"><script>"), None);
        assert_eq!(other_url("https://a.b/?x=1"), None);
        assert_eq!(other_server_link(Lang::Uk, "0.26g", None), "");
        assert_eq!(
            other_server_link(Lang::Uk, "0.26g", Some("https://026g.skhid.link")),
            " <a href=\"https://026g.skhid.link/?lang=uk\">Survarium 0.26g</a>"
        );
        // Without the setting the pages are as they were.
        assert!(
            !page(Lang::En, Form::Exchange, "")
                .body
                .contains("Survarium 0.26g</a>")
        );
    }

    #[test]
    fn pages_are_branded_unofficial_and_localized() {
        let en = page(Lang::En, Form::Exchange, "").body;
        let uk = page(Lang::Uk, Form::Exchange, "").body;
        assert!(en.contains("СХІД") && en.contains("SKHID") && en.contains("unofficial"));
        assert!(uk.contains("СХІД") && uk.contains("Неофіційний ком'юніті"));
        assert!(uk.contains("Срібло на золото") && en.contains("Silver for gold"));
    }

    #[test]
    fn a_range_is_read_against_the_file_size() {
        assert_eq!(parse_range("", 100), Some(None));
        assert_eq!(parse_range("bytes=0-", 100), Some(Some((0, 99))));
        assert_eq!(parse_range("bytes=10-19", 100), Some(Some((10, 19))));
        assert_eq!(parse_range("bytes=90-500", 100), Some(Some((90, 99))));
        assert_eq!(parse_range("bytes=-10", 100), Some(Some((90, 99))));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=50-10", 100), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), Some(None));
        assert_eq!(parse_range("items=1-2", 100), Some(None));
        assert_eq!(parse_range("bytes=x-y", 100), None);
    }

    #[test]
    fn at_most_five_downloads_run_at_once_and_the_slot_comes_back() {
        let counter = AtomicU32::new(0);
        let mut slots = Vec::new();
        for _ in 0..MAX_DOWNLOADS {
            assert!(counter.fetch_add(1, Ordering::AcqRel) < MAX_DOWNLOADS);
            slots.push(Slot(&counter));
        }
        assert!(counter.fetch_add(1, Ordering::AcqRel) >= MAX_DOWNLOADS);
        counter.fetch_sub(1, Ordering::AcqRel);
        slots.pop();
        assert_eq!(counter.load(Ordering::Acquire), MAX_DOWNLOADS - 1);
        drop(slots);
        assert_eq!(counter.load(Ordering::Acquire), 0);
    }

    const BLOB: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn a_blob_is_named_by_its_checksum_and_nothing_else() {
        let download = Path::new("/srv/download/files.zip");
        let blob = format!("/resources/{BLOB}");
        assert_eq!(
            resource_file(&blob, download),
            Some(PathBuf::from("/srv/download/resources").join(BLOB))
        );
        for path in [
            "/resources/".to_owned(),
            "/resources".to_owned(),
            format!("/resources/{}", &BLOB[1..]),
            format!("/resources/{BLOB}0"),
            format!("/resources/{BLOB}/"),
            format!("/resources/{}", BLOB.to_ascii_uppercase()),
            format!("/resources/{}g", &BLOB[1..]),
            format!("/resources/../{}", &BLOB[3..]),
            format!("/resources/%2e%2e%2f{}", &BLOB[9..]),
            format!("/resources//{}", &BLOB[1..]),
            format!("/Resources/{BLOB}"),
            format!("/download/{BLOB}"),
            format!("{blob}.zip"),
            "/resources/..".to_owned(),
        ] {
            assert_eq!(resource_file(&path, download), None, "{path}");
        }
        // Only a GET or a HEAD of the 0.26g page, with a download file.
        let get_blob = get(&blob);
        assert!(resource_request(true, &get_blob, Some(download)).is_some());
        let head = Request {
            method: "HEAD".into(),
            ..get(&blob)
        };
        assert!(resource_request(true, &head, Some(download)).is_some());
        assert_eq!(resource_request(false, &get_blob, Some(download)), None);
        assert_eq!(resource_request(true, &get_blob, None), None);
        assert_eq!(
            resource_request(true, &post(&blob, &[]), Some(download)),
            None
        );
        assert_eq!(
            resource_request(true, &get("/download"), Some(download)),
            None
        );
    }

    /// What `answer` writes to a connection, as a client reads it.
    fn answered(answer: impl FnOnce(&mut TcpStream)) -> Vec<u8> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            let mut got = Vec::new();
            stream.read_to_end(&mut got).unwrap();
            got
        });
        let (mut stream, _) = listener.accept().unwrap();
        answer(&mut stream);
        drop(stream);
        client.join().unwrap()
    }

    #[test]
    fn a_blob_is_handed_out_as_the_download_is_in_slots_of_its_own() {
        let dir = std::env::temp_dir().join(format!("srp-web-resources-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("resources")).unwrap();
        let download = dir.join("files.zip");
        std::fs::write(&download, b"the archive").unwrap();
        let file = resource_file(&format!("/resources/{BLOB}"), &download).unwrap();
        std::fs::write(&file, b"abc").unwrap();
        let web = web(None);
        let request = get(&format!("/resources/{BLOB}"));
        let whole = answered(|stream| web.resource(stream, &request, &file));
        assert_eq!(
            String::from_utf8(whole).unwrap(),
            format!(
                "HTTP/1.0 200 OK\r\nContent-Type: application/octet-stream\r\n\
                 Content-Disposition: attachment; filename=\"{BLOB}\"\r\n\
                 Accept-Ranges: bytes\r\nContent-Length: 3\r\n\
                 X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\nabc"
            )
        );
        // HEAD: the head alone. A range: that part.
        let head = Request {
            method: "HEAD".into(),
            ..get(&format!("/resources/{BLOB}"))
        };
        let answer = answered(|stream| web.resource(stream, &head, &file));
        assert!(answer.ends_with(
            b"Content-Length: 3\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n"
        ));
        let part = Request {
            range: "bytes=1-".into(),
            ..get(&format!("/resources/{BLOB}"))
        };
        let answer =
            String::from_utf8(answered(|stream| web.resource(stream, &part, &file))).unwrap();
        assert!(
            answer.starts_with("HTTP/1.0 206 Partial Content\r\n"),
            "{answer}"
        );
        assert!(
            answer.contains("Content-Range: bytes 1-2/3\r\n") && answer.ends_with("\r\n\r\nbc")
        );
        // A name of the pattern with no file, or a folder: not found.
        let missing = dir.join("resources").join(BLOB.replace('a', "0"));
        for path in [&missing, &dir.join("resources")] {
            let answer = answered(|stream| web.resource(stream, &request, path));
            assert_eq!(
                answer,
                b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
        }
        // Its slots are its own: full, it is busy, and the archive is not.
        web.resource_downloads
            .store(MAX_DOWNLOADS, Ordering::Release);
        let answer = answered(|stream| web.resource(stream, &request, &file));
        assert!(answer.starts_with(b"HTTP/1.0 503 Service Unavailable\r\nRetry-After: 60\r\n"));
        let archive = answered(|stream| web.download(stream, &get("/download"), &download));
        assert_eq!(
            String::from_utf8(archive).unwrap(),
            "HTTP/1.0 200 OK\r\nContent-Type: application/zip\r\n\
             Content-Disposition: attachment; filename=\"files.zip\"\r\n\
             Accept-Ranges: bytes\r\nContent-Length: 11\r\n\
             X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\nthe archive"
        );
        assert_eq!(web.downloads.load(Ordering::Acquire), 0);
        assert_eq!(
            web.resource_downloads.load(Ordering::Acquire),
            MAX_DOWNLOADS
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_home_page_counts_registered_and_online_players() {
        let web = web(None);
        let count = |web: &Web| {
            let body = web
                .route(
                    &Request {
                        method: "GET".into(),
                        path: "/".into(),
                        lang: Lang::En,
                        forwarded: String::new(),
                        range: String::new(),
                        client: None,
                        form: Vec::new(),
                    },
                    "t",
                )
                .body;
            body.split("<div class=stats>")
                .nth(1)
                .unwrap()
                .split("</div></div>")
                .next()
                .unwrap()
                .replace("</b><span>", " ")
                .replace("</span></div><div><b>", " | ")
                .replace("<div><b>", "")
        };
        assert!(count(&web).starts_with("0 players online | 0 registered"));
        let first = web.store.sign_in("a", b"secret").unwrap();
        web.store.sign_in("b", b"secret").unwrap();
        let session = web.store.create_session(first);
        assert!(web.store.hold(session).is_some());
        let shown = count(&web);
        assert!(shown.contains("| 2 registered"), "{shown}");
    }

    #[test]
    fn the_home_page_says_how_to_start_what_the_download_holds() {
        let stats = |launcher| Stats {
            registered: 0,
            online: 0,
            maintenance: None,
            registrations_open: true,
            v026g: true,
            launcher,
            installs_game: false,
        };
        // With the launcher: one file to start, nothing to type, and the
        // download is needed once. The script and the command line are
        // folded away below.
        for (lang, press, once, folded) in [
            (
                Lang::En,
                "press <b>PLAY</b>",
                "You do this only once",
                "Without the launcher",
            ),
            (
                Lang::Uk,
                "натисніть <b>PLAY</b>",
                "лише один раз",
                "Без лаунчера",
            ),
        ] {
            let page = home(lang, stats(true));
            let steps = page.split("<ol>").nth(1).unwrap();
            let (shown, hidden) = steps.split_once("<details>").unwrap();
            assert!(shown.contains("<code>Skhid.bat</code>"), "{shown}");
            assert!(shown.contains(press), "{shown}");
            assert!(shown.contains(once), "{shown}");
            assert!(!shown.contains("binaries"), "{shown}");
            // Where to unpack is said by what the folder holds, not by a
            // name a player may read as "inside game".
            assert!(!shown.contains("over the game's"), "{shown}");
            assert!(!shown.contains("поверх теки"), "{shown}");
            assert!(!shown.contains("<pre>"), "{shown}");
            assert!(!shown.contains("play_skhid.bat"), "{shown}");
            assert!(hidden.contains(&format!("<summary>{folded}</summary>")));
            assert!(hidden.contains("play_skhid.bat"));
            assert!(hidden.contains("<pre>survarium_skhid.exe -localization="));
            assert_eq!(steps.matches("<details>").count(), 1);
            assert_eq!(steps.matches("</details>").count(), 1);
        }
        // A download whose launcher brings the game: no step that sends a
        // player for the game first; an empty folder, the room it needs, and
        // that the first start downloads. The game's own archive is named
        // only as the way out, and the folder of a game a player has already.
        for (lang, empty, first, have, way_out) in [
            (
                Lang::En,
                "into a new, empty folder on a disk with about 10 GB free",
                "The first time it gets the game itself",
                "Already have Survarium 0.26g",
                "If the launcher cannot get the game",
            ),
            (
                Lang::Uk,
                "у нову порожню теку на диску, де вільно близько 10 ГБ",
                "Першого разу він сам отримує гру",
                "Уже маєте Survarium 0.26g",
                "Якщо лаунчер не може отримати гру",
            ),
        ] {
            let page = home(
                lang,
                Stats {
                    installs_game: true,
                    ..stats(true)
                },
            );
            let steps = page.split("<ol>").nth(1).unwrap();
            let steps = steps.split("</ol>").next().unwrap();
            // The first step is the download, not the game.
            let first_step = steps.split("</li>").next().unwrap();
            assert!(first_step.contains(empty), "{first_step}");
            assert!(first_step.contains("class=dl"), "{first_step}");
            assert!(!steps.contains("Get the original Survarium"), "{steps}");
            assert!(!steps.contains("Отримайте оригінальну"), "{steps}");
            for said in [empty, first, have, way_out, "2.9", "Google Drive"] {
                let said = if lang == Lang::Uk && said == "2.9" {
                    "2,9"
                } else {
                    said
                };
                assert!(steps.contains(said), "{said}: {steps}");
            }
            assert!(steps.contains("<code>Skhid.bat</code>"), "{steps}");
            assert!(
                steps.contains("<code>bash Skhid.sh launcher</code>"),
                "{steps}"
            );
            assert_eq!(steps.matches("<details>").count(), 1);
        }
        // An archive with the extractor and no launcher is not such a one.
        let page = home(
            Lang::En,
            Stats {
                installs_game: true,
                ..stats(false)
            },
        );
        assert!(page.contains("Get the original Survarium"));
        assert!(!page.contains("empty folder"));
        // Without it: the page as it was.
        let page = home(Lang::En, stats(false));
        assert!(!page.contains("launcher_skhid.exe"));
        assert!(!page.contains("<details>"));
        assert!(page.contains("Start it with this command line:<pre>survarium_skhid.exe"));
        assert!(page.contains("<code>play_skhid.bat</code>"));
    }

    #[test]
    fn forms_decode_and_text_is_escaped() {
        assert_eq!(decode("a+b%20c%3D%26%zz%4"), "a b c=&%zz%4");
        assert_eq!(escape("<b>\"&"), "&lt;b&gt;&quot;&amp;");
        assert_eq!(apply(5, -10), 0);
        assert_eq!(delta("+12"), Some(12));
        assert_eq!(delta("x"), None);
    }
}
