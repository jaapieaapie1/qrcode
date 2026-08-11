//! The float routines the pipeline needs that `core` does not provide.
//!
//! `core` already carries `abs`, `min`, `max`, `clamp`, `signum`, `copysign`,
//! `to_radians` and the classification predicates, because none of them need
//! more than sign and comparison logic. What it does not carry is anything that
//! would pull in a math library. This module implements exactly the four the
//! pipeline calls — `sqrt`, `ceil`, `round`, `sin_cos` — and nothing else:
//! `floor`, `atan2`, `ln`, `hypot` and the rest are absent because no
//! production path forms them. (`ln` and standalone `sin`/`cos` appear in test
//! fixtures, which only ever build with `std`.)
//!
//! # Two implementations, one interface
//!
//! * With the `std` feature (the default) each function forwards to the `std`
//!   method of the same name. Existing users get bit-for-bit the arithmetic they
//!   had before this module existed; there is no behaviour change to opt out of.
//! * Without `std` — the ESP32-class build — the hand-rolled [`soft`] versions
//!   run instead.
//!
//! [`soft`] is compiled into test builds either way, so that the native test
//! suite can diff it against `std` across dense ranges, and building with
//! `RUSTFLAGS='--cfg qr_detect_soft_float'` routes the *whole pipeline* through
//! it on a `std` host, which is how the detection corpus is verified to reach
//! the same answers on both paths.
//!
//! # Why free functions rather than an extension trait
//!
//! A trait would keep the `x.sqrt()` call syntax, but inherent methods win
//! method resolution: on a `std` build `f64`'s own `sqrt` would silently shadow
//! the trait's, and `--cfg qr_detect_soft_float` would test nothing at all. Free
//! functions cannot be shadowed, so the switch is honest.
//!
//! # Accuracy and domain
//!
//! Verified against `std` in `tests` below, over dense sweeps plus the edge
//! cases each function has:
//!
//! | function | domain verified | worst observed error |
//! |---|---|---|
//! | `sqrt` | `[0, f64::MAX]` incl. subnormals | 1 ulp (rel. < 2.3e-16) |
//! | `ceil`, `round` | all finite, plus +/-0, +/-inf, NaN | exact (bit-for-bit) |
//! | `sin_cos` | `[-2^20, 2^20]` | < 3e-16 absolute |
//!
//! Those are three to nine orders of magnitude tighter than the target the
//! geometry needs (a pixel coordinate carries about 1e-3 of meaningful
//! precision), so the bare-metal path is not the accuracy bottleneck anywhere.
//!
//! Outside the stated trig domain (|x| > 2^20) the reduction gives up and
//! returns NaN — it does not degrade gracefully like std. That is safe here
//! because the pipeline never forms an angle outside `[-2pi, 2pi]` (all three
//! call sites audited: compile-time-bounded constants, nothing data-derived).
//! Any new caller must respect that domain. Non-finite inputs return NaN, and
//! `sqrt` follows IEEE-754 for negatives (NaN), zeros (signed zero back) and
//! infinity.

