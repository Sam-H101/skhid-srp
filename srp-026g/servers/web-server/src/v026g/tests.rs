//! The 0.26g page against a store and 0.26g accounts in memory, with a
//! lobby link that records what it is told.

use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use session::v026g::GameAccounts;
use session::{LockExt, Net, SessionStore, SignInError};
use survarium::v026g::dictionary::Dictionary;
use survarium::v026g::lobby::FULL_CONDITION;

use super::{Game, LobbyLink, level_start, sha1_hex};
use crate::{Lang, Request, Web};

#[derive(Default)]
struct Told {
    changed: Mutex<Vec<u32>>,
    text: Mutex<Option<(String, String)>>,
    cleared: AtomicU32,
}

impl LobbyLink for Told {
    fn account_changed(&self, account_id: u32) {
        self.changed.locked().push(account_id);
    }
    fn set_admin_text(&self, first: &str, second: &str) {
        *self.text.locked() = Some((first.to_owned(), second.to_owned()));
    }
    fn clear_admin_text(&self) {
        *self.text.locked() = None;
        self.cleared
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn admin_text(&self) -> Option<(String, String)> {
        self.text.locked().clone()
    }
}

struct World {
    web: Web,
    accounts: Arc<GameAccounts>,
    told: Arc<Told>,
}

fn world() -> World {
    let accounts = Arc::new(GameAccounts::new());
    let told = Arc::new(Told::default());
    let web = Web {
        store: Arc::new(SessionStore::new()),
        token: Some("tok".into()),
        admin_path: Some("/hidden".into()),
        admin_ips: vec![Net::parse("127.0.0.1").unwrap()],
        trust_proxy: false,
        token_failures: Mutex::new((0, None)),
        connections: AtomicU32::new(0),
        downloads: AtomicU32::new(0),
        game: Some(Game {
            accounts: accounts.clone(),
            lobby: told.clone(),
        }),
    };
    World {
        web,
        accounts,
        told,
    }
}

impl World {
    /// An account as the 0.26g login server registers it (the digest of
    /// `password`), with its 0.26g state as the lobby makes it. Returns its
    /// id.
    fn player(&self, name: &str, password: &str) -> u32 {
        let id = self
            .web
            .store
            .sign_in_with_digest_from(
                name,
                &sha1_hex(password.as_bytes()),
                "127.0.0.1".parse().unwrap(),
                Instant::now(),
            )
            .unwrap();
        self.accounts.get_or_create(id, name);
        id
    }

    fn balances(&self, id: u32) -> (u32, u32) {
        let account = self.accounts.get(id).unwrap();
        let account = account.locked();
        (account.generic_money, account.premium_money)
    }

