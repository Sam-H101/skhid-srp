//! How fast a game brings its clock to the match clock when it is told how
//! far off it is (0xE7), by the build of the game.
//!
//! The game's clock is a timer (`game`+0x52D8) that runs at `1 + f` of real
//! time while it takes a correction, `1 - f` for one backwards, until the
//! whole of it is made up: `set_offset` 0x1EE9E0 puts the end `offset / f`
//! away. A correction that comes while one runs takes its place: what was
//! left of the first is not made up. `f` is the game's own setting
//! `time_floating_factor` (the console variable at 0xA9D0D0: 0.05, from
//! 0.001 to 0.5), which the `game` constructor copies into the timer once
//! (0x1E9848) and nothing else reads. PROVEN (`docs/notes/match.md` section
//! 29). Nothing of what a match computes depends on it: games of either
//! factor play one match.

/// The original game's `time_floating_factor`: a clock 400 ms behind is
/// right after 8 s.
pub const STOCK_FACTOR: f32 = 0.05;
/// What the client patch sets it to from [`FAST_FROM_REVISION`]
/// (`client-patches/patch_026g.py`, `CLOCK_FACTOR`): 400 ms in 1.3 s.
pub const FAST_FACTOR: f32 = 0.3;
/// The first released patch revision whose exe has [`FAST_FACTOR`].
pub const FAST_FROM_REVISION: u8 = 8;

/// The factor of a game that signed in with this patch revision. An exe
/// built by hand before the late join had a released revision says
/// [`super::late_join::PATCH_REVISION`] and has the original's.
pub fn factor(revision: u8) -> f32 {
    if (FAST_FROM_REVISION..super::late_join::PATCH_REVISION).contains(&revision) {
        FAST_FACTOR
    } else {
        STOCK_FACTOR
    }
}

/// How long a game of `factor` takes to make up a correction of
/// `offset_ms` either way, in ms: never less than the correction itself
/// (no game's factor is over 1).
pub fn correction_ms(offset_ms: u32, factor: f32) -> u64 {
    let factor = f64::from(factor.clamp(0.001, 1.0));
    (f64::from(offset_ms) / factor).ceil() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_before_the_patch_has_the_originals_factor() {
        for revision in [0, 4, 5, 7, super::super::late_join::PATCH_REVISION, 0xFF] {
            assert_eq!(factor(revision), STOCK_FACTOR, "revision {revision}");
        }
        for revision in [8, 9, 0x7F] {
            assert_eq!(factor(revision), FAST_FACTOR, "revision {revision}");
        }
    }

    #[test]
    fn a_correction_takes_its_size_over_the_factor() {
        assert_eq!(correction_ms(400, STOCK_FACTOR), 8_000);
        assert_eq!(correction_ms(465, STOCK_FACTOR), 9_300);
        assert_eq!(correction_ms(300, FAST_FACTOR), 1_000);
        assert_eq!(correction_ms(465, FAST_FACTOR), 1_550);
        assert_eq!(correction_ms(0, FAST_FACTOR), 0);
        // A factor no game has: the clock is never taken to stand still.
        assert_eq!(correction_ms(100, 0.0), 100_000);
        assert_eq!(correction_ms(100, 7.0), 100);
    }
}
