//! Game data made up for tests: the test data with scope items and
//! variants laid over it as a release patch adds them ([`with_scopes`]):
//! rows in `items_dict` of a new dictionary and a config for each. The test
//! data itself has no variant, so its own table is empty
//! ([`super::ScopeFits::shared`] in tests).

use binary_config_parser::tree::ConfigValue;

use super::super::dictionary::{self, Dictionary};
use super::super::game_data::{self, GameData};
use super::{FIT_KEY, SCOPE_CATEGORY, ScopeFits};

/// The shop's PSO-1 (sold for [`PSO_1_COST`]).
pub const PSO_1: u16 = 2101;
/// The shop's Cobra EKP-8 (sold for [`COBRA_COST`]).
pub const COBRA: u16 = 2108;
/// A scope item no variant names: nothing goes on with it.
pub const LONE_SCOPE: u16 = 2199;
pub const PSO_1_COST: u32 = 12_000;
pub const COBRA_COST: u32 = 7_000;
/// The PSO-1 made for the AK-74N (and its premium copy), named by number.
pub const AK_74N_PSO_1: u16 = 2001;
/// The Cobra made for the AK-74N, named by name.
pub const AK_74N_COBRA: u16 = 2003;
/// The Cobra made for the SVD, a weapon with a stock scope (PSO-1, 304):
/// its modification sets the scope.
pub const SVD_COBRA: u16 = 2029;
/// Variants the table leaves out: no modification, a modification of no
/// row of its own, a weapon as its scope, a vest as its weapon, a pair
/// another variant has (higher id), a variant as its scope, an add on the
/// SVD (which has a stock scope: the client would assert on the duplicate
/// name), a set on the AK-74M (which has none: no scope would show), the
/// SVD's stock PSO-1 (304) as its shop scope.
pub const BROKEN: [u16; 9] = [2090, 2091, 2092, 2093, 2094, 2095, 2096, 2097, 2098];
/// The weapons the patch gives an `addons { mount: 1 }` placeholder, so
/// that an add of `rifle_scope_dict_id` takes (the note's §2).
pub const PLACEHOLDERS: [&str; 2] = ["ak_74n", "ak_74n_premium"];

