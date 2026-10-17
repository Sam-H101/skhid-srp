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
pub const PSO_1_COST: u32 = 24_000;
pub const COBRA_COST: u32 = 12_000;
/// The PSO-1 made for the AK-74N (and its premium copy), named by number.
pub const AK_74N_PSO_1: u16 = 2001;
/// The Cobra made for the AK-74N, named by name.
pub const AK_74N_COBRA: u16 = 2003;
/// The Cobra made for the SVD, a weapon with a stock scope (PSO-1, 304).
pub const SVD_COBRA: u16 = 2029;
/// Variants the table leaves out: no modification, a modification of no
/// row of its own, a weapon as its scope, a vest as its weapon, a pair
/// another variant has (higher id), a variant as its scope.
pub const BROKEN: [u16; 6] = [2090, 2091, 2092, 2093, 2094, 2095];

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

/// A variant's config: its modification adds `rifle_scope_dict_id` = itself
/// to the weapon's `addons` (`own` false: the modification is another
/// id's), and `fit` is its [`FIT_KEY`] block.
fn variant_config(
    dict_id: u16,
    modification: Option<i32>,
    own: bool,
    fit: ConfigValue,
) -> ConfigValue {
    let id = modification.unwrap_or(990_000 + i32::from(dict_id));
    let mut parameters = vec![("weight", ConfigValue::Float(0.3))];
    if let Some(modification) = modification {
        parameters.push(("modification", ConfigValue::Integer(modification)));
    }
    let row_id = if own { id } else { id + 1 };
    let modifier = named(vec![
        ("path", ConfigValue::Indexed(vec![text("addons")])),
        (
            "value",
            ConfigValue::Indexed(vec![
                text("rifle_scope_dict_id"),
                ConfigValue::Integer(dict_id.into()),
            ]),
        ),
        ("type", ConfigValue::Integer(3)),
    ]);
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
    vec![
        (PSO_1, "scope_pso_1", PSO_1_COST, shop("pso_1")),
        (COBRA, "scope_cobra_ekp_8", COBRA_COST, shop("cobra_ekp_8")),
        (LONE_SCOPE, "scope_lone", 9_000, shop("lone")),
        (
            AK_74N_PSO_1,
            "fit_ak_74n_pso_1",
            0,
            variant_config(AK_74N_PSO_1, Some(992_001), true, fit(number(PSO_1), ak())),
        ),
        (
            AK_74N_COBRA,
            "fit_ak_74n_cobra_ekp_8",
            0,
            variant_config(
                AK_74N_COBRA,
                Some(992_003),
                true,
                fit(text("scope_cobra_ekp_8"), ak()),
            ),
        ),
        (
            SVD_COBRA,
            "fit_svd_cobra_ekp_8",
            0,
            variant_config(
                SVD_COBRA,
                Some(992_029),
                true,
                fit(number(COBRA), vec![number(63)]),
            ),
        ),
        (
            BROKEN[0],
            "fit_no_modification",
            0,
            variant_config(BROKEN[0], None, true, fit(number(PSO_1), ak())),
        ),
        (
            BROKEN[1],
            "fit_modification_not_its_own",
            0,
            variant_config(BROKEN[1], Some(992_091), false, fit(number(PSO_1), ak())),
        ),
        (
            BROKEN[2],
            "fit_weapon_as_scope",
            0,
            variant_config(BROKEN[2], Some(992_092), true, fit(text("ak_74m"), ak())),
        ),
        (
            BROKEN[3],
            "fit_on_a_vest",
            0,
            variant_config(
                BROKEN[3],
                Some(992_093),
                true,
                fit(number(PSO_1), vec![text("edge_torso_2")]),
            ),
        ),
        (
            BROKEN[4],
            "fit_twice",
            0,
            variant_config(
                BROKEN[4],
                Some(992_094),
                true,
                fit(number(PSO_1), vec![text("ak_74n")]),
            ),
        ),
        (
            BROKEN[5],
            "fit_variant_as_scope",
            0,
            variant_config(
                BROKEN[5],
                Some(992_095),
                true,
                fit(number(AK_74N_PSO_1), vec![text("ak_74m")]),
            ),
        ),
    ]
}

/// The test data ([`game_data::shared`]) with the scope items and variants
/// of [`added`] in a new dictionary and their configs, as a patch lays them.
pub fn with_scopes() -> GameData {
    let stock = game_data::shared();
    let mut dictionary_config = ConfigValue::parse(stock.file(dictionary::PATH).unwrap()).unwrap();
    let ConfigValue::Named(root) = &mut dictionary_config else {
        panic!("the dictionary is not a named table");
    };
    let (_, items) = root
        .iter_mut()
        .find(|(name, _)| name == "items_dict")
        .expect("items_dict");
    let ConfigValue::Indexed(rows) = items else {
        panic!("items_dict is not a list");
    };
    let mut files = Vec::new();
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
