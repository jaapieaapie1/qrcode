//! `qr-render` — turn a [`BitMatrix`] into something you can look at.
//!
//! Three renderers share a single [`Options`] type:
//!
//! - [`render_image`] — a rasterised [`image::RgbaImage`].
//! - [`render_svg`] — a standalone, dependency-free SVG document ([`String`]).
//! - [`render_ascii`] — a monospace text grid for terminal debugging.
//!
//! # Modules and colours
//!
//! A `true` module in the [`BitMatrix`] is a *dark* module and is painted with
//! [`Options::foreground`]; a `false` module is *light* and is painted with
//! [`Options::background`]. The quiet zone is always light (background).
//!
//! # Coordinates
//!
//! Following the rest of the stack, matrix coordinates are `(x, y)` =
//! (column, row) with the origin at the top-left. In every rendered output the
//! module at `(0, 0)` sits at the top-left, just inside the quiet zone.

use image::{Rgba, RgbaImage};
use qr_core::BitMatrix;

/// An RGBA colour, one byte per channel: `[r, g, b, a]`, `a = 255` is opaque.
pub type Color = [u8; 4];

/// Opaque black — the default foreground.
pub const BLACK: Color = [0, 0, 0, 255];
/// Opaque white — the default background.
pub const WHITE: Color = [255, 255, 255, 255];

/// Rendering options shared by every renderer.
///
/// Construct with [`Options::default`] and adjust fields, or use the builder
/// helpers ([`Options::scale`], [`Options::quiet_zone`], [`Options::colors`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Side length of a single module, in pixels (raster) or user units (SVG).
    ///
    /// Clamped to a minimum of `1` at render time; a `0` scale would produce an
    /// empty image.
    pub scale: u32,
    /// Width of the light border around the code, measured in modules.
    ///
    /// The QR specification recommends `4`, which is the default.
    pub quiet_zone: u32,
    /// Colour of dark (`true`) modules.
    pub foreground: Color,
    /// Colour of light (`false`) modules and the quiet zone.
    pub background: Color,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            scale: 8,
            quiet_zone: 4,
            foreground: BLACK,
            background: WHITE,
        }
    }
}

impl Options {
    /// Sets [`Options::scale`] (fluent).
    #[must_use]
    pub fn scale(mut self, scale: u32) -> Self {
        self.scale = scale;
        self
    }

    /// Sets [`Options::quiet_zone`] (fluent).
    #[must_use]
    pub fn quiet_zone(mut self, quiet_zone: u32) -> Self {
        self.quiet_zone = quiet_zone;
        self
    }

    /// Sets both colours (fluent).
    #[must_use]
    pub fn colors(mut self, foreground: Color, background: Color) -> Self {
        self.foreground = foreground;
        self.background = background;
        self
    }

    /// Effective scale, never `0`. Clamped so the rendered side in pixels
    /// (`(size + 2 * quiet_zone) * scale`) cannot overflow `u32`.
    #[inline]
    fn eff_scale_for(self, matrix: &BitMatrix) -> u32 {
        let scale = self.scale.max(1);
        let max_scale = u32::MAX / self.modules_side(matrix);
        scale.min(max_scale.max(1))
    }

    /// Side length of the rendered output, in modules (matrix + both quiet
    /// zones). Quiet zone saturates rather than overflowing.
    #[inline]
    fn modules_side(self, matrix: &BitMatrix) -> u32 {
        matrix
            .size()
            .saturating_add(self.quiet_zone.saturating_mul(2))
    }
}

/// Rasterises `matrix` into an [`image::RgbaImage`].
///
/// The image is `(matrix.size() + 2 * quiet_zone) * scale` pixels square. Each
/// module becomes a solid `scale`×`scale` block; the quiet zone is filled with
/// [`Options::background`].
#[must_use]
pub fn render_image(matrix: &BitMatrix, opts: &Options) -> RgbaImage {
    let scale = opts.eff_scale_for(matrix);
    let quiet = opts.quiet_zone;
    let side_px = opts.modules_side(matrix) * scale;

    let fg = Rgba(opts.foreground);
    let bg = Rgba(opts.background);

    let mut img = RgbaImage::from_pixel(side_px, side_px, bg);

    let size = matrix.size();
    for my in 0..size {
        for mx in 0..size {
            if !matrix.get(mx, my) {
                continue;
            }
            let px0 = (mx + quiet) * scale;
            let py0 = (my + quiet) * scale;
            for dy in 0..scale {
                for dx in 0..scale {
                    img.put_pixel(px0 + dx, py0 + dy, fg);
                }
            }
        }
    }

    img
}

