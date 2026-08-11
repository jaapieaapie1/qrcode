//! Grayscale-to-binary conversion.
//!
//! Primary path is Bradley-style adaptive thresholding: each pixel is compared
//! against the mean of a local window (computed in O(1) via an integral image).
//! For very small images — or when the adaptive result degenerates to a single
//! colour — we fall back to a global Otsu threshold.
//!
//! The window radius is a parameter, not a constant, and there are two settings
//! of it: [`PRIMARY_WINDOW_DIV`] and [`RETRY_WINDOW_DIV`]. Neither holds every
//! envelope. A wide window is required to keep solid features solid at any print
//! contrast; a narrow window is required to track a steep illumination gradient.
//! The two constraints cross rather than merely leaving a narrow band between
//! them, so they are resolved a layer up — the pipeline binarizes at the primary
//! radius and, only when that yields nothing at all, tries the whole frame again
//! at the retry radius. See the crate docs for what that costs.

/// A binarized image. `true` = dark module, `false` = light. Coordinates are
/// `(x, y)` = (column, row), origin top-left.
pub struct Binary {
    width: u32,
    height: u32,
    dark: Vec<bool>,
}

impl Binary {
    /// Width in pixels.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Returns whether the pixel at `(x, y)` is dark. Out-of-bounds reads are
    /// treated as light (`false`).
    #[inline]
    pub fn get(&self, x: i64, y: i64) -> bool {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return false;
        }
        self.dark[(y as usize) * (self.width as usize) + (x as usize)]
    }
}

/// Window divisor for the primary pass: radius ~1/8 of the smaller side.
///
/// Wide enough that a window never fits inside the largest solid feature a
/// symbol can present — a finder pattern's 3-module core — which is the
/// condition for that core's interior to hollow out. See
/// [`tests::a_solid_block_does_not_hollow_out`].
pub const PRIMARY_WINDOW_DIV: usize = 8;

/// Window divisor for the retry pass: radius ~1/16 of the smaller side.
///
/// Tracks uneven illumination roughly twice as closely as the primary, which is
/// what a small low-contrast symbol in the corner of a vignetted frame needs.
/// It cannot be the primary: at this radius a large finder core hollows out.
/// See [`tests::the_retry_window_holds_a_vignette_the_primary_loses`].
pub const RETRY_WINDOW_DIV: usize = 16;

/// Binarizes a luma buffer (`width * height` bytes, row-major).
///
/// `window_div` selects the adaptive window radius as a fraction of the smaller
/// image side — [`PRIMARY_WINDOW_DIV`] or [`RETRY_WINDOW_DIV`]. It is a
/// parameter rather than a constant because no single radius holds both the
/// print-contrast and the illumination envelope; the pipeline runs the primary
/// and, only if it finds nothing at all, tries again at the retry radius.
///
/// Returns `None` if the dimensions do not match the buffer length or the image
/// is empty.
pub fn binarize(luma: &[u8], width: u32, height: u32, window_div: usize) -> Option<Binary> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || luma.len() != w * h {
        return None;
    }

    // Small images: adaptive windows are meaningless, use a global threshold.
    // Note this makes the retry pass a no-op for them — both passes land on the
    // same Otsu result — which is correct: there is no second opinion to have.
    if width < 24 || height < 24 {
        return Some(otsu(luma, width, height));
    }

    let bin = adaptive(luma, width, height, window_div);

    // Guard against a degenerate all-one-colour result (e.g. flat gradient) by
    // falling back to Otsu.
    let dark_count = bin.dark.iter().filter(|&&d| d).count();
    let total = w * h;
    if dark_count == 0 || dark_count == total {
        return Some(otsu(luma, width, height));
    }
    Some(bin)
}

