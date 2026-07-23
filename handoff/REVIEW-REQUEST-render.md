# Review Request — Step 2c: qr-render

Builder: Bob-R
Ready for Review: YES

## Scope

Rendering layer only. Consumes `qr_core::BitMatrix`; no dependency on the
concurrently-built qr-encode/qr-decode/qr-detect. Tested against hand-made
`BitMatrix` fixtures.

## Files changed

- `crates/qr-render/Cargo.toml` (lines 7–9): added `qr-core = { workspace = true }`
  and `image = { workspace = true }` under `[dependencies]`.
- `crates/qr-render/src/lib.rs` (new, full file): the crate — `Options`, three
  renderers, tests.

## Public API

- `Color = [u8; 4]` type alias (RGBA, `a=255` opaque). Consts `BLACK`, `WHITE`.
- `struct Options { scale: u32, quiet_zone: u32, foreground: Color, background: Color }`
  - `Default`: scale 8, quiet_zone 4, fg black, bg white.
  - Fluent builders: `.scale()`, `.quiet_zone()`, `.colors()`.
- `render_image(&BitMatrix, &Options) -> image::RgbaImage`
- `render_svg(&BitMatrix, &Options) -> String` (no deps beyond std formatting)
- `render_ascii(&BitMatrix, &Options) -> String`

Convention (documented on the crate): `true` module = dark = `foreground`;
`false` = light = `background`; quiet zone always background. Coordinates
`(x, y)` = (column, row), top-left origin — matches the rest of the stack.

## Decisions

- Output side = `(matrix.size() + 2*quiet_zone) * scale` for image and SVG,
  identical formula for both.
- `scale` clamped to a minimum of 1 (a 0 scale would yield a 0-px image). Tested.
- SVG uses one background `<rect>` plus a single `<path>` (one subpath per dark
  module, `M x y h s v s h-s z`), `shape-rendering="crispEdges"`, matching
  `viewBox`. Empty matrix emits no `<path>`. Colours: `#rrggbb` when opaque,
  else `rgba(r,g,b,frac)`.
- ASCII: two chars per module (`██` dark, spaces light) for square aspect;
  honours quiet zone; trailing newline.
- No new workspace deps needed — both `qr-core` and `image` were already in
  `workspace.dependencies`. Root Cargo.toml untouched.

## Uncertainties / notes for Richard

- SVG alpha is formatted to 3 decimals with trailing zeros trimmed
  (128/255 → `0.502`). If a fixed precision is preferred, easy change.
- ASCII uses filled block `█` for dark on default (light) terminal background.
  Some debuggers prefer the inverse; trivial to flip if desired.
- No PNG-encoding helper exposed (caller uses `image`'s own `save`/`write_to`);
  brief only asked for the `RgbaImage`. Flag if a convenience wrapper is wanted.

## Test results

`cargo test -p qr-render`: 13 passed, 0 failed.
`cargo clippy -p qr-render --all-targets -- -D warnings`: clean.

Coverage: default options; image dimensions (incl. zero-scale clamp);
quiet-zone-is-background; module fg/light spot checks; custom colours; SVG
well-formedness + dimensions + module geometry + empty-matrix + translucent
colour; ASCII dimensions/spots + all-light; css_color formatting.
