//! 8-bit sequence arithmetic, as the 0.26g client does it (transport.md §1
//! rows 2 and 16, §2.3 step 6): datagram seqs, unique ids and order ids are
//! all one byte and compare in a half range of 128.

/// `a` is newer than `b`: 1 to 127 ahead of it, modulo 256.
pub fn is_newer(a: u8, b: u8) -> bool {
    let distance = a.wrapping_sub(b);
    distance != 0 && distance < 128
}

/// What [`Window::record`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// Not seen before; now in the window.
    New,
    /// Already in the window.
    Duplicate,
    /// Exactly 128 behind the newest: outside the window, so not recorded.
    /// (The client takes such a value as new, 0x3F3AA0.)
    Outside,
}

/// The newest value seen and which of the 127 before it were seen: the
/// client's tracker at conn+0x170 (datagram seqs) and at channel+0x18
/// (unique ids). Bit 127 is the newest itself, bit `127 - k` is `newest - k`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Window {
    newest: Option<u8>,
    bits: u128,
}

impl Window {
    pub fn newest(&self) -> Option<u8> {
        self.newest
    }

    /// The window, bit 127 = the newest. Zero before anything was recorded.
    pub fn bits(&self) -> u128 {
        self.bits
    }

    pub fn contains(&self, value: u8) -> bool {
        let Some(newest) = self.newest else {
            return false;
        };
        let back = newest.wrapping_sub(value);
        back < 128 && self.bits >> (127 - u32::from(back)) & 1 != 0
    }

    pub fn record(&mut self, value: u8) -> Recorded {
        let Some(newest) = self.newest else {
            self.newest = Some(value);
            self.bits = 1 << 127;
            return Recorded::New;
        };
        if is_newer(value, newest) {
            let ahead = u32::from(value.wrapping_sub(newest));
            self.bits = self.bits >> ahead | 1 << 127;
            self.newest = Some(value);
            return Recorded::New;
        }
        let back = u32::from(newest.wrapping_sub(value));
        if back >= 128 {
            return Recorded::Outside;
        }
        let bit = 1u128 << (127 - back);
        if self.bits & bit != 0 {
            return Recorded::Duplicate;
        }
        self.bits |= bit;
        Recorded::New
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_is_one_to_127_ahead() {
        assert!(is_newer(1, 0));
        assert!(is_newer(0, 255));
        assert!(is_newer(127, 0));
        assert!(!is_newer(128, 0));
        assert!(!is_newer(0, 0));
        assert!(!is_newer(255, 0));
        assert!(is_newer(3, 250));
    }

    #[test]
    fn the_window_follows_the_newest_across_the_wrap() {
        let mut window = Window::default();
        assert!(!window.contains(0));
        assert_eq!(window.record(254), Recorded::New);
        assert_eq!(window.record(1), Recorded::New);
        assert_eq!(window.record(255), Recorded::New);
        assert_eq!(window.record(255), Recorded::Duplicate);
        assert_eq!(window.record(1), Recorded::Duplicate);
        assert_eq!(window.newest(), Some(1));
        assert!(window.contains(254) && window.contains(255) && window.contains(1));
        assert!(!window.contains(0));
        assert_eq!(window.bits(), 1 << 127 | 1 << 125 | 1 << 124);
        // 128 behind the newest is outside.
        assert_eq!(window.record(129), Recorded::Outside);
        // A jump of 127 keeps only what is still within reach.
        assert_eq!(window.record(128), Recorded::New);
        assert!(window.contains(1));
        assert!(!window.contains(255));
    }
}
