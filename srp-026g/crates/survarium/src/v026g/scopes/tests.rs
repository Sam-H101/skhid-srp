use super::fixtures::{
    self, AK_74N_COBRA, AK_74N_PSO_1, BROKEN, COBRA, LONE_SCOPE, PSO_1, SVD_COBRA,
};
use super::*;

fn dict(name: &str) -> u16 {
    Dictionary::shared().item_named(name).unwrap().dict_id
}

#[test]
fn the_test_data_has_no_variant_so_no_pair() {
    // The players' data before the release patch: no scope is offered.
    let (fits, left_out) = ScopeFits::derive(game_data::shared(), Dictionary::shared()).unwrap();
    assert!(fits.is_empty(), "{fits:?}");
    assert!(left_out.is_empty(), "{left_out:?}");
    assert!(ScopeFits::shared().is_empty());
    // 0.26g's own three scopes are no shop items.
    for stock in [69, 304, 362] {
        assert!(!ScopeFits::shared().is_scope(stock));
    }
}

#[test]
fn the_pairs_are_read_from_the_variants_configs() {
    let data = fixtures::with_scopes();
    let dictionary = fixtures::dictionary_of(&data);
    let (fits, left_out) = ScopeFits::derive(&data, &dictionary).unwrap();
    let (ak, ak_premium, svd) = (dict("ak_74n"), dict("ak_74n_premium"), dict("svd"));
    // By number or by name, one variant for the weapon and its premium copy.
    for weapon in [ak, ak_premium] {
        assert_eq!(fits.variant(weapon, PSO_1), Some(AK_74N_PSO_1));
        assert_eq!(fits.variant(weapon, COBRA), Some(AK_74N_COBRA));
    }
    assert_eq!(fits.variant(svd, COBRA), Some(SVD_COBRA));
    assert_eq!(fits.variant(svd, PSO_1), None);
    assert_eq!(fits.variant(dict("ak_74m"), PSO_1), None);
    assert_eq!(fits.all().len(), 5);
    assert!(fits.fits(svd, SVD_COBRA) && !fits.fits(ak, SVD_COBRA));
    assert_eq!(fits.scopes(), [PSO_1, COBRA]);
    let mut weapons = vec![ak, ak_premium, svd];
    weapons.sort_unstable();
    assert_eq!(fits.weapons(), weapons);
    assert!(fits.is_scope(PSO_1) && !fits.is_scope(LONE_SCOPE) && !fits.is_scope(AK_74N_PSO_1));
    assert!(fits.is_variant(AK_74N_PSO_1) && !fits.is_variant(PSO_1));
    // Every broken one is left out, with why, and nothing else is.
    for broken in BROKEN {
        assert!(!fits.is_variant(broken), "{broken}");
        assert!(
            left_out.iter().any(|why| why.contains(&broken.to_string())),
            "{broken}: {left_out:?}"
        );
    }
    assert_eq!(left_out.len(), BROKEN.len(), "{left_out:#?}");
    assert!(
        left_out
            .iter()
            .any(|why| why.contains("has variant 2001 on it already"))
    );
    assert_eq!(fixtures::fits(), &fits);
}

#[test]
fn a_pair_given_twice_keeps_the_lower_variant() {
    let fit = |variant| Fit {
        weapon: 63,
        scope: PSO_1,
        variant,
    };
    let (fits, twice) = ScopeFits::new(vec![fit(2050), fit(2007), fit(2030)]);
    assert_eq!(fits.all(), [fit(2007)]);
    assert_eq!(twice, [fit(2030), fit(2050)]);
}

/// The lobby movie's proposal adds an `items_compatibility` row for each
/// weapon and the scope items it takes. srp reads those rows in one place,
/// `check_play`, and only for items in a kit's ammo slots; no scope can be
/// in a kit slot, and the start check refuses data that would let one.
#[test]
fn compatibility_rows_of_scopes_leave_srp_as_it_was() {
    use super::super::dictionary::PROFILE_SLOTS;
    use super::super::tables;
    let data = fixtures::with_scopes();
    let dictionary = fixtures::dictionary_of(&data);
    let rows = fixtures::compatibility_rows();
    assert!(!rows.is_empty());
    for (weapon, scope) in rows {
        assert!(dictionary.compatible(weapon, scope) && dictionary.compatible(scope, weapon));
    }
    // The pairs come from the variants alone.
    assert_eq!(
        &ScopeFits::derive(&data, &dictionary).unwrap().0,
        fixtures::fits()
    );
    // No scope item or variant goes into any kit slot.
    let scopes = dictionary
        .items
        .iter()
        .filter(|item| item.category == SCOPE_CATEGORY);
    for item in scopes {
        for slot in 0..PROFILE_SLOTS as u8 {
            assert!(
                !dictionary.fits(item.dict_id, slot),
                "{} {slot}",
                item.dict_id
            );
        }
    }
    assert_eq!(tables::check(&data), Ok(()));
    // Data with a slot for scopes stops srp at start.
    let wrong = fixtures::with_scopes_and_slots(&[(SCOPE_CATEGORY, 8)]);
    let refused = tables::check(&wrong).unwrap_err();
    assert!(refused.0.contains("into profile slot 8"), "{refused}");
}

