//! What the patched 0.26g launcher and its updater ask this server for
//! (`client-patches/launcher/README.md`).
//!
//! - `GET /api/client-files`: which archive `/download` hands out now, as its
//!   SHA-256 and size. The updater compares it with what it installed last.
//! - `GET /launcher/…`: the page the launcher shows in its window, in the
//!   place of the original game's news page: the best players, what the
//!   server counts, and the legal notice. It is made for the launcher's
//!   embedded browser (Internet Explorer's engine, 899 by 335 pixels): fixed
//!   sizes, old CSS only, no script, no picture, nothing from another host.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use session::SessionStore;
use session::v026g::Board;

use crate::{CONNECT, ContentType, Game, Lang, Response, available_download, escape, locked};

/// The players the page lists.
const TOP_PLAYERS: usize = 10;
/// How long the page's numbers are kept before they are counted again, so
/// that asking often costs nothing.
const BOARD_AGE: Duration = Duration::from_secs(15);

/// The name by which an archive is known to hold the launcher: a ZIP keeps
/// its files' names as they are, in each file's header and in the list at
/// its end.
const LAUNCHER_NAME: &[u8] = b"game/binaries/x86/launcher_skhid.exe";

/// What was read of the download file the last time: it is read again only
/// when the file's size or time changes (a new archive is put in its place
/// without a restart).
struct Checked {
    file: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    sha256: String,
    /// The archive holds the launcher, so the site's page tells players to
    /// start that.
    launcher: bool,
}

static CHECKED: Mutex<Option<Checked>> = Mutex::new(None);
static BOARD: Mutex<Option<(Instant, Board)>> = Mutex::new(None);

/// The file's SHA-256, and whether [`LAUNCHER_NAME`] is in it, in one read.
fn read_through(file: &PathBuf) -> Option<(String, bool)> {
    let mut open = File::open(file).ok()?;
    let mut hash = openssl::sha::Sha256::new();
    // The end of what was read before stays in front of each new piece, so
    // a name that lies across two pieces is found too.
    let keep = LAUNCHER_NAME.len() - 1;
    let mut buffer = vec![0_u8; keep + 64 * 1024];
    let (mut kept, mut launcher) = (0, false);
    loop {
        match open.read(&mut buffer[kept..]) {
            Ok(0) => break,
            Ok(read) => {
                hash.update(&buffer[kept..kept + read]);
                let filled = kept + read;
                launcher = launcher
                    || buffer[..filled]
                        .windows(LAUNCHER_NAME.len())
                        .any(|part| part == LAUNCHER_NAME);
                kept = filled.min(keep);
                buffer.copy_within(filled - kept..filled, 0);
            }
            Err(_) => return None,
        }
    }
    let sha256 = hash
        .finish()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some((sha256, launcher))
}

/// What the download file is, if there is one.
struct Offered {
    sha256: String,
    size: u64,
    launcher: bool,
}

fn offered(file: &PathBuf) -> Option<Offered> {
    let meta = std::fs::metadata(file).ok().filter(|meta| meta.is_file())?;
    let (size, modified) = (meta.len(), meta.modified().ok());
    let mut checked = locked(&CHECKED);
    if let Some(known) = checked.as_ref() {
        if known.file == *file && known.size == size && known.modified == modified {
            return Some(Offered {
                sha256: known.sha256.clone(),
                size,
                launcher: known.launcher,
            });
        }
    }
    let (sha256, launcher) = read_through(file)?;
    // The file may have been replaced while it was read: what is said must
    // be of one file.
    let after = std::fs::metadata(file).ok()?;
    if after.len() != size || after.modified().ok() != modified {
        return None;
    }
    *checked = Some(Checked {
        file: file.clone(),
        size,
        modified,
        sha256: sha256.clone(),
        launcher,
    });
    Some(Offered {
        sha256,
        size,
        launcher,
    })
}

/// Whether the archive `/download` hands out holds the launcher: the home
/// page then tells players to start it, and else the start script, so the
/// page is right before and after the archive is replaced.
pub(super) fn offers_launcher() -> bool {
    available_download()
        .and_then(offered)
        .is_some_and(|offered| offered.launcher)
}

fn text(status: &'static str, body: String) -> Response {
    Response {
        status,
        content_type: ContentType::Text,
        body,
    }
}

