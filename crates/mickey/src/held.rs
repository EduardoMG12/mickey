//! The set of keys or buttons currently held down, so that losing the
//! connection can release exactly those.

/// One bit per evdev code. 768 bits cover every code up to `KEY_MAX` (0x2ff).
pub struct HeldSet {
    bits: [u64; 12],
}

impl HeldSet {
    const CAPACITY: usize = 12 * 64;

    pub fn new() -> HeldSet {
        HeldSet { bits: [0; 12] }
    }

    pub fn set(&mut self, code: u16, pressed: bool) {
        let code = usize::from(code);
        if code >= Self::CAPACITY {
            return;
        }
        let (word, bit) = (code / 64, code % 64);
        if pressed {
            self.bits[word] |= 1 << bit;
        } else {
            self.bits[word] &= !(1 << bit);
        }
    }

    /// Calls `release` for every held code, lowest first, and empties the set.
    pub fn release_all(&mut self, mut release: impl FnMut(u16)) {
        for code in 0..Self::CAPACITY {
            if self.bits[code / 64] & (1 << (code % 64)) != 0 {
                // CAPACITY is 768, so every code fits in a u16.
                release(code as u16);
            }
        }
        self.bits = [0; 12];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(set: &mut HeldSet) -> Vec<u16> {
        let mut codes = Vec::new();
        set.release_all(|code| codes.push(code));
        codes
    }

    #[test]
    fn releases_only_what_is_still_held() {
        let mut set = HeldSet::new();
        set.set(30, true);
        set.set(0x110, true);
        set.set(63, true);
        set.set(64, true);
        set.set(63, false);
        assert_eq!(held(&mut set), vec![30, 64, 0x110]);
        assert_eq!(held(&mut set), Vec::<u16>::new());
    }

    #[test]
    fn codes_past_the_capacity_are_ignored() {
        let mut set = HeldSet::new();
        set.set(u16::MAX, true);
        assert_eq!(held(&mut set), Vec::<u16>::new());
    }
}