/// Renders `matrix` as a standalone SVG document.
///
/// The document is self-contained and dependency-free: a background `<rect>`
/// covering the whole canvas plus a single `<path>` whose subpaths are the dark
/// modules. Dimensions match [`render_image`]: side `= (size + 2 * quiet_zone) *
/// scale` user units, with a matching `viewBox`.
#[must_use]
pub fn render_svg(matrix: &BitMatrix, opts: &Options) -> String {
    let scale = opts.eff_scale_for(matrix);
    let quiet = opts.quiet_zone;
    let side = opts.modules_side(matrix) * scale;
    let size = matrix.size();

    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{side}\" height=\"{side}\" \
         viewBox=\"0 0 {side} {side}\" shape-rendering=\"crispEdges\">"
    ));
    s.push_str(&format!(
        "<rect width=\"{side}\" height=\"{side}\" fill=\"{}\"/>",
        css_color(opts.background)
    ));

    let mut d = String::new();
    for my in 0..size {
        for mx in 0..size {
            if !matrix.get(mx, my) {
                continue;
            }
            let px = (mx + quiet) * scale;
            let py = (my + quiet) * scale;
            d.push_str(&format!("M{px} {py}h{scale}v{scale}h-{scale}z"));
        }
    }

    if !d.is_empty() {
        s.push_str(&format!(
            "<path d=\"{d}\" fill=\"{}\"/>",
            css_color(opts.foreground)
        ));
    }

    s.push_str("</svg>");
    s
}

/// Renders `matrix` as a monospace text grid for terminal debugging.
///
/// Dark modules are drawn as two full blocks (`██`) and light modules as two
/// spaces, so the output keeps roughly square proportions in a terminal. The
/// quiet zone is honoured. The returned string ends with a trailing newline.
#[must_use]
pub fn render_ascii(matrix: &BitMatrix, opts: &Options) -> String {
    const DARK: &str = "\u{2588}\u{2588}";
    const LIGHT: &str = "  ";

    let quiet = opts.quiet_zone;
    let size = matrix.size();
    let side = size + 2 * quiet;

    let mut out = String::new();
    for row in 0..side {
        for col in 0..side {
            let dark = row >= quiet
                && row < quiet + size
                && col >= quiet
                && col < quiet + size
                && matrix.get(col - quiet, row - quiet);
            out.push_str(if dark { DARK } else { LIGHT });
        }
        out.push('\n');
    }
    out
}

