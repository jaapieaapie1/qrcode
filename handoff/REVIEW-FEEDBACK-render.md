# Review Feedback — Step 2c: qr-render

**Verdict: Step 2c CLEAR**

Date: 2026-07-23
Reviewer: Richard-R
Ready for Builder: YES

Counts — BLOCKER: 0 | MAJOR: 0 | MINOR: 1

Verified: `cargo test -p qr-render` → 13 passed, 0 failed. `cargo clippy -p qr-render --all-targets -- -D warnings` → clean. Both claims accurate.

---

## Must Fix
None.

## Should Fix

- **MINOR — `src/lib.rs:110`, `:146`, `:97` — unchecked u32 overflow on pixel-dimension math.**
  `side_px = (matrix.size() + 2*quiet_zone) * scale` is plain u32 arithmetic. At v40
  (size 177, quiet 4 → 185) any `scale > ~23_000_000` overflows u32: in debug/test builds
  the multiply **panics**; in release it silently wraps to a wrong (tiny) dimension. `2*quiet_zone`
  and `size + 2*quiet_zone` have the same exposure for pathological `quiet_zone`. Confirmed
  the product goes `None` under `checked_mul`.
  This is caller-supplied config, not untrusted decode data, and the trigger value is absurd —
  hence MINOR, not blocking — but the render API returns a bare `RgbaImage`/`String` with no
  error channel, so a panic is the only failure mode. Recommend: compute the side with
  `checked_mul` and clamp/saturate (returning a best-effort or empty image), or at minimum
  document the `scale`/`quiet_zone` precondition on the public fns. A tiny test at the overflow
  boundary would lock it in.

## Escalate to Architect
None. (The MINOR above is a code-level hardening choice, not a product decision. If Arch
wants a hard cap or a `Result`-returning variant that is a design call, but it is not required
to clear this step.)

---

## Rulings on Bob's open questions

1. **SVG alpha precision** — Fine as-is. `128/255 → 0.502` (3-dp, trailing zeros trimmed) is
   valid CSS and round-trips visually. No change.
2. **ASCII polarity** — CORRECT as implemented. Verified `DARK = "██"` (U+2588 ×2) for `true`
   modules, spaces for light (`src/lib.rs:189-190`). Dark modules render visually dark. Keep it.
   (Bob's REVIEW-REQUEST note rationalized it as "dark on light terminal"; the reasoning is
   moot — the implementation matches the required convention regardless of terminal theme.)
3. **PNG wrapper** — Correctly omitted. Out of scope; `RgbaImage` suffices and the caller uses
   `image`'s own `save`/`write_to`. Do not add.

## What I checked and it passed

- **Dimension math**: `(size + 2*quiet) * scale`, identical for image and SVG. Correct.
- **Quiet zone**: canvas pre-filled with background; modules offset by `quiet`. Band + corner
  test proves it. No off-by-one.
- **Module→pixel mapping**: `px0 = (mx+quiet)*scale`, fills exactly `scale×scale`. No off-by-one.
- **(x,y)=(col,row)**: loops `my` rows / `mx` cols, calls `get(mx, my)` everywhere. Consistent
  with qr-core and the brief.
- **SVG path**: `M x y h s v s h-s z` draws a correct closed square; `crispEdges`; `viewBox`
  matches width/height. No user text is embedded, so no XML-escaping gap (colors are `#rrggbb`
  or `rgba(...)` only). Empty matrix emits no `<path>`. All correct.
- **Color/alpha**: opaque → `#rrggbb`, translucent → `rgba(r,g,b,frac)`; raster passes RGBA
  straight through `put_pixel` (replace, not blend — acceptable for RGBA output).
- **API soundness**: `Default` sane (scale 8, quiet 4, black/white); `scale.max(1)` prevents
  0-px image (tested); zero-size/zero-quiet inputs produce empty output without panic.
- **Test adequacy**: spot-checks assert fg at a dark coord and bg at a light coord, so a
  flipped module fails; module geometry + quiet-zone-band tests fail on a shifted quiet zone.
  Adequate.
- **Deps / unsafe**: only `qr-core` + `image` (both already in workspace deps; root untouched).
  No `unsafe`. No needless complexity. No scope drift worth flagging.

## Cleared
Reviewed `crates/qr-render/src/lib.rs` and `Cargo.toml`: three renderers, `Options`, and tests.
Correctness, API, and coverage are sound. One MINOR overflow-hardening note, non-blocking.
Step 2c is clear.
