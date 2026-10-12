// SPDX-License-Identifier: GPL-3.0-or-later
// Division-free implementation for x86 MSVC compiler helper symbols.
pub fn div_rem(mut n: u64, d: u64) -> (u64, u64) {
    assert!(d != 0);
    let (mut q, mut r) = (0u64, 0u64);
    for _ in 0..64 {
        let carry = r >> 63;
        r = (r << 1) | (n >> 63);
        n <<= 1;
        q <<= 1;
        if carry != 0 || r >= d {
            r = r.wrapping_sub(d);
            q |= 1;
        }
    }
    (q, r)
}

#[cfg(test)]
mod tests {
    use super::div_rem;
    #[test]
    fn agrees_with_native_division() {
        let boundaries = [0, 1, 2, 10, u32::MAX as u64, 1 << 32, 1 << 63, u64::MAX];
        for n in boundaries {
            for d in boundaries.into_iter().filter(|d| *d != 0) {
                assert_eq!(div_rem(n, d), (n / d, n % d));
            }
        }
        let mut x = 0x193f_354b_1965_625du64;
        for _ in 0..4096 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let n = x;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let d = x | 1;
            assert_eq!(div_rem(n, d), (n / d, n % d));
        }
    }
}
