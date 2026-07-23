# Review Feedback — Step 2d qr-detect

**Verdict: Step 2d CLEAR**

Date: 2026-07-23
Reviewer: Richard-T
Ready for Builder: YES (no BLOCKERs, no MAJORs)

Reviewed: Cargo.toml, lib.rs, geometry.rs, binarize.rs, finder.rs, grouping.rs,
sampler.rs, tests/detect.rs. Verified `cargo test -p qr-detect` (default 10+13,
no-default 10+12) and `cargo clippy --all-targets -- -D warnings` — both configs
green and clean.

Counts: 0 BLOCKER, 0 MAJOR, 2 MINOR, plus 3 observations for the log.

---

## BLOCKER
None.

## MAJOR
None.

## MINOR

- **finder.rs:91 — vertical centre biased +0.5 px.**
  `center = (top_dark_edge + 1.0 + bottom_dark_edge) / 2.0`. `top_dark_edge`/
  `bottom_dark_edge` are the first LIGHT rows bracketing the centre dark band, so
  the true midpoint is `(top_dark_edge + bottom_dark_edge) / 2.0`. Worked example:
  dark band on rows 10-12 gives edges 9 and 13, true centre 11.0, code returns
  11.5. This is a uniform half-pixel bias on all finder `cy` values and is
  inconsistent with `horizontal_center` (which returns the true midpoint, no +0.5).
  Sub-pixel and it rounds out in the current fixtures (all pass), but it is a real
  defect. Fix: drop the `+ 1.0`.

- **lib.rs:20-23 — stale doc comment contradicts shipped config.**
  The crate-level doc says the `decode` feature is "off by default until that
  sibling crate lands." `Cargo.toml:17` has `default = ["decode"]` and qr-decode
  has landed. Update the doc to state decode is default-ON (disable via
  `--no-default-features`), matching Cargo.toml.

---

## Observations (log to BUILD-LOG, not blocking)

- **Mirrored symbols are not handled.** `order_corners` (grouping.rs:51) always
  forces `cross >= 0`, i.e. upright handedness. A genuinely mirror-imaged QR gets
  its TR/BL relabelled and samples as its mirror; it will not decode without a
  mirror retry, which is not implemented here. Standard decoders handle mirror at
  the decode stage (try, then transpose+retry). Confirm with Arch whether mirror
  retry is a qr-detect or qr-decode responsibility; not in the Step 2d brief, so
  logging rather than blocking. Rotation (incl. 45°) IS handled correctly — the
  1:1:3:1:1 ratio is scale-invariant along any scanline and the right-angle
  grouping is orientation-free.

- **Test coverage gaps (code paths are safe; traced by hand).** No test exercises:
  (a) dimension-mismatch buffer `luma.len() != w*h` — binarize.rs:46 returns None,
  detect returns empty, safe; (b) uniform non-white / tiny (<7px) image —
  Otsu fallback + run-length guards yield no finders, no panic; (c) finder-like
  noise producing a false positive — the `votes >= 3` cluster filter (finder.rs:196)
  and the geometry gates guard this, but it is untested. Suggest one negative test
  for (c) and one for tiny/uniform input to lock the robustness contract. Existing
  warps are genuinely off-axis (7°, 20°, asymmetric keystone on all four corners)
  and noise (±40 on a 0/255 field) stays the correct side of threshold — adequate.

- **Degenerate homography.** `square_to_quad` divides by
  `dx1*dy2 - dx2*dy1` (geometry.rs:128), which is zero for collinear destination
  corners → NaN. In-pipeline this is unreachable: grouping rejects near-collinear
  triples (`cos.abs() > 0.30`, leg-ratio 0.7..1.4) before a transform is built, and
  the grid-space source quad is always well-formed. Flagging only because
  `PerspectiveTransform` is `pub` and a direct caller could feed degenerate input;
  no action needed for Step 2d.

---

## Spot-checks that passed

- Integral-image sums, window clamping, and the `value*count*t_den < sum*t_num`
  threshold (12% bias) are correct; no usize underflow (radius clamp + w,h >= 24 on
  the adaptive path). Otsu between-class-variance and `<=` threshold correct;
  all-one-colour and small-image fallbacks wired.
- 1:1:3:1:1 tolerance (±module/2 on unit bands, ±1.5·module on the 3× band),
  H/V module agreement, and centre clustering (tol 1.5·module, module within 60%)
  are sane. The `runs.len() - 5` window index cannot underflow (guarded by the
  `< 5` continue).
- `snap_dimension`: rem_euclid(4) remap to `d ≡ 1 (mod 4)`, 21..=177 bound, and the
  post-snap leg-consistency check (`leg_err <= 0.20`) are correct. Version derived
  as `(dim-17)/4`, always >= 1.
- Greedy grouping marks finders used → a finder cannot be consumed by two symbols.
  `corners` ordered TL,TR,BR,BL and documented; matches `sample`'s corner map order.
- Alignment refine: 5×5 template (`(i==0&&j==0) || |i|==2 || |j|==2` = dark)
  correctly models border-ring/gap/centre; basis vectors follow perspective from the
  provisional transform; modpx<1 guard; v1 (no alignment) falls back to the
  parallelogram BR corner cleanly.
- Feature discipline: `decode` default-on verified compiling/testing/clippy-clean
  in BOTH configs; `image` used only for `GrayImage`/`as_raw`. `read_one` prefers a
  decoded detection then first; `detect` returns all.