/// `GET /api/client-files`: the format of `client/updater`
/// (`skhid-client-files 1`, then a word and its value on each line).
pub(super) fn client_files() -> Response {
    client_files_of(available_download())
}

fn client_files_of(file: Option<&PathBuf>) -> Response {
    let Some(file) = file else {
        return text("404 Not Found", "no client files are offered here\n".into());
    };
    match offered(file) {
        Some(Offered { sha256, size, .. }) => text(
            "200 OK",
            format!("skhid-client-files 1\nsha256 {sha256}\nsize {size}\n"),
        ),
        // Being replaced right now, or unreadable: the updater tries again.
        None => text(
            "503 Service Unavailable",
            "the client files cannot be read now\n".into(),
        ),
    }
}

/// The page's language from the launcher's own. The launcher asks for
/// `/launcher/v2/survarium/?lang=<en|ru|ua|de|pl>`: `ua` and `en` are read
/// with the request ([`Lang::pick`]), and for the three languages the site
/// has no text in, the browser's language decides as on every page. A code
/// as the last part of the path (`/launcher/ua`, `/launcher/v2/index-ua.html`)
/// is taken too.
pub(super) fn language(path: &str, chosen: Lang) -> Lang {
    let last = path.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    let last = last.strip_suffix(".html").unwrap_or(&last);
    let code = last.rsplit('-').next().unwrap_or("");
    match code {
        "ua" | "uk" | "ukrainian" => Lang::Uk,
        "en" | "ru" | "de" | "pl" | "english" | "russian" | "german" | "polish" => Lang::En,
        _ => chosen,
    }
}

/// What the page counts, beside the board.
#[derive(Clone, Copy)]
pub(super) struct Counts {
    pub online: usize,
    pub registered: usize,
    pub matches: usize,
    pub queued: usize,
    pub clans: usize,
    /// The minutes to a planned maintenance.
    pub maintenance: Option<u32>,
}

/// `GET /launcher/…`.
pub(super) fn page(lang: Lang, store: &SessionStore, game: &Game) -> Response {
    let board = {
        let mut kept = locked(&BOARD);
        match kept.as_ref() {
            Some((at, board)) if at.elapsed() < BOARD_AGE => board.clone(),
            _ => {
                let board = game.accounts.board(TOP_PLAYERS);
                *kept = Some((Instant::now(), board.clone()));
                board
            }
        }
    };
    let counts = Counts {
        online: store.players_present(),
        registered: store.registered_accounts(),
        matches: game.matches.active_matches(),
        queued: store.matchmaking().queued_players(),
        clans: game.accounts.clans().len(),
        maintenance: store.minutes_to_maintenance(),
    };
    let test = CONNECT.get().is_some_and(|connect| connect.test_server);
    Response {
        status: "200 OK",
        content_type: ContentType::Html,
        body: render(lang, &board, counts, test),
    }
}

/// A number with its thousands apart: 12 345.
fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push_str("&nbsp;");
        }
        out.push(digit);
    }
    out
}