fn named(entries: Vec<(&str, ConfigValue)>) -> ConfigValue {
    ConfigValue::Named(
        entries
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

fn text(value: &str) -> ConfigValue {
    ConfigValue::String(value.to_owned())
}

/// What a scope config has for the client to show: models and zoom.
fn scope_data(name: &str) -> ConfigValue {
    named(vec![
        ("fov_factor", ConfigValue::Float(0.25)),
        ("hide_weapon_on_aim", ConfigValue::Boolean(true)),
        (
            "aimed_model",
            text(&format!("weapons/attach/fit/{name}_aim")),
        ),
        ("idle_model", text(&format!("weapons/attach/fit/{name}"))),
        ("type", ConfigValue::Integer(5)),
    ])
}

/// How a variant's modification puts its scope on the weapon.
#[derive(Clone, Copy)]
enum Puts {
    /// `type` 0 on `[addons, rifle_scope_dict_id]`.
    Set,
    /// `type` 3 on `[addons]` with `[rifle_scope_dict_id, <id>]`.
    Add,
}

/// A variant's config: its modification `puts` `addons.rifle_scope_dict_id`
/// = itself (`modification` None: no `parameters.modification`; `own`
/// false: the row is another id's), and `fit` is its [`FIT_KEY`] block.
fn variant_config(
    dict_id: u16,
    modification: Option<i32>,
    own: bool,
    puts: Puts,
    fit: ConfigValue,
) -> ConfigValue {
    let id = modification.unwrap_or(990_000 + i32::from(dict_id));
    let mut parameters = vec![("weight", ConfigValue::Float(0.3))];
    if let Some(modification) = modification {
        parameters.push(("modification", ConfigValue::Integer(modification)));
    }
    let row_id = if own { id } else { id + 1 };
    let modifier = match puts {
        Puts::Set => named(vec![
            (
                "path",
                ConfigValue::Indexed(vec![text("addons"), text("rifle_scope_dict_id")]),
            ),
            ("value", ConfigValue::Integer(dict_id.into())),
            ("type", ConfigValue::Integer(0)),
        ]),
        Puts::Add => named(vec![
            ("path", ConfigValue::Indexed(vec![text("addons")])),
            (
                "value",
                ConfigValue::Indexed(vec![
                    text("rifle_scope_dict_id"),
                    ConfigValue::Integer(dict_id.into()),
                ]),
            ),
            ("type", ConfigValue::Integer(3)),
        ]),
    };
    named(vec![
        ("parameters", named(parameters)),
        (
            "modifications",
            ConfigValue::Indexed(vec![named(vec![
                ("id", ConfigValue::Integer(row_id)),
                ("modifiers", ConfigValue::Indexed(vec![modifier])),
            ])]),
        ),
        ("data", scope_data(&format!("v{dict_id}"))),
        (FIT_KEY, fit),
    ])
}

fn fit(scope: ConfigValue, weapons: Vec<ConfigValue>) -> ConfigValue {
    named(vec![
        ("scope", scope),
        ("weapons", ConfigValue::Indexed(weapons)),
    ])
}

/// `(dict id, name, base_cost, config)` of each added item.
fn added() -> Vec<(u16, &'static str, u32, ConfigValue)> {
    use Puts::{Add, Set};
    let number = |id: u16| ConfigValue::Integer(id.into());
    let shop = |name: &str| {
        named(vec![
            (
                "parameters",
                named(vec![("weight", ConfigValue::Float(0.3))]),
            ),
            ("data", scope_data(name)),
        ])
    };
    let ak = || vec![text("ak_74n"), text("ak_74n_premium")];
    let variant = |dict_id: u16, name, modification, own, puts, scope, weapons| {
        let config = variant_config(dict_id, modification, own, puts, fit(scope, weapons));
        (dict_id, name, 0, config)
    };
    vec![
        (PSO_1, "scope_pso_1", PSO_1_COST, shop("pso_1")),
        (COBRA, "scope_cobra_ekp_8", COBRA_COST, shop("cobra_ekp_8")),
        (LONE_SCOPE, "scope_lone", 9_000, shop("lone")),
        variant(
            AK_74N_PSO_1,
            "fit_ak_74n_pso_1",
            Some(992_001),
            true,
            Add,
            number(PSO_1),
            ak(),
        ),
        variant(
            AK_74N_COBRA,
            "fit_ak_74n_cobra_ekp_8",
            Some(992_003),
            true,
            Add,
            text("scope_cobra_ekp_8"),
            ak(),
        ),
        variant(
            SVD_COBRA,
            "fit_svd_cobra_ekp_8",
            Some(992_029),
            true,
            Set,
            number(COBRA),
            vec![number(63)],
        ),
        variant(
            BROKEN[0],
            "fit_no_modification",
            None,
            true,
            Add,
            number(PSO_1),
            ak(),
        ),
        variant(
            BROKEN[1],
            "fit_modification_not_its_own",
            Some(992_091),
            false,
            Add,
            number(PSO_1),
            ak(),
        ),
        variant(
            BROKEN[2],
            "fit_weapon_as_scope",
            Some(992_092),
            true,
            Add,
            text("ak_74m"),
            ak(),
        ),
        variant(
            BROKEN[3],
            "fit_on_a_vest",
            Some(992_093),
            true,
            Add,
            number(PSO_1),
            vec![text("edge_torso_2")],
        ),
        variant(
            BROKEN[4],
            "fit_twice",
            Some(992_094),
            true,
            Add,
            number(PSO_1),
            vec![text("ak_74n")],
        ),
        variant(
            BROKEN[5],
            "fit_variant_as_scope",
            Some(992_095),
            true,
            Add,
            number(AK_74N_PSO_1),
            vec![text("ak_74m")],
        ),
        variant(
            BROKEN[6],
            "fit_adds_on_the_svd",
            Some(992_096),
            true,
            Add,
            number(PSO_1),
            vec![text("svd")],
        ),
        variant(
            BROKEN[7],
            "fit_sets_on_the_ak_74m",
            Some(992_097),
            true,
            Set,
            number(PSO_1),
            vec![text("ak_74m")],
        ),
        variant(
            BROKEN[8],
            "fit_stock_scope_sold",
            Some(992_098),
            true,
            Add,
            number(304),
            ak(),
        ),
    ]
}

/// The weapon configs of [`PLACEHOLDERS`] with `addons { mount: 1 }`, as
/// the patch lays them: `(path, bytes)`.
fn placeholder_configs() -> Vec<(String, Vec<u8>)> {
    let stock = game_data::shared();
    PLACEHOLDERS
        .iter()
        .map(|name| {
            let path = Dictionary::shared()
                .item_named(name)
                .unwrap()
                .cfg_name
                .clone();
            let mut config = ConfigValue::parse(stock.file(&path).unwrap()).unwrap();
            let ConfigValue::Named(root) = &mut config else {
                panic!("{path} is not a named table");
            };
            assert!(root.iter().all(|(key, _)| key != "addons"), "{path}");
            root.push((
                "addons".to_owned(),
                named(vec![("mount", ConfigValue::Integer(1))]),
            ));
            (path, config.to_bytes().unwrap())
        })
        .collect()
}

/// The test data ([`game_data::shared`]) with the scope items and variants
/// of [`added`] in a new dictionary and their configs, as a patch lays them,
/// and an `items_compatibility` row for each weapon and the scope items it
/// takes ([`compatibility_rows`]), as the lobby movie's proposal adds them
/// (`docs/notes/weapon_scopes.md` §4).
pub fn with_scopes() -> GameData {
    with_scopes_and_slots(&[])
}

/// The weapon and scope item of each `items_compatibility` row the
/// fixture adds, as the release plans them: one for each pair its variants
/// make, and one for each weapon and its stock scope ([`STOCK_ROWS`]: the
/// lobby movie shows a greyed "standard scope" in an empty slot).
pub fn compatibility_rows() -> Vec<(u16, u16)> {
    let dict = |name: &str| Dictionary::shared().item_named(name).unwrap().dict_id;
    let (ak, ak_premium, svd) = (dict("ak_74n"), dict("ak_74n_premium"), dict("svd"));
    let mut rows = vec![
        (ak, PSO_1),
        (ak_premium, PSO_1),
        (ak, COBRA),
        (ak_premium, COBRA),
        (svd, COBRA),
    ];
    rows.extend(
        STOCK_ROWS
            .iter()
            .map(|&(weapon, stock)| (dict(weapon), stock)),
    );
    rows
}

/// Each 0.26g weapon with a stock scope and that scope (its config's
/// `addons.rifle_scope_dict_id`): seven rows.
pub const STOCK_ROWS: [(&str, u16); 7] = [
    ("rem_700", 69),
    ("svd", 304),
    ("vsk_94", 304),
    ("vss", 304),
    ("vss_premium", 304),
    ("sv_98", 362),
    ("vssk", 362),
];

/// [`with_scopes`], and `slots` as `(category, profile slot)` rows of
/// `slots_restrictions` (a data mistake a test makes).
pub fn with_scopes_and_slots(slots: &[(u8, u8)]) -> GameData {
    let stock = game_data::shared();
    let mut dictionary_config = ConfigValue::parse(stock.file(dictionary::PATH).unwrap()).unwrap();
    let ConfigValue::Named(root) = &mut dictionary_config else {
        panic!("the dictionary is not a named table");
    };
    let mut files = placeholder_configs();
    for (table, entry) in root.iter_mut() {
        match (table.as_str(), entry) {
            ("items_dict", ConfigValue::Indexed(rows)) => {
                for (dict_id, name, cost, config) in added() {
                    let cfg_name = format!("gameplay/items/scopes/{name}.options");
                    rows.push(named(vec![
                        ("cfg_name", text(&cfg_name)),
                        ("is_stack", ConfigValue::Boolean(false)),
                        ("name", text(name)),
                        ("item_category", ConfigValue::Integer(SCOPE_CATEGORY.into())),
                        ("dict_id", ConfigValue::Integer(dict_id.into())),
                        ("base_cost", ConfigValue::Integer(cost as i32)),
                        ("is_premium", ConfigValue::Boolean(false)),
                    ]));
                    files.push((cfg_name, config.to_bytes().unwrap()));
                }
            }
            // Named tables in the game's file: `compatibility_<n>`,
            // `restriction_<n>`; a patch's rows need names of their own.
            ("items_compatibility", ConfigValue::Named(rows)) => {
                for (n, (weapon, scope)) in compatibility_rows().into_iter().enumerate() {
                    rows.push((
                        format!("compatibility_scope_{n}"),
                        named(vec![
                            ("first_item", ConfigValue::Integer(weapon.into())),
                            ("second_item", ConfigValue::Integer(scope.into())),
                        ]),
                    ));
                }
            }
            ("slots_restrictions", ConfigValue::Named(rows)) => {
                for (n, &(category, slot)) in slots.iter().enumerate() {
                    rows.push((
                        format!("restriction_scope_{n}"),
                        named(vec![
                            ("category_id", ConfigValue::Integer(category.into())),
                            ("profile_slot_id", ConfigValue::Integer(slot.into())),
                        ]),
                    ));
                }
            }
            ("items_dict" | "items_compatibility" | "slots_restrictions", _) => {
                panic!("{table} is not the table the game's file has")
            }
            _ => {}
        }
    }
    files.push((
        dictionary::PATH.to_owned(),
        dictionary_config.to_bytes().unwrap(),
    ));
    stock.laid_over("the test data and the scopes of a patch", files)
}

/// The dictionary of `data`.
pub fn dictionary_of(data: &GameData) -> Dictionary {
    Dictionary::parse(data.file(dictionary::PATH).unwrap()).unwrap()
}

/// The table of [`with_scopes`], made once.
pub fn fits() -> &'static ScopeFits {
    static FITS: std::sync::OnceLock<ScopeFits> = std::sync::OnceLock::new();
    FITS.get_or_init(|| {
        let data = with_scopes();
        ScopeFits::derive(&data, &dictionary_of(&data)).unwrap().0
    })
}