/// The hand-rolled path is live when there is no `std` to delegate to, or when
/// the verification switch forces it.
#[cfg(any(not(feature = "std"), qr_detect_soft_float))]
macro_rules! dispatch {
    ($($(#[$attr:meta])* $name:ident($($arg:ident: f64),*) -> $ret:ty;)*) => {
        $(
            $(#[$attr])*
            #[inline]
            pub(crate) fn $name($($arg: f64),*) -> $ret { soft::$name($($arg),*) }
        )*
    };
}

/// The delegating path: `std`'s own methods, so nothing about an existing build
/// changes.
#[cfg(all(feature = "std", not(qr_detect_soft_float)))]
macro_rules! dispatch {
    ($($(#[$attr:meta])* $name:ident($($arg:ident: f64),*) -> $ret:ty;)*) => {
        $(
            $(#[$attr])*
            #[inline]
            pub(crate) fn $name($($arg: f64),*) -> $ret { f64::$name($($arg),*) }
        )*
    };
}

dispatch! {
    sqrt(x: f64) -> f64;
    ceil(x: f64) -> f64;
    round(x: f64) -> f64;
    /// Only the Micro QR path forms an angle, so the trigonometry — and the
    /// range reduction and polynomials behind it — exists exactly when `micro`
    /// does.
    #[cfg(feature = "micro")]
    sin_cos(x: f64) -> (f64, f64);
}

/// The dependency-free implementations.
///
/// Compiled whenever they are reachable: on a `no_std` build, under the
/// verification switch, and in `cfg(test)` — the last so that a default `std`
/// test run still diffs them against `std` even though the pipeline itself is
/// not routed through them. A default `std` *build* leaves them out entirely.
#[cfg(any(not(feature = "std"), qr_detect_soft_float, test))]
pub(crate) mod soft {
    /// Sign bit; everything else masked off.
    const SIGN: u64 = 0x8000_0000_0000_0000;
    /// The 52 significand bits.
    const MANTISSA: u64 = 0x000f_ffff_ffff_ffff;

    // -----------------------------------------------------------------------
    // sqrt
    // -----------------------------------------------------------------------

    /// Scale applied to a subnormal before the iteration, and the matching
    /// scale-back on the result. `2^106` lands every subnormal comfortably
    /// inside the normal range and is an exact power of two, so the round trip
    /// costs no accuracy at all.
    const SUBNORMAL_SCALE: f64 = (1u64 << 53) as f64 * (1u64 << 53) as f64;
    /// `sqrt(2^106) = 2^53`, undone by multiplying by `2^-53`.
    const SUBNORMAL_UNSCALE: f64 = 1.0 / (1u64 << 53) as f64;

    /// Halving the biased exponent in place gives a seed within a few percent
    /// of the true root; the additive constant re-centres the bias and absorbs
    /// the mantissa's contribution. Four Newton steps then take a 3.5% seed to
    /// the last ulp (each step squares the relative error: 3.5e-2, 6e-4,
    /// 1.8e-7, 1.6e-14), and the fifth is what pins the final rounding.
    const SQRT_SEED_BIAS: u64 = 0x1ff7_a7ef_39ef_3000;

    /// Square root, to within an ulp.
    ///
    /// Newton-Raphson on `y <- (y + x/y) / 2` from a bit-trick seed. The
    /// iteration is self-correcting — an error in the seed is squared away
    /// rather than propagated — so the only accuracy that matters is the
    /// rounding of the final step.
    pub(crate) fn sqrt(x: f64) -> f64 {
        // NaN, negatives (including -0.0's neighbours, but not -0.0 itself).
        if x.is_nan() || x < 0.0 {
            return f64::NAN;
        }
        // +/-0.0 returns itself with its sign, and +inf returns +inf: both are
        // fixed points the iteration cannot produce (it would divide by zero,
        // or subtract inf from inf).
        if x == 0.0 || x == f64::INFINITY {
            return x;
        }
        let (x, unscale) = if x < f64::MIN_POSITIVE {
            // A subnormal's leading zeros would wreck the exponent-halving
            // seed, so lift it into the normal range first.
            (x * SUBNORMAL_SCALE, SUBNORMAL_UNSCALE)
        } else {
            (x, 1.0)
        };
        let mut y = f64::from_bits((x.to_bits() >> 1) + SQRT_SEED_BIAS);
        for _ in 0..5 {
            y = 0.5 * (y + x / y);
        }
        y * unscale
    }

    // -----------------------------------------------------------------------
    // rounding
    // -----------------------------------------------------------------------

    /// Truncation toward zero, by masking off the fractional significand bits.
    ///
    /// Not part of the public dispatch (no caller forms it), but [`ceil`] and
    /// [`round`] are both defined against it.
    fn trunc(x: f64) -> f64 {
        let bits = x.to_bits();
        // Unbiased exponent. Non-finite values land at 1024 and so take the
        // `>= 52` arm, which returns them unchanged — correct for both.
        let exp = ((bits >> 52) & 0x7ff) as i64 - 1023;
        if exp < 0 {
            // |x| < 1: the integer part is zero, carrying x's sign.
            return f64::from_bits(bits & SIGN);
        }
        if exp >= 52 {
            // Every significand bit already has integer weight.
            return x;
        }
        f64::from_bits(bits & !(MANTISSA >> exp))
    }

    /// Smallest integer `>= x`.
    ///
    /// `t != x` filters out the exact-integer case *and* NaN, which is what
    /// lets both fall through unchanged.
    pub(crate) fn ceil(x: f64) -> f64 {
        let t = trunc(x);
        if x > 0.0 && t != x { t + 1.0 } else { t }
    }

    /// Nearest integer, halves away from zero — `std`'s rule.
    ///
    /// Deliberately not `(x + 0.5).floor()`: that form rounds
    /// `0.49999999999999994` (the largest double below a half) up to 1, because
    /// the addition itself rounds to exactly 0.5. Taking the fractional part
    /// first cannot: `x - trunc(x)` is exact in binary floating point, so the
    /// comparison against 0.5 sees the true fraction.
    pub(crate) fn round(x: f64) -> f64 {
        let t = trunc(x);
        let frac = x - t;
        if frac >= 0.5 {
            t + 1.0
        } else if frac <= -0.5 {
            t - 1.0
        } else {
            // Includes NaN (every comparison false) and both infinities (whose
            // fraction is NaN), each of which `trunc` already returned intact.
            t
        }
    }

    // -----------------------------------------------------------------------
    // trigonometry
    // -----------------------------------------------------------------------

    /// Re-exported flat, so callers write `soft::sin_cos` and never see the
    /// module the `micro` gate needs in order to cover the whole block at once.
    #[cfg(feature = "micro")]
    pub(crate) use trig::sin_cos;

    /// Range reduction and the two kernel polynomials.
    ///
    /// Gated as a unit: only the Micro QR path forms an angle, so on a build
    /// without `micro` none of this is reachable and none of it is compiled.
    #[cfg(feature = "micro")]
    mod trig {
        use super::round;

        /// `2/pi`, for picking the quadrant.
        const FRAC_2_PI: f64 = core::f64::consts::FRAC_2_PI;

        /// `pi/2` split into three pieces whose sum carries far more than 53 bits.
        ///
        /// `PIO2_HI` has its low 33 significand bits clear, so `k * PIO2_HI` is
        /// *exact* for every quadrant index up to 2^20 and the subtraction
        /// `x - k * PIO2_HI` cancels without introducing any error of its own. The
        /// two tails then restore the bits `PIO2_HI` dropped. (Constants as in
        /// fdlibm's `__ieee754_rem_pio2`.)
        const PIO2_HI: f64 = 1.570_796_326_734_125_6;
        const PIO2_MID: f64 = 6.077_100_506_303_966e-11;
        const PIO2_LO: f64 = 2.022_266_248_795_950_6e-21;

        /// Minimax-style coefficients for `sin` on `[-pi/4, pi/4]` (fdlibm's
        /// `__kernel_sin`); the polynomial's own error there is below 2^-58.
        const S1: f64 = -1.666_666_666_666_663_2e-1;
        const S2: f64 = 8.333_333_333_322_49e-3;
        const S3: f64 = -1.984_126_982_985_795e-4;
        const S4: f64 = 2.755_731_370_707_006_8e-6;
        const S5: f64 = -2.505_076_025_340_686_3e-8;
        const S6: f64 = 1.589_690_995_211_55e-10;

        /// The same for `cos` (fdlibm's `__kernel_cos`), applied to the `x^4` tail
        /// after the exactly-representable `1 - x^2/2` head.
        const C1: f64 = 4.166_666_666_666_66e-2;
        const C2: f64 = -1.388_888_888_887_411e-3;
        const C3: f64 = 2.480_158_728_947_673e-5;
        const C4: f64 = -2.755_731_435_139_066_3e-7;
        const C5: f64 = 2.087_572_321_298_175e-9;
        const C6: f64 = -1.135_964_755_778_819_5e-11;

        /// `sin` on the reduced argument `r` in `[-pi/4, pi/4]`.
        fn kernel_sin(r: f64) -> f64 {
            let z = r * r;
            r + r * z * (S1 + z * (S2 + z * (S3 + z * (S4 + z * (S5 + z * S6)))))
        }

        /// `cos` on the reduced argument `r` in `[-pi/4, pi/4]`.
        fn kernel_cos(r: f64) -> f64 {
            let z = r * r;
            1.0 - 0.5 * z + z * z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6)))))
        }

        /// Beyond this the three-way `pi/2` split no longer covers the quadrant
        /// index exactly and the reduction starts shedding bits. The pipeline's
        /// angles all live in `[-2pi, 2pi]`, six orders of magnitude inside it.
        const REDUCTION_LIMIT: f64 = (1u64 << 20) as f64;

        /// Reduces `x` to `(quadrant mod 4, r)` with `r` in `[-pi/4, pi/4]`.
        ///
        /// Returns `None` for anything the reduction cannot honestly handle:
        /// non-finite input, or a magnitude past [`REDUCTION_LIMIT`].
        fn reduce(x: f64) -> Option<(u8, f64)> {
            if !x.is_finite() || x.abs() > REDUCTION_LIMIT {
                return None;
            }
            let k = round(x * FRAC_2_PI);
            // Cody-Waite: subtract the exact head first, then the two tails.
            let r = ((x - k * PIO2_HI) - k * PIO2_MID) - k * PIO2_LO;
            // `k` is bounded by REDUCTION_LIMIT, so this cast cannot saturate.
            Some(((k as i64).rem_euclid(4) as u8, r))
        }

        /// `(sin(x), cos(x))`.
        ///
        /// One reduction serves both, which is the whole reason the pipeline calls
        /// this rather than the two separately — and the reason there is no
        /// standalone `sin` or `cos` here: nothing forms one without the other.
        pub(crate) fn sin_cos(x: f64) -> (f64, f64) {
            let Some((quadrant, r)) = reduce(x) else {
                return (f64::NAN, f64::NAN);
            };
            let (s, c) = (kernel_sin(r), kernel_cos(r));
            match quadrant {
                0 => (s, c),
                1 => (c, -s),
                2 => (-s, -c),
                _ => (-c, s),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::soft;

    // Every `soft` function is checked against the `std` method it stands in
    // for. That is the only reference that matters: the bare-metal build has to
    // reach the answers the desktop build already reaches.
    //
    // The sweeps below are dense rather than illustrative — a few hundred
    // thousand points each — because a range-reduced polynomial is exactly the
    // kind of code that is correct on the samples you thought of and wrong one
    // quadrant over.

    /// Distance in units-in-the-last-place, for comparing two finite doubles of
    /// the same sign. Both are mapped through their bit patterns, which for
    /// positive doubles are monotonic in the value.
    fn ulps_apart(a: f64, b: f64) -> u64 {
        assert!(a >= 0.0 && b >= 0.0, "ulp comparison assumes non-negative");
        a.to_bits().abs_diff(b.to_bits())
    }

    #[test]
    fn sqrt_matches_std_to_one_ulp() {
        let mut worst_ulps = 0u64;
        let mut worst_rel = 0.0f64;
        let mut record = |x: f64| {
            let (got, want) = (soft::sqrt(x), x.sqrt());
            worst_ulps = worst_ulps.max(ulps_apart(got, want));
            if want != 0.0 {
                worst_rel = worst_rel.max(((got - want) / want).abs());
            }
        };
        // Geometric, to touch every one of the 2046 normal exponents.
        let mut x = f64::MIN_POSITIVE;
        while x < f64::MAX / 4.0 {
            record(x);
            x *= 1.05;
        }
        // Dense within one octave, to sweep the mantissa itself.
        for i in 0..200_000u64 {
            record(1.0 + i as f64 / 200_000.0);
        }
        // Dense where the pipeline actually works: squared pixel distances,
        // roughly 1e0 .. 1e8.
        for i in 0..200_000u64 {
            record(i as f64 * 503.0);
        }
        assert!(worst_ulps <= 1, "sqrt off by {worst_ulps} ulps");
        assert!(worst_rel < 2.3e-16, "sqrt relative error {worst_rel}");
    }

    #[test]
    fn sqrt_handles_subnormals_and_the_extremes() {
        // Values the iteration must reproduce *exactly*: the two signed zeros
        // and infinity are returned before it runs at all, and a perfect square
        // is a fixed point it cannot drift off.
        for x in [0.0, -0.0, f64::INFINITY, 1.0, 4.0, 0.25, 1e300] {
            let (got, want) = (soft::sqrt(x), x.sqrt());
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "sqrt({x}): {got} != {want} (bit-for-bit)"
            );
        }
        // The ends of the exponent range, including the subnormals that get
        // scaled into the normal range first. Same one-ulp contract as the
        // sweep: Newton in plain `f64` is not correctly rounded, and buying
        // that last bit would cost an exact residual (Dekker two-product) for
        // no gain the geometry can observe.
        for x in [
            f64::MIN_POSITIVE,
            f64::MIN_POSITIVE / 2.0,
            5e-324, // the smallest subnormal there is
            1e-310,
            f64::MAX,
        ] {
            let (got, want) = (soft::sqrt(x), x.sqrt());
            assert!(
                ulps_apart(got, want) <= 1,
                "sqrt({x:e}): {got:e} != {want:e}"
            );
        }
        assert!(soft::sqrt(-1.0).is_nan());
        assert!(soft::sqrt(f64::NAN).is_nan());
        assert!(soft::sqrt(f64::NEG_INFINITY).is_nan());
        assert!(soft::sqrt(-f64::MIN_POSITIVE).is_nan());
    }

    #[test]
    fn rounding_is_exact_against_std() {
        fn check(x: f64) {
            assert_eq!(soft::ceil(x).to_bits(), x.ceil().to_bits(), "ceil({x})");
            assert_eq!(soft::round(x).to_bits(), x.round().to_bits(), "round({x})");
        }
        // Dense around the origin, where every sign and half-way case lives.
        for i in -200_000i64..200_000 {
            check(i as f64 / 1000.0);
        }
        // Then out through the exponent range, including the point at which
        // every double is already an integer.
        let mut x = 1.0f64;
        while x.is_finite() {
            check(x);
            check(-x);
            check(x + 0.5);
            check(x - 0.5);
            x *= 1.5;
        }
    }

    #[test]
    fn rounding_handles_the_edge_cases() {
        // The classic (x + 0.5).floor() trap, plus the signed zeros and the
        // non-finite values.
        for x in [
            0.499_999_999_999_999_94,
            -0.499_999_999_999_999_94,
            0.5,
            -0.5,
            1.5,
            -1.5,
            2.5,
            -2.5,
            0.0,
            -0.0,
            -0.25,
            4_503_599_627_370_495.5, // 2^52 - 0.5, the last half-integer
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert_eq!(soft::ceil(x).to_bits(), x.ceil().to_bits(), "ceil({x})");
            assert_eq!(soft::round(x).to_bits(), x.round().to_bits(), "round({x})");
        }
        let all: [fn(f64) -> f64; 2] = [soft::ceil, soft::round];
        for f in all {
            assert!(f(f64::NAN).is_nan());
        }
    }

    #[test]
    #[cfg(feature = "micro")]
    fn trig_matches_std_across_the_documented_domain() {
        let mut worst = 0.0f64;
        let mut worst_at = 0.0f64;
        let mut record = |x: f64| {
            let (s, c) = soft::sin_cos(x);
            let (ws, wc) = x.sin_cos();
            let err = (s - ws).abs().max((c - wc).abs());
            if err > worst {
                worst = err;
                worst_at = x;
            }
        };
        // Dense through many full turns, so every quadrant of the reduction is
        // exercised thousands of times over with both signs.
        for i in -400_000i64..400_000 {
            record(i as f64 * 5e-4); // +/-200 radians, ~32 turns
        }
        // And out to the edge of the reduction's stated domain.
        let mut x = 1.0f64;
        while x <= (1u64 << 20) as f64 {
            for probe in [x, -x, x + 0.25, x - 0.25] {
                record(probe);
            }
            x *= 1.001;
        }
        assert!(worst < 1e-7, "trig error {worst} at {worst_at}");
        // The real number, well inside the contract, recorded so a regression
        // that merely stays under the loose bound still shows up.
        assert!(worst < 3e-16, "trig error {worst} at {worst_at}");
    }

    #[test]
    #[cfg(feature = "micro")]
    fn trig_rejects_what_it_cannot_reduce() {
        for x in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            1e30,
            -1e30,
            (1u64 << 21) as f64,
        ] {
            let (s, c) = soft::sin_cos(x);
            assert!(s.is_nan() && c.is_nan(), "sin_cos({x}) should be NaN");
        }
    }

    #[test]
    #[cfg(feature = "micro")]
    fn trig_lands_exactly_on_the_axes() {
        use core::f64::consts::{FRAC_PI_2, PI, TAU};
        // The reduction's whole job: at a multiple of pi/2 the residual is the
        // tiny amount by which the double differs from the true multiple, so
        // the "zero" component must come out at that scale rather than at the
        // scale of a lost bit.
        for (x, want_s, want_c) in [
            (0.0, 0.0, 1.0),
            (FRAC_PI_2, 1.0, 0.0),
            (PI, 0.0, -1.0),
            (PI + FRAC_PI_2, -1.0, 0.0),
            (TAU, 0.0, 1.0),
            (-FRAC_PI_2, -1.0, 0.0),
        ] {
            let (s, c) = soft::sin_cos(x);
            assert!((s - want_s).abs() < 1e-15, "sin({x}) = {s}");
            assert!((c - want_c).abs() < 1e-15, "cos({x}) = {c}");
        }
    }

    /// The dispatch layer itself: whichever path is compiled in, it must agree
    /// with `soft` to the tolerance the pipeline is verified at. Under
    /// `--cfg qr_detect_soft_float` this is an identity; under the default it
    /// is the last check that the two paths have not drifted apart.
    #[test]
    fn the_dispatch_agrees_with_the_hand_rolled_path() {
        for i in 0..20_000i64 {
            let x = i as f64 * 0.0007;
            assert!((super::sqrt(x) - soft::sqrt(x)).abs() <= f64::EPSILON * x.max(1.0));
            assert_eq!(super::ceil(x).to_bits(), soft::ceil(x).to_bits());
            assert_eq!(super::round(x).to_bits(), soft::round(x).to_bits());
            #[cfg(feature = "micro")]
            {
                let ((s, c), (ss, sc)) = (super::sin_cos(x), soft::sin_cos(x));
                assert!((s - ss).abs() < 1e-15 && (c - sc).abs() < 1e-15);
            }
        }
    }
}

/// Exists only under `--cfg qr_detect_soft_float`. The CI equivalence lane
/// runs this test by exact name and fails if it is absent, so a misspelled
/// `--cfg` (which compiles silently — check-cfg only lints source spellings)
/// cannot turn that lane into a green no-op.
#[cfg(qr_detect_soft_float)]
#[cfg(test)]
mod lane_liveness {
    #[test]
    fn soft_float_lane_is_live() {}
}
