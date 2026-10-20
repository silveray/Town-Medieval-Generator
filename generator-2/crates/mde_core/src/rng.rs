//! Deterministic SplitMix64 RNG.
//!
//! Deliberately tiny and dependency-free: the same seed must give the same town on
//! every platform and in a future C++ port, which rules out `rand`'s
//! version-dependent algorithms.

/// SplitMix64 generator. Cheap to copy; use [`Rng::fork`] to derive independent
/// streams for sub-generators so adding a call in one place does not reshuffle
/// everything downstream.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `[min, max)`.
    pub fn range_f32(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.next_f32()
    }

    /// Uniform integer in `[min, max)`. Returns `min` when the range is empty.
    pub fn range_i32(&mut self, min: i32, max: i32) -> i32 {
        if max <= min {
            return min;
        }
        let span = (max as i64 - min as i64) as u64;
        (min as i64 + (self.next_u64() % span) as i64) as i32
    }

    /// Uniform index in `[0, len)`. Panics if `len == 0`.
    pub fn index(&mut self, len: usize) -> usize {
        assert!(len > 0, "Rng::index on empty range");
        (self.next_u64() % len as u64) as usize
    }

    /// `true` with probability `p`.
    pub fn chance(&mut self, p: f32) -> bool {
        self.next_f32() < p
    }

    /// Approximately normal in `[0, 1)`, mean 0.5: the average of three uniforms.
    pub fn normal3(&mut self) -> f32 {
        (self.next_f32() + self.next_f32() + self.next_f32()) / 3.0
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.index(items.len())]
    }

    /// Index chosen with probability proportional to `weights[i]`.
    /// Returns 0 if all weights are zero or negative.
    pub fn weighted(&mut self, weights: &[f32]) -> usize {
        let total: f32 = weights.iter().map(|w| w.max(0.0)).sum();
        if total <= 0.0 {
            return 0;
        }
        let mut r = self.next_f32() * total;
        for (i, w) in weights.iter().enumerate() {
            let w = w.max(0.0);
            if r < w {
                return i;
            }
            r -= w;
        }
        weights.len() - 1
    }

    /// Fisher–Yates shuffle.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.index(i + 1);
            items.swap(i, j);
        }
    }

    /// Independent child stream; `salt` distinguishes siblings.
    pub fn fork(&mut self, salt: u64) -> Rng {
        Rng::new(self.next_u64() ^ salt.wrapping_mul(0xD6E8_FEB8_6659_FD93))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let a: Vec<u64> = (0..4).scan(Rng::new(42), |r, _| Some(r.next_u64())).collect();
        let b: Vec<u64> = (0..4).scan(Rng::new(42), |r, _| Some(r.next_u64())).collect();
        assert_eq!(a, b);
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
    }

    #[test]
    fn ranges() {
        let mut r = Rng::new(7);
        for _ in 0..10_000 {
            let f = r.next_f32();
            assert!((0.0..1.0).contains(&f));
            let i = r.range_i32(-3, 4);
            assert!((-3..4).contains(&i));
        }
        assert_eq!(r.range_i32(5, 5), 5);
    }

    #[test]
    fn weighted_respects_zero() {
        let mut r = Rng::new(3);
        for _ in 0..1000 {
            assert_ne!(r.weighted(&[1.0, 0.0, 2.0]), 1);
        }
    }
}