fn render(lang: Lang, board: &Board, counts: Counts, test: bool) -> String {
    let mut rows = String::new();
    for (index, row) in board.top.iter().enumerate() {
        let clan = if row.clan_tag.is_empty() {
            String::new()
        } else {
            format!("<i>[{}]</i> ", escape(&row.clan_tag))
        };
        rows += &format!(
            "<tr{}><td class=n>{}</td><td class=p>{clan}{}</td><td class=l>{}</td><td class=r>{}</td></tr>",
            if index % 2 == 1 { " class=o" } else { "" },
            index + 1,
            escape(&row.name),
            row.level,
            row.rating,
        );
    }
    let table = if board.top.is_empty() {
        format!(
            "<p class=none>{}</p>",
            lang.tr(
                "No battle has been fought yet. Be the first.",
                "Ще не відбулося жодного бою. Будьте першими."
            )
        )
    } else {
        format!(
            "<table cellspacing=0 cellpadding=0><tr><th class=n>#</th><th class=p>{}</th>\
             <th class=l>{}</th><th class=r>{}</th></tr>{rows}</table>",
            lang.tr("Player", "Гравець"),
            lang.tr("Level", "Рівень"),
            lang.tr("Rating", "Рейтинг"),
        )
    };
    let cell = |value: u64, label: String| {
        format!("<td><b>{}</b><span>{label}</span></td>", grouped(value))
    };
    let now = format!(
        "<table class=nums cellspacing=0 cellpadding=0><tr>{}{}{}</tr></table>",
        cell(counts.online as u64, lang.tr("online", "онлайн")),
        cell(counts.queued as u64, lang.tr("searching", "у пошуку")),
        cell(
            counts.matches as u64,
            lang.tr("matches now", "матчів зараз")
        ),
    );
    let totals = format!(
        "<table class=nums cellspacing=0 cellpadding=0><tr>{}{}{}</tr></table>",
        cell(counts.registered as u64, lang.tr("players", "гравців")),
        cell(board.battles, lang.tr("battles fought", "проведено боїв")),
        cell(counts.clans as u64, lang.tr("clans", "кланів")),
    );
    let status = match counts.maintenance {
        Some(0) => lang.tr(
            "Maintenance: the server is down",
            "Технічні роботи: сервер вимкнено",
        ),
        Some(minutes) => lang.tr(
            &format!("Maintenance in {minutes} min"),
            &format!("Технічні роботи через {minutes} хв"),
        ),
        None => lang.tr("On the server now", "Зараз на сервері"),
    };
    let status_class = if counts.maintenance.is_some() {
        "cap warn"
    } else {
        "cap"
    };
    let badge = if test {
        format!(" <em>{}</em>", lang.tr("TEST SERVER", "ТЕСТОВИЙ СЕРВЕР"))
    } else {
        String::new()
    };
    let legal = lang.tr(
        "Unofficial community server. Not affiliated with or endorsed by Vostok Games. \
         Survarium is a trademark of its owners. An early test: accounts and progress \
         can be reset at any time.",
        "Неофіційний ком'юніті сервер. Не пов'язаний із Vostok Games і не схвалений ними. \
         Survarium є торговельною маркою її власників. Раннє тестування: акаунти та \
         прогрес можуть бути скинуті будь-коли.",
    );
    format!(
        "<!DOCTYPE html><html lang={code}><head><meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge\">\
         <meta http-equiv=\"content-type\" content=\"text/html; charset=UTF-8\">\
         <title>Skhid</title><style type=\"text/css\">{STYLE}</style></head><body>\
         <div id=container>\
         <div id=left><h1>{top}{badge}</h1>{table}<div class=cap>{server}</div></div>\
         <div id=right>\
         <div class=tile>{now}<div class=\"{status_class}\">{status}</div></div>\
         <div class=tile>{totals}<div class=cap>{since}</div></div>\
         <div class=tile><p class=legal>{legal}</p><div class=cap>{notice}</div></div>\
         </div></div></body></html>",
        code = lang.code(),
        top = lang.tr("Top 10 players", "Топ-10 гравців"),
        server = lang.tr(
            "Skhid &middot; Survarium <i>0.26g</i> community server",
            "Схід &middot; ком'юніті сервер Survarium <i>0.26g</i>"
        ),
        since = lang.tr("Since the server opened", "Від відкриття сервера"),
        notice = lang.tr("Legal notice", "Правова інформація"),
        legal = escape(&legal),
    )
}

/// The original page's measures (`game/launcher/public/style.css`): a
/// container of 899 by 335, a left panel of 456 by 314 at 15, 10, and three
/// tiles of 404 by 98 on the right, each with a caption strip at its foot.
/// Its colours: black panels, grey text, white captions, the amber of the
/// maintenance line.
const STYLE: &str = "*{padding:0;margin:0;outline:0;border:0}\
html,body{background:#000;font-size:13px;color:#b3b3b3;font-family:Arial,Tahoma,sans-serif;overflow:hidden}\
#container{position:relative;width:899px;height:335px;overflow:hidden;background:#161514}\
#left{float:left;position:relative;width:454px;height:312px;margin:10px 0 0 15px;background:#000;border:1px solid #3b3832;overflow:hidden}\
#right{float:right;width:404px;padding:4px 16px 0 0}\
.tile{position:relative;width:402px;height:96px;margin:7px 0 10px 0;background:#000;border:1px solid #3b3832;overflow:hidden}\
h1{height:30px;line-height:30px;padding:0 13px;font-size:13px;font-weight:bold;color:#e7a300;text-transform:uppercase;background:#1b1a18;border-bottom:1px solid #3b3832}\
h1 em{float:right;font-style:normal;color:#fff;background:#b53a26;line-height:16px;padding:0 6px;margin-top:7px;font-size:11px}\
table{border-collapse:collapse}\
#left table{width:454px}\
#left th,#left td{height:22px;line-height:22px;font-size:12px;text-align:left;white-space:nowrap;overflow:hidden}\
#left th{height:20px;line-height:20px;font-size:10px;font-weight:normal;color:#77746c;text-transform:uppercase}\
#left tr.o td{background:#0d0d0c}\
.n{width:30px;padding-left:13px;color:#77746c}\
.p{color:#fff}\
.p i{font-style:normal;color:#e7a300}\
.l{width:60px}\
.r{width:70px;color:#e7a300;font-weight:bold}\
.none{padding:110px 13px 0 13px;text-align:center;color:#939392}\
.cap{position:absolute;left:0;bottom:0;width:100%;height:23px;line-height:23px;text-indent:13px;background:#1b1a18;border-top:1px solid #3b3832;font-weight:bold;color:#fff;text-transform:uppercase;white-space:nowrap;overflow:hidden}\
.cap i{font-style:normal;text-transform:none}\
.warn{color:#e7a300}\
.nums{width:402px;margin-top:11px}\
.nums td{width:134px;text-align:center;vertical-align:top}\
.nums b{display:block;font-size:26px;line-height:32px;color:#fff;font-weight:bold}\
.nums span{display:block;font-size:10px;line-height:14px;color:#939392;text-transform:uppercase}\
.legal{padding:7px 13px 0 13px;font-size:11px;line-height:13px;color:#939392}";

