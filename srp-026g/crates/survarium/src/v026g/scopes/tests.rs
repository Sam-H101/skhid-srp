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
