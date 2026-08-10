//! Grayscale-to-binary conversion.
//!
//! Primary path is Bradley-style adaptive thresholding: each pixel is compared
//! against the mean of a local window (computed in O(1) via an integral image).
//! For very small images — or when the adaptive result degenerates to a single
//! colour — we fall back to a global Otsu threshold.

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

/// Binarizes a luma buffer (`width * height` bytes, row-major).
///
/// Returns `None` if the dimensions do not match the buffer length or the image
/// is empty.
pub fn binarize(luma: &[u8], width: u32, height: u32) -> Option<Binary> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || luma.len() != w * h {
        return None;
    }

    // Small images: adaptive windows are meaningless, use a global threshold.
    if width < 24 || height < 24 {
        return Some(otsu(luma, width, height));
    }

    let bin = adaptive(luma, width, height);

    // Guard against a degenerate all-one-colour result (e.g. flat gradient) by
    // falling back to Otsu.
    let dark_count = bin.dark.iter().filter(|&&d| d).count();
    let total = w * h;
    if dark_count == 0 || dark_count == total {
        return Some(otsu(luma, width, height));
    }
    Some(bin)
}

/// Bradley adaptive threshold using an integral image.
fn adaptive(luma: &[u8], width: u32, height: u32) -> Binary {
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

    // Window radius ~ 1/8 of the smaller side, at least 4 px.
    let radius = (w.min(h) / 8).max(4) as i64;
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
        let b = binarize(&luma, w, h).unwrap();
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
        let b = binarize(&luma, w, h).unwrap();
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
        let b = binarize(&luma, side, side).unwrap();
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
    fn vignetted_module_errors(depth: f64) -> u32 {
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

        let b = binarize(&luma, w, h).unwrap();
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
        // where the current `/8` radius runs out first.
        //
        // Measured on this exact fixture, module errors out of 625:
        //
        //   depth  0.50  0.70  0.85  0.90  0.95
        //   /8        0     0     0    13    32
        //   /16       0     0     0     0     0
        //
        // So the envelope pinned here is a vignette depth of 0.85: the corner of
        // the frame at 15% of the centre's illumination, with 45% print
        // contrast. Past that the narrower window would still have held, and we
        // no longer do. `/8` is not negotiable — a micro symbol's 3-module
        // finder core hollows out beneath it (see the test above) — so this is a
        // real cost, accepted deliberately. The fix that keeps both envelopes is
        // two candidate binarizations driven through the finder scan, which is
        // its own step, not a threshold tweak.
        assert_eq!(
            vignetted_module_errors(0.85),
            0,
            "a 0.85-depth vignette must still binarize cleanly"
        );
    }
}