#[cfg(test)]
mod tests {
    use super::*;
    use session::v026g::BoardRow;

    fn counts() -> Counts {
        Counts {
            online: 12,
            registered: 3456,
            matches: 2,
            queued: 5,
            clans: 7,
            maintenance: None,
        }
    }

    fn board() -> Board {
        Board {
            top: vec![
                BoardRow {
                    name: "Sasha".into(),
                    clan_tag: "TAG".into(),
                    rating: 1420,
                    level: 31,
                },
                BoardRow {
                    name: "<b>x</b>".into(),
                    clan_tag: String::new(),
                    rating: 1300,
                    level: 4,
                },
            ],
            battles: 1_234_567,
        }
    }

    #[test]
    fn the_page_lists_the_board_and_the_counts() {
        let page = render(Lang::En, &board(), counts(), false);
        assert!(page.contains("<td class=n>1</td><td class=p><i>[TAG]</i> Sasha</td><td class=l>31</td><td class=r>1420</td>"));
        // A name is text, never markup.
        assert!(page.contains("&lt;b&gt;x&lt;/b&gt;"));
        assert!(!page.contains("<b>x</b>"));
        assert!(page.contains("<b>12</b><span>online</span>"));
        assert!(page.contains("<b>5</b><span>searching</span>"));
        assert!(page.contains("<b>2</b><span>matches now</span>"));
        assert!(page.contains("<b>3&nbsp;456</b><span>players</span>"));
        assert!(page.contains("<b>1&nbsp;234&nbsp;567</b><span>battles fought</span>"));
        assert!(page.contains("<b>7</b><span>clans</span>"));
        assert!(page.contains("Not affiliated with or endorsed by Vostok Games"));
        assert!(!page.contains("TEST SERVER"));
        assert!(render(Lang::En, &board(), counts(), true).contains("<em>TEST SERVER</em>"));
    }

    #[test]
    fn the_page_asks_nothing_of_another_host_and_runs_no_script() {
        for lang in [Lang::En, Lang::Uk] {
            let page = render(lang, &board(), counts(), true).to_ascii_lowercase();
            for banned in [
                "http://",
                "https://",
                "//",
                "<script",
                "<img",
                "<link",
                "<iframe",
                "url(",
                "@import",
                "javascript:",
                "href",
                "src=",
            ] {
                // The doctype and the X-UA-Compatible line name no address.
                assert!(!page.contains(banned), "{banned} in the launcher page");
            }
        }
    }

    #[test]
    fn the_page_is_ukrainian_for_the_ukrainian_launcher() {
        let page = render(Lang::Uk, &board(), counts(), false);
        assert!(page.contains("Топ-10 гравців"));
        assert!(page.contains("<span>проведено боїв</span>"));
        assert!(page.contains("lang=uk"));
        let empty = render(Lang::Uk, &Board::default(), counts(), false);
        assert!(empty.contains("Ще не відбулося жодного бою"));
        assert!(!empty.contains("<th"));
    }