/// Formats an RGBA colour as a CSS value for SVG: `#rrggbb` when opaque,
/// otherwise `rgba(r,g,b,a)` with alpha as a 0..1 fraction.
fn css_color(c: Color) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        let alpha = f32::from(c[3]) / 255.0;
        // Trim to 3 decimals; strip trailing zeros for a tidy string.
        let mut a = format!("{alpha:.3}");
        while a.ends_with('0') {
            a.pop();
        }
        if a.ends_with('.') {
            a.push('0');
        }
        format!("rgba({},{},{},{})", c[0], c[1], c[2], a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3×3 fixture: dark modules on the main diagonal, light elsewhere.
    ///
    /// ```text
    /// # . .
    /// . # .
    /// . . #
    /// ```
    fn diag3() -> BitMatrix {
        let mut m = BitMatrix::new(3);
        m.set(0, 0, true);
        m.set(1, 1, true);
        m.set(2, 2, true);
        m
    }

    #[test]
    fn default_options() {
        let o = Options::default();
        assert_eq!(o.scale, 8);
        assert_eq!(o.quiet_zone, 4);
        assert_eq!(o.foreground, BLACK);
        assert_eq!(o.background, WHITE);
    }

    #[test]
    fn image_dimensions() {
        let m = diag3();
        let opts = Options::default().scale(4).quiet_zone(2);
        let img = render_image(&m, &opts);
        // (3 + 2*2) * 4 = 28
        assert_eq!(img.width(), 28);
        assert_eq!(img.height(), 28);
    }

    #[test]
    fn image_zero_scale_clamped() {
        let m = diag3();
        let opts = Options::default().scale(0).quiet_zone(0);
        let img = render_image(&m, &opts);
        assert_eq!(img.width(), 3);
        assert_eq!(img.height(), 3);
    }

    #[test]
    fn image_quiet_zone_is_background() {
        let m = diag3();
        let opts = Options::default().scale(2).quiet_zone(4);
        let img = render_image(&m, &opts);
        let bg = Rgba(opts.background);
        // Every pixel in the top quiet-zone band is background.
        for y in 0..(4 * 2) {
            for x in 0..img.width() {
                assert_eq!(*img.get_pixel(x, y), bg, "quiet zone px ({x},{y})");
            }
        }
        // Corners are background too.
        assert_eq!(*img.get_pixel(0, 0), bg);
        assert_eq!(*img.get_pixel(img.width() - 1, img.height() - 1), bg);
    }

    #[test]
    fn image_module_spot_checks() {
        let m = diag3();
        let scale = 3;
        let quiet = 1;
        let opts = Options::default().scale(scale).quiet_zone(quiet);
        let img = render_image(&m, &opts);
        let fg = Rgba(opts.foreground);
        let bg = Rgba(opts.background);

        // Module (0,0) is dark: its block starts at (quiet*scale, quiet*scale).
        let (bx, by) = (quiet * scale, quiet * scale);
        for dy in 0..scale {
            for dx in 0..scale {
                assert_eq!(*img.get_pixel(bx + dx, by + dy), fg, "dark (0,0)");
            }
        }
        // Module (1,0) is light.
        let lx = (1 + quiet) * scale;
        assert_eq!(*img.get_pixel(lx, by), bg, "light (1,0)");
        // Module (2,2) is dark.
        let cx = (2 + quiet) * scale;
        let cy = (2 + quiet) * scale;
        assert_eq!(*img.get_pixel(cx, cy), fg, "dark (2,2)");
    }

    #[test]
    fn image_custom_colors() {
        let m = diag3();
        let fg = [10, 20, 30, 255];
        let bg = [200, 210, 220, 255];
        let opts = Options::default().scale(1).quiet_zone(0).colors(fg, bg);
        let img = render_image(&m, &opts);
        assert_eq!(*img.get_pixel(0, 0), Rgba(fg)); // dark
        assert_eq!(*img.get_pixel(1, 0), Rgba(bg)); // light
    }

    #[test]
    fn svg_well_formed_and_dimensions() {
        let m = diag3();
        let opts = Options::default().scale(10).quiet_zone(4);
        let svg = render_svg(&m, &opts);
        // (3 + 2*4) * 10 = 110
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("width=\"110\""));
        assert!(svg.contains("height=\"110\""));
        assert!(svg.contains("viewBox=\"0 0 110 110\""));
        assert!(svg.contains("xmlns=\"http://www.w3.org/2000/svg\""));
        // Background rect + module path present.
        assert!(svg.contains("<rect"));
        assert!(svg.contains("<path"));
        // Balanced tags.
        assert_eq!(svg.matches("<svg").count(), 1);
        assert_eq!(svg.matches("</svg>").count(), 1);
    }

    #[test]
    fn svg_module_geometry() {
        let m = diag3();
        let opts = Options::default().scale(10).quiet_zone(4);
        let svg = render_svg(&m, &opts);
        // Dark module (0,0) → path move to ((0+4)*10, (0+4)*10) = (40,40).
        assert!(svg.contains("M40 40h10v10h-10z"), "svg: {svg}");
        // Dark module (2,2) → (60,60).
        assert!(svg.contains("M60 60h10v10h-10z"));
        // Default colours.
        assert!(svg.contains("fill=\"#000000\""));
        assert!(svg.contains("fill=\"#ffffff\""));
    }

    #[test]
    fn svg_empty_matrix_has_no_path() {
        let m = BitMatrix::new(3); // all light
        let svg = render_svg(&m, &Options::default());
        assert!(!svg.contains("<path"));
        assert!(svg.contains("<rect"));
        assert!(svg.ends_with("</svg>"));
    }

    #[test]
    fn svg_translucent_color() {
        let m = diag3();
        let opts = Options::default().colors([1, 2, 3, 128], WHITE);
        let svg = render_svg(&m, &opts);
        assert!(svg.contains("rgba(1,2,3,0.502)"), "svg: {svg}");
    }

    #[test]
    fn ascii_dimensions_and_spots() {
        let m = diag3();
        let opts = Options::default().quiet_zone(1);
        let ascii = render_ascii(&m, &opts);
        let lines: Vec<&str> = ascii.lines().collect();
        // side = 3 + 2*1 = 5 rows.
        assert_eq!(lines.len(), 5);
        // Each cell is 2 chars wide → 10 chars per line.
        for l in &lines {
            assert_eq!(l.chars().count(), 10);
        }
        // Row 0 is quiet zone → all spaces.
        assert!(lines[0].chars().all(|c| c == ' '));
        // Module (0,0) is at row 1, cols 2..4 → dark blocks.
        assert!(lines[1].starts_with("  \u{2588}\u{2588}"));
    }

    #[test]
    fn ascii_all_light() {
        let m = BitMatrix::new(2);
        let ascii = render_ascii(&m, &Options::default().quiet_zone(0));
        assert!(!ascii.contains('\u{2588}'));
        assert_eq!(ascii.lines().count(), 2);
    }

    #[test]
    fn css_color_opaque_and_alpha() {
        assert_eq!(css_color([255, 0, 128, 255]), "#ff0080");
        assert_eq!(css_color([0, 0, 0, 0]), "rgba(0,0,0,0.0)");
        assert_eq!(css_color([1, 2, 3, 255]), "#010203");
    }
}