/// Bradley adaptive threshold using an integral image, with the window radius
/// set to `1/window_div` of the smaller image side.
fn adaptive(luma: &[u8], width: u32, height: u32, window_div: usize) -> Binary {
    let (w, h) = (width as usize, height as usize);

    // Integral image with a zero-padded first row/column: `integral[y*(w+1)+x]`
    // is the sum of all pixels strictly above and to the left of `(x, y)`.
    let stride = w + 1;
    let mut integral = vec![0u64; stride * (h + 1)];
    for y in 0..h {
        let mut row_sum = 0u64;
        for x in 0..w {
            row_sum += luma[y * w + x] as u64;
            integral[(y + 1) * stride + (x + 1)] = integral[y * stride + (x + 1)] + row_sum;
        }
    }

    // Window radius ~ 1/window_div of the smaller side, at least 4 px.
    let radius = (w.min(h) / window_div.max(1)).max(4) as i64;
    // Bradley "t" parameter: a pixel is dark when it is >= 12% below the local
    // mean.
    let t_num: u64 = 88; // (100 - 12)
    let t_den: u64 = 100;

    let mut dark = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let x0 = (x as i64 - radius).max(0) as usize;
            let y0 = (y as i64 - radius).max(0) as usize;
            let x1 = ((x as i64 + radius) as usize).min(w - 1);
            let y1 = ((y as i64 + radius) as usize).min(h - 1);
            let count = ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64;
            let sum = integral[(y1 + 1) * stride + (x1 + 1)] + integral[y0 * stride + x0]
                - integral[y0 * stride + (x1 + 1)]
                - integral[(y1 + 1) * stride + x0];
            let value = luma[y * w + x] as u64;
            // dark if value*count < mean*count * t_num/t_den == sum * t_num/t_den
            dark[y * w + x] = value * count * t_den < sum * t_num;
        }
    }

    Binary {
        width,
        height,
        dark,
    }
}

