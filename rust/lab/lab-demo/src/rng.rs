#[derive(Debug, Clone)]
pub struct Pcg {
    hi: u64,
    lo: u64,
}

impl Pcg {
    #[must_use]
    pub fn new(seed1: u64, seed2: u64) -> Self {
        Self {
            hi: seed1,
            lo: seed2,
        }
    }

    pub fn uint64(&mut self) -> u64 {
        const MUL_HI: u64 = 2_549_297_995_355_413_924;
        const MUL_LO: u64 = 4_865_540_595_714_422_341;
        const INC_HI: u64 = 6_364_136_223_846_793_005;
        const INC_LO: u64 = 1_442_695_040_888_963_407;
        const CHEAP_MUL: u64 = 0xda94_2042_e4dd_58b5;
        let product = u128::from(self.lo) * u128::from(MUL_LO);
        let mut hi = u64::try_from(product >> 64).unwrap_or_default();
        let lo_part = u64::try_from(product & u128::from(u64::MAX)).unwrap_or_default();
        hi = hi
            .wrapping_add(self.hi.wrapping_mul(MUL_LO))
            .wrapping_add(self.lo.wrapping_mul(MUL_HI));
        let (lo, carry) = lo_part.overflowing_add(INC_LO);
        let hi_state = hi.wrapping_add(INC_HI).wrapping_add(u64::from(carry));
        self.lo = lo;
        self.hi = hi_state;
        let mut out = hi_state;
        out ^= out >> 32;
        out = out.wrapping_mul(CHEAP_MUL);
        out ^= out >> 48;
        out.wrapping_mul(lo | 1)
    }

    pub fn float64(&mut self) -> f64 {
        let bits = (self.uint64() << 11) >> 11;
        #[allow(clippy::cast_precision_loss)]
        let value = bits as f64;
        value / 9_007_199_254_740_992.0
    }

    pub fn int_n(&mut self, n: u64) -> u64 {
        if n.is_power_of_two() {
            return self.uint64() & (n - 1);
        }
        let mut product = u128::from(self.uint64()) * u128::from(n);
        let mut lo = u64::try_from(product & u128::from(u64::MAX)).unwrap_or_default();
        if lo < n {
            let thresh = n.wrapping_neg() % n;
            while lo < thresh {
                product = u128::from(self.uint64()) * u128::from(n);
                lo = u64::try_from(product & u128::from(u64::MAX)).unwrap_or_default();
            }
        }
        u64::try_from(product >> 64).unwrap_or_default()
    }
}