    #[test]
    fn a_planned_maintenance_is_said_on_the_page() {
        let mut soon = counts();
        soon.maintenance = Some(25);
        let page = render(Lang::En, &board(), soon, false);
        assert!(page.contains("<div class=\"cap warn\">Maintenance in 25 min</div>"));
        soon.maintenance = Some(0);
        assert!(
            render(Lang::En, &board(), soon, false).contains("Maintenance: the server is down")
        );
        assert!(
            render(Lang::En, &board(), counts(), false)
                .contains("<div class=\"cap\">On the server now</div>")
        );
    }

    #[test]
    fn the_language_comes_from_the_launchers_path() {
        for path in [
            "/launcher/v2/index-ua.html",
            "/launcher/ua",
            "/launcher/v2/ua",
            "/launcher/v2/ukrainian",
        ] {
            assert_eq!(language(path, Lang::En), Lang::Uk, "{path}");
        }
        for path in [
            "/launcher/v2/index-en.html",
            "/launcher/ru",
            "/launcher/v2/de",
            "/launcher/pl",
            "/launcher/v2/english",
        ] {
            assert_eq!(language(path, Lang::Uk), Lang::En, "{path}");
        }
        // Nothing in the path (the launcher's own form,
        // `/launcher/v2/survarium/?lang=ua`): `?lang=` or the browser's
        // language, as on every other page.
        assert_eq!(language("/launcher/v2/survarium/", Lang::Uk), Lang::Uk);
        assert_eq!(language("/launcher/v2/survarium/", Lang::En), Lang::En);
        assert_eq!(Lang::pick("ua", "en-US"), Lang::Uk);
        assert_eq!(Lang::pick("en", "uk-UA"), Lang::En);
        assert_eq!(Lang::pick("ru", "uk-UA,uk;q=0.9"), Lang::Uk);
        assert_eq!(Lang::pick("de", "de-DE"), Lang::En);
        assert_eq!(language("/launcher/v2/", Lang::Uk), Lang::Uk);
        assert_eq!(language("/launcher", Lang::En), Lang::En);
    }

    #[test]
    fn numbers_are_grouped_by_thousands() {
        for (value, text) in [
            (0, "0"),
            (999, "999"),
            (1000, "1&nbsp;000"),
            (1_234_567, "1&nbsp;234&nbsp;567"),
        ] {
            assert_eq!(grouped(value), text);
        }
    }

    #[test]
    fn the_offered_archive_is_its_checksum_and_size() {
        let dir = std::env::temp_dir().join(format!("srp-launcher-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("files.zip");
        std::fs::write(&file, b"abc").unwrap();
        let answer = client_files_of(Some(&file));
        assert_eq!(answer.status, "200 OK");
        assert_eq!(
            answer.body,
            "skhid-client-files 1\n\
             sha256 ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\nsize 3\n"
        );
        // Another archive in its place is seen without a restart.
        std::fs::write(
            &file,
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
        )
        .unwrap();
        assert!(client_files_of(Some(&file)).body.contains(
            "sha256 248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1\nsize 56\n"
        ));
        assert_eq!(
            client_files_of(Some(&dir.join("gone.zip"))).status,
            "503 Service Unavailable"
        );
        assert_eq!(client_files_of(None).status, "404 Not Found");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_archive_with_the_launcher_is_known_by_the_launchers_name() {
        let dir = std::env::temp_dir().join(format!("srp-launcher-name-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("files.zip");
        // Without it; then with it at the start, across the border of two
        // pieces of the read, and at the end.
        std::fs::write(&file, vec![b'x'; 200_000]).unwrap();
        assert!(!offered(&file).unwrap().launcher);
        for before in [
            0,
            64 * 1024 - 7,
            64 * 1024 + 20,
            2 * 64 * 1024 + 30,
            199_000,
        ] {
            let mut bytes = vec![b'x'; before];
            bytes.extend_from_slice(LAUNCHER_NAME);
            if before != 199_000 {
                bytes.resize(200_000, b'x');
            }
            std::fs::write(&file, &bytes).unwrap();
            let (sha256, launcher) = read_through(&file).unwrap();
            assert!(launcher, "{before}");
            let whole: String = openssl::sha::sha256(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(sha256, whole, "{before}");
        }
        // The game exe's name is not the launcher's.
        std::fs::write(&file, b"game/binaries/x86/survarium_skhid.exe").unwrap();
        assert!(!read_through(&file).unwrap().1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