/// Global Otsu threshold. Pixels with value `<=` the threshold are dark.
fn otsu(luma: &[u8], width: u32, height: u32) -> Binary {
    let mut hist = [0u64; 256];
    for &p in luma {
        hist[p as usize] += 1;
    }
    let total = luma.len() as u64;

    let sum_total: u64 = hist.iter().enumerate().map(|(i, &c)| i as u64 * c).sum();
    let mut sum_back = 0u64;
    let mut weight_back = 0u64;
    let mut max_between = -1.0f64;
    let mut threshold = 127u8;

    for (i, &count) in hist.iter().enumerate() {
        weight_back += count;
        if weight_back == 0 {
            continue;
        }
        let weight_fore = total - weight_back;
        if weight_fore == 0 {
            break;
        }
        sum_back += i as u64 * count;
        let mean_back = sum_back as f64 / weight_back as f64;
        let mean_fore = (sum_total - sum_back) as f64 / weight_fore as f64;
        let diff = mean_back - mean_fore;
        let between = weight_back as f64 * weight_fore as f64 * diff * diff;
        if between > max_between {
            max_between = between;
            threshold = i as u8;
        }
    }

    let dark = luma.iter().map(|&p| p <= threshold).collect();
    Binary {
        width,
        height,
        dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn otsu_splits_bimodal() {
        // 16x2 image: left half black (10), right half white (240).
        let w = 16u32;
        let h = 2u32;
        let mut luma = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                luma[(y * w + x) as usize] = if x < 8 { 10 } else { 240 };
            }
        }
        let b = binarize(&luma, w, h, PRIMARY_WINDOW_DIV).unwrap();
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                assert_eq!(b.get(x, y), x < 8, "pixel ({x},{y})");
            }
        }
    }

    #[test]
    fn adaptive_handles_gradient_with_marks() {
        // 64x64 gradient background with thin dark lines (module-scale marks that
        // stay well below their local window mean, as real QR modules do).
        let w = 64u32;
        let h = 64u32;
        let mut luma = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let bg = 80 + x; // 80..144 gradient
                luma[(y * w + x) as usize] = bg as u8;
            }
        }
        // Vertical dark lines 2 px wide every 6 px.
        for y in 0..h {
            for x in 0..w {
                if x % 6 < 2 {
                    luma[(y * w + x) as usize] = 0;
                }
            }
        }
        let b = binarize(&luma, w, h, PRIMARY_WINDOW_DIV).unwrap();
        assert!(b.get(24, 30), "dark line should be dark"); // 24 % 6 == 0
        assert!(!b.get(27, 30), "gap should be light"); // 27 % 6 == 3
    }

    #[test]
    fn a_solid_block_does_not_hollow_out() {
        // The 3x3 core of a finder pattern is the largest solid feature in any
        // symbol, and in a tightly framed shot it covers a good fraction of the
        // image: an 11-module micro symbol with its quiet zone is 15 modules
        // across, so the core is a fifth of the frame.
        //
        // Where the adaptive window fits *inside* such a block the local mean is
        // the block's own colour, the pixel is not below it, and the centre of
        // the block flips to light. The finder scan's vertical cross-check walks
        // the contiguous dark band through the centre, so a hole there costs the
        // whole symbol. The window must stay wider than the block.
        let side = 168u32; // 21 modules of 8 px, as a micro symbol renders
        let block = 24u32; // three modules
        let mut luma = vec![255u8; (side * side) as usize];
        let lo = (side - block) / 2;
        for y in lo..lo + block {
            for x in lo..lo + block {
                luma[(y * side + x) as usize] = 0;
            }
        }
        let b = binarize(&luma, side, side, PRIMARY_WINDOW_DIV).unwrap();
        for y in lo..lo + block {
            for x in lo..lo + block {
                assert!(b.get(x as i64, y as i64), "({x}, {y}) hollowed out");
            }
        }
    }

    /// Renders a worn-print symbol in the corner of a camera frame under a
    /// radial vignette, and returns how many of its module centres binarize
    /// wrongly.
    ///
    /// `depth` is the vignette depth: illumination falls as
    /// `1 - depth * (r / r_max)^2` from the frame centre. Print contrast is 45%
    /// (dark modules reflect 0.55 of the light ones), which is worn thermal
    /// print rather than fresh laser.
    fn vignetted_module_errors(depth: f64, window_div: usize) -> u32 {
        let (w, h) = (1280u32, 720u32);
        let (modules, scale) = (25u32, 3u32);
        let (ox, oy) = (40u32, 40u32); // near the frame corner, where it hurts
        let cx = (w - 1) as f64 / 2.0;
        let cy = (h - 1) as f64 / 2.0;
        let r_max = (cx * cx + cy * cy).sqrt();

        // Deterministic module pattern, as dense as real data.
        let dark_module = |mx: u32, my: u32| {
            let mut v = (mx.wrapping_mul(73_856_093) ^ my.wrapping_mul(19_349_663)).wrapping_add(1);
            v ^= v >> 13;
            v = v.wrapping_mul(0x5bd1_e995);
            v ^= v >> 15;
            v & 1 == 0
        };

        let mut luma = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f64 - cx, y as f64 - cy);
                let r = (dx * dx + dy * dy).sqrt() / r_max;
                let illumination = 1.0 - depth * r * r;
                let mut reflectance = 1.0f64;
                if x >= ox
                    && y >= oy
                    && x < ox + modules * scale
                    && y < oy + modules * scale
                    && dark_module((x - ox) / scale, (y - oy) / scale)
                {
                    reflectance = 0.55;
                }
                luma[(y * w + x) as usize] = (255.0 * illumination * reflectance)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }

        let b = binarize(&luma, w, h, window_div).unwrap();
        let mut bad = 0;
        for my in 0..modules {
            for mx in 0..modules {
                let px = (ox + mx * scale + scale / 2) as i64;
                let py = (oy + my * scale + scale / 2) as i64;
                if b.get(px, py) != dark_module(mx, my) {
                    bad += 1;
                }
            }
        }
        bad
    }

    #[test]
    fn adaptive_survives_a_vignette_to_the_documented_depth() {
        // The other side of the window-radius trade-off, and the direction it
        // costs. A wider window tracks illumination less closely, so a small
        // low-contrast symbol in the corner of a large frame — where the window
        // is clamped against the border and the vignette curves inside it — is
        // where the primary `/8` radius runs out first.
        //
        // Measured on this exact fixture, module errors out of 625:
        //
        //   depth  0.50  0.70  0.85  0.90  0.95
        //   /8        0     0     0    13    32
        //   /16       0     0     0     0     0
        //
        // 0.85 is what the *primary* pass holds, and it is not negotiable that
        // the primary is `/8`: at `/16` a micro symbol's 3-module finder core
        // hollows out (see the two tests above) and, worse, so does any solid
        // feature at ordinary print contrast. Past 0.85 the primary pass is not
        // where the answer comes from — the pipeline's retry is, at the radius
        // pinned in the next test.
        assert_eq!(
            vignetted_module_errors(0.85, PRIMARY_WINDOW_DIV),
            0,
            "a 0.85-depth vignette must still binarize cleanly on the primary pass"
        );
    }

    #[test]
    fn the_retry_window_holds_a_vignette_the_primary_loses() {
        // The reason the retry pass exists, stated as a measurement rather than
        // as an intention — and pinned at the *crossover* rather than at two
        // comfortable points either side of it, so the gap between the radii is
        // what the test actually holds.
        //
        // 0.90 is the first depth in the table above where the two disagree.
        // Both assertions are about that one depth: the primary has given up on
        // it and the retry is still clean on it. The first also guards the
        // fixture — if it ever stops distinguishing the radii, this fails rather
        // than passing vacuously.
        assert!(
            vignetted_module_errors(0.90, PRIMARY_WINDOW_DIV) > 0,
            "fixture no longer distinguishes the two radii; the retry would be untested"
        );
        assert_eq!(
            vignetted_module_errors(0.90, RETRY_WINDOW_DIV),
            0,
            "the retry radius must hold at the depth the primary loses"
        );
        assert_eq!(
            vignetted_module_errors(0.95, RETRY_WINDOW_DIV),
            0,
            "and must keep holding well past it"
        );

        // This pins the binarization half only. That the *pipeline* reaches it,
        // and recovers a symbol it otherwise loses, is pinned end-to-end by
        // `detect::a_deep_vignette_is_recovered_by_the_retry_pass` and — for the
        // degraded-sample case the trigger has to catch — by
        // `micro_detect::a_degraded_micro_sample_does_not_suppress_the_retry`.
    }

    #[test]
    fn a_solid_block_at_worn_print_contrast_does_not_hollow_out() {
        // The axis the fixture set was missing, and the one that killed the
        // two-scale discriminator: contrast and illumination are independent,
        // and every other hollowing fixture here paints luma 0 on luma 255.
        //
        // Real worn thermal print is nothing like that. At 45% contrast a dark
        // module is luma 140 on a 255 background — the exact value
        // `vignetted_module_errors` already models on the illumination axis —
        // and any rule that keys off *how dark* a feature is rather than *how
        // big* it is silently stops firing here while every full-contrast test
        // in the file stays green.
        //
        // Swept across the range a print can plausibly land in, so the pin does
        // not sit on one lucky value.
        for dark in [0u8, 51, 76, 102, 115, 140, 153] {
            let side = 168u32; // 21 modules of 8 px, as a micro symbol renders
            let block = 24u32; // three modules
            let mut luma = vec![255u8; (side * side) as usize];
            let lo = (side - block) / 2;
            for y in lo..lo + block {
                for x in lo..lo + block {
                    luma[(y * side + x) as usize] = dark;
                }
            }
            let b = binarize(&luma, side, side, PRIMARY_WINDOW_DIV).unwrap();
            for y in lo..lo + block {
                for x in lo..lo + block {
                    assert!(
                        b.get(x as i64, y as i64),
                        "({x}, {y}) hollowed out at dark level {dark}"
                    );
                }
            }
        }
    }
}