/// The weapon's scope as the client reads it (`addons.rifle_scope_dict_id`,
/// 0x24AA0C): none is a bare weapon.
fn scope_of(weapon: &ConfigValue) -> Option<i32> {
    weapon.get("addons")?.get("rifle_scope_dict_id")?.as_i32()
}

/// `weapon` with the modification of a record of `variant` applied as the
/// client applies a `type` 0 modifier (0x357F10: the node takes the value).
fn with_record(weapon: &ConfigValue, variant: &ConfigValue) -> ConfigValue {
    let modification = variant
        .get("parameters")
        .and_then(|parameters| parameters.get("modification"))
        .and_then(ConfigValue::as_i32)
        .unwrap();
    let row = variant
        .get("modifications")
        .unwrap()
        .children()
        .into_iter()
        .find(|row| row.get("id").and_then(ConfigValue::as_i32) == Some(modification))
        .unwrap();
    let modifier = row.get("modifiers").unwrap().children()[0];
    assert_eq!(modifier.get("type").and_then(ConfigValue::as_i32), Some(0));
    let mut built = weapon.clone();
    let ConfigValue::Named(root) = &mut built else {
        panic!("a weapon config is a named table");
    };
    let (_, addons) = root.iter_mut().find(|(key, _)| key == "addons").unwrap();
    let ConfigValue::Named(addons) = addons else {
        panic!("addons is a named table");
    };
    let (_, scope) = addons
        .iter_mut()
        .find(|(key, _)| key == "rifle_scope_dict_id")
        .unwrap();
    *scope = modifier.get("value").unwrap().clone();
    built
}

/// The owner's rule (2026-10-17): a bought scope may replace a stock one.
/// On the SVD (stock PSO-1, 304) the variant's record sets the scope to the
/// variant; with the record off the client builds the plain config, which
/// has the stock PSO-1 again: never a bare SVD. A stock scope is never a
/// shop scope: not sold, never put on.
#[test]
fn a_bought_scope_replaces_a_stock_one_and_gives_it_back() {
    let data = fixtures::with_scopes();
    let dictionary = fixtures::dictionary_of(&data);
    let fits = fixtures::fits();
    let config = |dict_id: u16| {
        let path = &dictionary.item(dict_id).unwrap().cfg_name;
        game_data::config(&data, path).unwrap().unwrap()
    };
    let svd = dict("svd");
    let plain = config(svd);
    assert_eq!(scope_of(&plain), Some(304));
    let variant = fits.variant(svd, COBRA).unwrap();
    assert_eq!(variant, SVD_COBRA);
    let worn = with_record(&plain, &config(variant));
    assert_eq!(scope_of(&worn), Some(i32::from(SVD_COBRA)));
    // Taken off: no record, the plain config, the stock PSO-1.
    assert_eq!(scope_of(&plain), Some(304));
    // An add on the SVD (the client would assert on a second
    // `rifle_scope_dict_id`) is left out, as is the stock PSO-1 named as a
    // shop scope.
    assert!(!fits.is_variant(BROKEN[6]) && !fits.is_variant(BROKEN[8]));
    // No stock scope is a shop scope, and none goes on any weapon.
    let stock_scopes: Vec<u16> = dictionary
        .items
        .iter()
        .filter(|item| WEAPON_CATEGORIES.contains(&item.category))
        .filter_map(|item| scope_of(&config(item.dict_id)))
        .map(|id| id as u16)
        .collect();
    let mut found = stock_scopes.clone();
    found.sort_unstable();
    found.dedup();
    assert_eq!(found, [69, 304, 362]);
    for stock in found {
        assert!(!fits.is_scope(stock) && !fits.is_variant(stock), "{stock}");
        for weapon in fits.weapons() {
            assert!(
                super::super::modifications::upgrade_slots_in(fits, weapon, stock).is_empty(),
                "{stock} on {weapon}"
            );
        }
    }
}