    fn admin(&self, fields: &[(&str, &str)]) -> String {
        let mut all = vec![("token", "tok")];
        all.extend_from_slice(fields);
        self.web.route(&post("/hidden", &all), "t").body
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
        ..post(path, &[])
    }
}

#[test]
fn the_digest_is_the_clients() {
    // SHA-1 of the empty text and of "abc", as 40 lower-case hex characters.
    assert_eq!(&sha1_hex(b""), b"da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(
        &sha1_hex(b"abc"),
        b"a9993e364706816aba3e25717850c26c9cd0d89d"
    );
    assert!(session::password::is_sha1_digest(&sha1_hex(b"anything")));
}

#[test]
fn the_typed_password_is_checked_as_the_026g_login_keeps_it() {
    let world = world();
    let id = world.player("fighter", " secret ");
    let (silver, gold) = world.balances(id);
    let exchange = |user: &str, password: &str, gold: &str| {
        world
            .web
            .route(
                &post(
                    "/exchange",
                    &[("user", user), ("password", password), ("gold", gold)],
                ),
                "t",
            )
            .body
    };
    // Wrong, trimmed (the game hashes the spaces too), the digest itself
    // (not what is typed), and a name nobody has: all the same answer.
    let digest = String::from_utf8(sha1_hex(b" secret ").to_vec()).unwrap();
    for (user, password) in [
        ("fighter", "nope"),
        ("fighter", "secret"),
        ("fighter", digest.as_str()),
        ("ghost", " secret "),
    ] {
        let body = exchange(user, password, "1");
        assert!(body.contains("Wrong user name or password"), "{body}");
    }
    assert_eq!(world.balances(id), (silver, gold));
    assert!(world.web.store.account_id_by_name("ghost").is_none());
    assert!(world.told.changed.locked().is_empty());

    let done = exchange("Fighter", " secret ", "2");
    assert!(done.contains("Done: 200 silver for 2 gold"), "{done}");
    assert_eq!(world.balances(id), (silver - 200, gold + 2));
    // The lobby is told whose account changed.
    assert_eq!(world.told.changed.locked()[..], [id]);
    // The page is the 0.26g one.
    assert!(done.contains("SURVARIUM 0.26G") && !done.contains("0.21D"));
}

#[test]
fn gold_is_sold_for_silver_by_the_games_own_rule() {
    let world = world();
    let id = world.player("trader", "secret");
    world.accounts.get(id).unwrap().locked().premium_money = 5;
    let (silver, _) = world.balances(id);
    let sell = |gold: &str| {
        world
            .web
            .route(
                &post(
                    "/exchange",
                    &[
                        ("user", "trader"),
                        ("password", "secret"),
                        ("gold", gold),
                        ("way", "sell"),
                    ],
                ),
                "t",
            )
            .body
    };
    let refused = sell("6");
    assert!(refused.contains("Not enough gold: you have 5"), "{refused}");
    assert_eq!(world.balances(id), (silver, 5));
    let done = sell("5");
    assert!(done.contains("Done: 5 gold for 500 silver"), "{done}");
    assert_eq!(world.balances(id), (silver + 500, 0));
    assert!(sell("0").contains("Gold must be"));
    // Silver that is not there buys nothing.
    world.accounts.get(id).unwrap().locked().generic_money = 99;
    let poor = world
        .web
        .route(
            &post(
                "/exchange",
                &[("user", "trader"), ("password", "secret"), ("gold", "1")],
            ),
            "t",
        )
        .body;
    assert!(poor.contains("Not enough silver"), "{poor}");
    assert_eq!(world.balances(id), (99, 0));
}

#[test]
fn an_account_that_never_saw_the_lobby_or_is_banned_exchanges_nothing() {
    let world = world();
    let id = world
        .web
        .store
        .sign_in_with_digest_from(
            "newcomer",
            &sha1_hex(b"secret"),
            "127.0.0.1".parse().unwrap(),
            Instant::now(),
        )
        .unwrap();
    let ask = || {
        world
            .web
            .route(
                &post(
                    "/exchange",
                    &[("user", "newcomer"), ("password", "secret"), ("gold", "1")],
                ),
                "t",
            )
            .body
    };
    assert!(ask().contains("Sign in to the game once first"));
    world.accounts.get_or_create(id, "newcomer");
    world.web.store.ban_account(
        id,
        session::Ban {
            until: None,
            reason: "test".into(),
        },
    );
    assert!(ask().contains("This account is banned"));
}

#[test]
fn the_admin_changes_money_and_the_account() {
    let world = world();
    let id = world.player("tester", "secret");
    let (silver, gold) = world.balances(id);
    let refused = world.web.route(
        &post(
            "/hidden",
            &[("token", "bad"), ("user", "tester"), ("gold", "+9")],
        ),
        "t",
    );
    assert!(refused.body.contains("Wrong token"));
    assert_eq!(world.balances(id), (silver, gold));

    let done = world.admin(&[
        ("user", "tester"),
        ("gold", "+500"),
        ("silver", "-999999999"),
    ]);
    assert!(done.contains("now 0 silver"), "{done}");
    assert_eq!(world.balances(id), (0, gold + 500));
    assert_eq!(world.told.changed.locked()[..], [id]);

    // Premium, a level with experience on top, reputation.
    let done = world.admin(&[
        ("tab", "account"),
        ("acc_user", "tester"),
        ("premium_hours", "+24"),
        ("level", "3"),
        ("experience", "+10"),
        ("faction", "2"),
        ("reputation", "+150"),
    ]);
    assert!(done.contains("level 3"), "{done}");
    assert!(done.contains("24 h of premium"), "{done}");
    assert!(done.contains("Reputation with faction 2: 150"), "{done}");
    assert!(done.contains("value=account id=tab-account checked"));
    {
        let account = world.accounts.get(id).unwrap();
        let account = account.locked();
        assert_eq!(account.level(), 3);
        assert_eq!(account.exp_total, level_start(3) + 10);
        assert_eq!(account.premium_hours(crate::unix_now()), Some(24));
        assert_eq!(account.reputation(2), 150);
    }
    // Taken back; the premium ends.
    let done = world.admin(&[
        ("tab", "account"),
        ("acc_user", "tester"),
        ("premium_hours", "-48"),
        ("faction", "2"),
        ("reputation", "-1000"),
    ]);
    assert!(done.contains("0 h of premium"), "{done}");
    {
        let account = world.accounts.get(id).unwrap();
        let account = account.locked();
        assert_eq!(account.premium_until, 0);
        assert_eq!(account.reputation(2), 0);
        assert_eq!(account.level(), 3);
    }
    // Wrong fields change nothing.
    for fields in [
        // One past the highest level, 100.
        [("level", "101"), ("faction", "")],
        [("level", ""), ("faction", "9")],
        [("experience", "x"), ("faction", "")],
        [("reputation", "+5"), ("faction", "")],
    ] {
        let mut all = vec![("tab", "account"), ("acc_user", "tester")];
        all.extend_from_slice(&fields);
        assert!(world.admin(&all).contains("whole numbers"), "{fields:?}");
    }
    assert!(
        world
            .admin(&[("tab", "account"), ("acc_user", "ghost"), ("level", "1")])
            .contains("No such user")
    );
    assert_eq!(world.told.changed.locked().len(), 3);
}

#[test]
fn a_level_under_the_spent_skill_points_is_refused() {
    let world = world();
    let id = world.player("skilled", "secret");
    {
        let account = world.accounts.get(id).unwrap();
        let mut account = account.locked();
        account.exp_total = level_start(5);
        account.skills = vec![(1, 4)];
    }
    let refused = world.admin(&[("tab", "account"), ("acc_user", "skilled"), ("level", "3")]);
    assert!(refused.contains("skill points"), "{refused}");
    assert_eq!(world.accounts.get(id).unwrap().locked().level(), 5);
    assert!(world.told.changed.locked().is_empty());
    let done = world.admin(&[("tab", "account"), ("acc_user", "skilled"), ("level", "4")]);
    assert!(done.contains("level 4"), "{done}");
}

#[test]
fn the_admin_gives_items_of_the_dictionary_only() {
    let world = world();
    let id = world.player("tester", "secret");
    let dictionary = Dictionary::shared();
    let owned = |dict_id: u16| -> Vec<u32> {
        let account = world.accounts.get(id).unwrap();
        let account = account.locked();
        account
            .inventory
            .iter()
            .filter(|item| item.dict_id == dict_id)
            .map(|item| item.condition_or_stack)
            .collect()
    };
    // The starter kit's pistol and its ammo are in the dictionary: one that
    // is no stack, one that is.
    let (weapon, ammo) = {
        let account = world.accounts.get(id).unwrap();
        let account = account.locked();
        let pick = |stack: bool| {
            account
                .inventory
                .iter()
                .map(|item| item.dict_id)
                .find(|dict_id| {
                    dictionary
                        .item(*dict_id)
                        .is_some_and(|item| item.is_stack == stack)
                })
                .expect("the starter loadout has a weapon and its ammo")
        };
        (pick(false), pick(true))
    };
    let had = (owned(weapon).len(), owned(ammo)[0]);

    let by_id = world.admin(&[
        ("tab", "item"),
        ("item_user", "tester"),
        ("item", &weapon.to_string()),
        ("count", "2"),
    ]);
    assert!(by_id.contains("given 2 of"), "{by_id}");
    let weapons = owned(weapon);
    assert_eq!(weapons.len(), had.0 + 2);
    assert_eq!(weapons[weapons.len() - 1], FULL_CONDITION);

    // By name; a stack grows, it is not a second instance.
    let name = &dictionary.item(ammo).unwrap().name;
    let by_name = world.admin(&[
        ("tab", "item"),
        ("item_user", "tester"),
        ("item", name),
        ("count", "30"),
    ]);
    assert!(by_name.contains("given 30 of"), "{by_name}");
    assert_eq!(owned(ammo), [had.1 + 30]);
    assert_eq!(world.told.changed.locked().len(), 2);

    // An id the dictionary has not (the client would crash on it), and
    // counts out of range.
    let unknown = (1..u16::MAX)
        .find(|id| dictionary.item(*id).is_none())
        .unwrap();
    for (item, count, answer) in [
        (unknown.to_string(), "1", "No such item"),
        ("no_such_item".to_owned(), "1", "No such item"),
        (weapon.to_string(), "9", "count for this item is 1 to 8"),
        (weapon.to_string(), "0", "count for this item"),
        (
            ammo.to_string(),
            "100001",
            "count for this item is 1 to 100000",
        ),
    ] {
        let body = world.admin(&[
            ("tab", "item"),
            ("item_user", "tester"),
            ("item", &item),
            ("count", count),
        ]);
        assert!(body.contains(answer), "{item} x{count}: {body}");
    }
    assert_eq!(world.told.changed.locked().len(), 2);
    // Every id is still one the dictionary has, and the account is whole.
    let account = world.accounts.get(id).unwrap();
    let mut account = account.locked();
    assert!(
        account
            .inventory
            .iter()
            .all(|item| dictionary.item(item.dict_id).is_some())
    );
    assert!(!account.repair());
}

#[test]
fn the_admin_sets_and_clears_the_text_for_everyone() {
    let world = world();
    let shown = world.admin(&[
        ("tab", "text"),
        ("line1", "Restart at 20:00"),
        ("line2", "<b>"),
    ]);
    assert!(shown.contains("shown to everyone"), "{shown}");
    assert_eq!(
        world.told.admin_text(),
        Some(("Restart at 20:00".to_owned(), "<b>".to_owned()))
    );
    // The page shows what stands, escaped.
    assert!(shown.contains("Shown now:") && shown.contains("&lt;b&gt;"));
    // Too long for the client's buffers (a Cyrillic letter is 2 bytes).
    let long = "я".repeat(32);
    let refused = world.admin(&[("tab", "text"), ("line1", &long)]);
    assert!(refused.contains("at most 63 bytes"), "{refused}");
    assert_eq!(world.told.admin_text().unwrap().0, "Restart at 20:00");
    let cleared = world.admin(&[("tab", "text"), ("clear_text", "1")]);
    assert!(cleared.contains("The text is cleared"), "{cleared}");
    assert_eq!(world.told.admin_text(), None);
    // Without the token nothing is set.
    let refused = world.web.route(
        &post(
            "/hidden",
            &[("token", "bad"), ("tab", "text"), ("line1", "x")],
        ),
        "t",
    );
    assert!(refused.body.contains("Wrong token"));
    assert_eq!(world.told.admin_text(), None);
}

#[test]
fn bans_maintenance_and_registration_are_the_stores() {
    let world = world();
    let id = world.player("cheater", "secret");
    let session = world.web.store.create_session(id);
    let banned = world.admin(&[
        ("tab", "ban"),
        ("ban_user", "cheater"),
        ("hours", "2"),
        ("reason", "test"),
    ]);
    assert!(
        banned.contains("cheater banned for 2 h; 1 session(s) ended"),
        "{banned}"
    );
    // The session the 0.26g lobby holds has ended (its link is signed out
    // on the lobby's next pass), and the 0.26g sign-in is refused.
    assert!(world.web.store.has_ended(session));
    let again = world.web.store.sign_in_with_digest_from(
        "cheater",
        &sha1_hex(b"secret"),
        "127.0.0.1".parse().unwrap(),
        Instant::now(),
    );
    assert_eq!(again, Err(SignInError::Banned));
    world.admin(&[("tab", "unban"), ("unban_user", "cheater")]);
    assert!(world.web.store.account_ban(id).is_none());

    world.admin(&[("tab", "maint"), ("minutes", "45")]);
    assert_eq!(world.web.store.minutes_to_maintenance(), Some(45));
    world.admin(&[("tab", "signups"), ("signups", "closed")]);
    let refused = world.web.store.sign_in_with_digest_from(
        "late",
        &sha1_hex(b"secret"),
        "127.0.0.1".parse().unwrap(),
        Instant::now(),
    );
    assert_eq!(refused, Err(SignInError::RegistrationsClosed));
}

#[test]
fn the_pages_are_the_026g_ones() {
    let world = world();
    let home = world.web.route(&get("/"), "t").body;
    assert!(home.contains("survarium_skhid.exe -localization=english -server="));
    assert!(home.contains("-mt_allocator=768 -minimum_resources_memory_size=768"));
    assert!(!home.contains("-client") && !home.contains("0.21d") && !home.contains("0.21D"));
    assert!(home.contains("Survarium 0.26g client"));
    let exchange = world.web.route(&get("/exchange"), "t").body;
    assert!(exchange.contains("Sell gold for silver") && exchange.contains("name=way"));
    let admin = world.web.route(&get("/hidden"), "t");
    assert_eq!(admin.status, "200 OK");
    for tab in [
        "money", "ban", "unban", "bans", "maint", "signups", "account", "item", "text",
    ] {
        assert!(admin.body.contains(&format!("id=tab-{tab}")), "{tab}");
    }
    let missing = world.web.route(&get("/nothing"), "t");
    assert_eq!(missing.status, "404 Not Found");
    assert!(missing.body.contains("0.26G"));
    let mut elsewhere = get("/hidden");
    elsewhere.client = "203.0.113.5".parse().ok();
    assert_eq!(world.web.route(&elsewhere, "t").status, "404 Not Found");
}
