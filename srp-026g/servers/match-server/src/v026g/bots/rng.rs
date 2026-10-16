//! Each bot's own random numbers: SplitMix64, seeded from the match and
//! the place, so a match's bots repeat what they did when it is replayed
//! with the same worlds (`docs/notes/player_bots_research.md` B3).

#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The bot at `place` of match `match_id`.
    pub fn for_bot(match_id: u32, place: usize) -> Self {
        Self::new((u64::from(match_id) << 32) ^ (place as u64).wrapping_mul(0x9E37_79B9))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// In `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// In `[low, high)`.
    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }

    /// In `0..n`; 0 when `n` is 0.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_numbers_and_ranges_hold() {
        let (mut a, mut b) = (Rng::for_bot(0x1234, 3), Rng::for_bot(0x1234, 3));
        let mut other = Rng::for_bot(0x1234, 4);
        let first: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        assert_eq!(first, (0..8).map(|_| b.next_u64()).collect::<Vec<_>>());
        assert_ne!(first, (0..8).map(|_| other.next_u64()).collect::<Vec<_>>());
        for _ in 0..1000 {
            let x = a.range(-2.0, 3.0);
            assert!((-2.0..3.0).contains(&x));
            assert!(a.below(7) < 7);
        }
    }
}
