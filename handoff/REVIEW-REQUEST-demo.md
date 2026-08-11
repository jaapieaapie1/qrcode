# Review Request — Step 8 browser demo (WASM)

**Builder:** Bob-W
**Ready for Review:** YES
**Scope lock honored:** new `demo-wasm/` tree, one root `Cargo.toml` line
(`exclude`), one `.gitignore` line, one new CI job. No product crate touched —
`crates/` is byte-identical to what I found.

## What this is

A self-contained browser page that scans QR + Micro QR from the camera (or an
image file) and generates them as SVG, with every algorithm running in
`wasm32-unknown-unknown` built from this repo's crates. No CDN, no framework, no
bundler, no JavaScript QR library.

`demo-wasm` is **excluded from the root workspace**, exactly like `esp-selftest`,
so its `wasm-bindgen` dependency never enters the product crates' graph and the
`lint`/`test` jobs' `--workspace` globs cannot see it. It carries its own
`Cargo.lock` (committed, same convention as `esp-selftest`).

## Files

- `Cargo.toml` (root, line 14-16) — `exclude = ["esp-selftest", "demo-wasm"]`
  plus the comment explaining why. Only root edit.
- `.gitignore` (line 2) — `demo-wasm/www/pkg/` (generated build output).
- `.github/workflows/ci.yml` (new `demo-build` job, before `esp32-emulation`) —
  see CI section below.
- `demo-wasm/Cargo.toml` (new, 25 lines) — cdylib; deps `qr-core`, `qr-encode`,
  `qr-render`, `qr-detect` (path) + `wasm-bindgen 0.2`. Release profile
  `opt-level = "s"`, lto, `panic = "abort"`.
- `demo-wasm/src/lib.rs` (new, ~270 lines) — the two `#[wasm_bindgen]` exports
  and the hand-rolled JSON writer.
- `demo-wasm/www/index.html` (new, ~85 lines) — two sections, no inline script.
- `demo-wasm/www/app.js` (new, ~330 lines) — camera + file scan loop, overlay,
  results list, live generation.
- `demo-wasm/www/style.css` (new, ~180 lines) — minimal, light/dark via
  `prefers-color-scheme` + `color-scheme: light dark`.
- `demo-wasm/README.md` (new) — build path, serve instructions, API reference.

## WASM API

### `scan(luma: Uint8Array, width: u32, height: u32) -> String` (JSON)

Runs `qr_detect::detect_all_luma` and returns a JSON array; one object per
symbol from **both** symbologies:

```json
{ "kind": "qr"|"micro", "text": string|null, "bytes_hex": string|null,
  "corners": [[x,y],[x,y],[x,y],[x,y]], "version": number|null,
  "error": string|null }
```

- `text` is `Decoded::text` (null when the payload is not valid UTF-8);
  `bytes_hex` is the raw payload so binary/Shift-JIS payloads are still visible.
- `version` is `1..=40` (QR) or `1..=4` (Micro). When the symbol did **not**
  decode there is no `Decoded` to ask, so it is derived from the sampled grid
  side (`4v+17` / `2v+9`) and is `null` if that side is not a legal symbol size.
- **Failed-decode candidates are included** with `error` set and payload fields
  null, per the brief, so the overlay can outline them as "found but unreadable".
  The message is generic (`"located but not decodable (29x29 modules sampled)"`)
  because `Detection` carries `Option<Decoded>`, not the `DecodeError`. Getting
  the real error would mean adding a `qr-decode` dep and re-running `decode` on
  the matrix; I kept the dep list to the four the brief named. **Flag if you
  want the specific error instead** — it is a 5-line change.
- Returns `[]` (never throws) on dimension mismatch, empty buffer, or no hit.

Return type is `String`, not `JsValue`: `JsValue` for a structured object needs
either `serde-wasm-bindgen` or manual `js_sys::Object` building, and the brief
ruled out serde. JS does one `JSON.parse`.

### `generate(text: &str, ec: &str, micro: bool) -> String` (SVG or error)

`render_svg` over `encode` / `encode_micro`, scale 8, quiet zone 4 (QR) / 2
(Micro, per ISO). Errors return `"ERROR: …"` — the `Display` impls of
`EncodeError`/`MicroEncodeError` are already end-user readable, so they are
passed through with the prefix.

**The one API decision worth your attention:** error correction is never
silently downgraded. `micro=true` with `ec="H"` returns
`ERROR: Micro QR has no level H — M4 tops out at Q. Pick Q, or uncheck Micro.`
rather than quietly re-encoding at Q. The alternative reading of "auto ec
handling" — walk down the levels until something fits — would hand the user a
symbol at an EC level they did not ask for and cannot see, which is exactly the
kind of silent capability change I would not ship in a library demo.

The cost of that strictness is that M1 (numeric-only, detection-only) becomes
unreachable, since no L/M/Q/H selection maps to it. So `generate` also accepts
`ec="D"` = `MicroEcLevel::Detection`, micro-only, and the page shows that option
only when Micro is checked (and hides `H` then). QR + `"D"` and unknown letters
are both readable errors. This is a small extension beyond the brief's "L/M/Q/H
select"; it keeps all of M1–M4 demonstrable with no hidden behaviour. Say the
word and I will drop it.

## Page behaviour

- **Scan** — `getUserMedia({ facingMode: { ideal: 'environment' } })`, scanned
  on a `requestAnimationFrame` loop gated to one scan per 100 ms. Each frame is
  drawn to an offscreen canvas downscaled to a 720 px longest edge, converted to
  luma in JS with the integer approximation `(77R + 150G + 29B) >> 8`, and
  passed to `scan` as a `Uint8Array`. That is why **no `web-sys` dependency** is
  needed: all DOM work stays in JS and wasm only ever sees a raw buffer.
- Overlay canvas over the video: green quad = decoded, amber = located but
  unreadable, labelled `QR v3` / `Micro M2`. Detector coordinates are mapped
  through the `object-fit: contain` letterbox rather than a naive width ratio,
  so a portrait camera frame in a landscape box still lines up.
- Decoded list: deduped by `kind + payload`, newest first, max 8, with
  timestamps. A code held in front of the camera decodes 10x/second, so repeats
  only refresh the timestamp (at most once a second) and the map is pruned to
  the visible 8 so it cannot grow unbounded during a long session.
- The results list is built with `document.createElement` + `textContent` —
  scanned payloads are attacker-controlled and never touch `innerHTML`. The only
  `innerHTML` write is the SVG from our own renderer (commented as such).
- **File input** takes the same path via `drawImage`, for testing without a
  camera; it stops the camera loop first.
- **Generate** — live regeneration on input (120 ms debounce), EC select, Micro
  checkbox, errors inline, download link via `Blob` URL (previous URL revoked).

## Build path chosen

**wasm-pack** (`brew install wasm-pack`, 0.15.0 locally):

```sh
cd demo-wasm && wasm-pack build --release --target web --out-dir www/pkg
cd www && python3 -m http.server 8080
```

It bundles the matching `wasm-bindgen` CLI (no version-skew footgun) and runs
`wasm-opt`. README documents the plain
`cargo build --target wasm32-unknown-unknown` + `wasm-bindgen-cli` fallback for
anyone who does not want it. Output is 112 KB of wasm, unzipped.

## CI

New `demo-build` job (ubuntu, wasm32 target, cache scoped to `demo-wasm`):

1. wasm-pack installed from a **pinned** GitHub release tarball
   (`v0.15.0`, musl) rather than a third-party action or a `curl | sh`
   installer — verified the URL returns 200.
2. `cargo fmt --check` + `cargo clippy --release --target wasm32-unknown-unknown`
   in `demo-wasm`, because the excluded crate is invisible to the `lint` job's
   `--workspace` (same reasoning, and same comment, as the `esp32-emulation`
   job).
3. `wasm-pack build --release --target web --out-dir www/pkg`.
4. Asserts `pkg/qr_demo.js` + `pkg/qr_demo_bg.wasm` exist and that `app.js`
   actually imports that path — cheap guard against the page and the build
   output drifting apart. No browser test.

## What I verified

- `wasm-pack build --release --target web` — clean, 112 KB wasm + 9 KB JS glue.
- `cargo fmt --check` and `cargo clippy -- -D warnings` in `demo-wasm` — clean
  for the wasm32 target.
- `cargo test --workspace` — green, 0 failures (product crates untouched).
- `cd esp-selftest && cargo build --release` — still green.
- **Headless round trip through the real wasm binary.** I built a second
  `--target nodejs` package into the scratchpad (not committed) and ran a node
  script that calls the actual exports: `generate` an SVG, rasterise its module
  rects back into a luma plane, feed that to `scan`, assert the payload comes
  back. 21 assertions, ALL PASS, covering: six generate→scan round trips (QR
  L/M/Q/H incl. non-ASCII UTF-8; Micro M1 via `D`, M2, M4); every error path
  (empty text, micro+H, QR+D, bogus letter, oversized QR payload, oversized
  micro payload); `scan` edge cases (blank frame, dimension mismatch, zero
  size → `[]`); JSON shape and escaping (a payload containing `"`, `\t` and `\`
  survives `JSON.parse` byte-exact, all six keys present, 4 corner pairs,
  `error: null`, hex payload); and a deliberately corrupted-but-locatable symbol
  returning `error != null` **with** its four corners — the "found but
  unreadable" path the overlay depends on.
- `python3 -m http.server` serves `/`, `app.js`, `style.css`, `pkg/qr_demo.js`
  (200, `text/javascript`) and `pkg/qr_demo_bg.wasm` (200, **`application/wasm`**
  — the MIME streaming instantiation needs).
- `node --check` on `app.js` (ES module syntax).

## What needs browser verification (Arch)

Everything DOM-side is unexercised by the above:

1. Module load in a browser (`init()` fetching `qr_demo_bg.wasm` over HTTP).
2. `getUserMedia` permission flow, environment-facing selection, and whether
   ~10 fps at 720 px is comfortable on a real phone.
3. Overlay alignment against a live video element — the `object-fit: contain`
   mapping is reasoned, not measured.
4. The file-input path with a real photo (my headless test used a synthetic
   rasterisation of our own SVG, which is a much kinder image than a camera
   frame).
5. Live generate typing feel, the Micro/EC option swap, and the download link.
6. Light and dark rendering.

Note for whoever tests on a phone: `getUserMedia` needs a secure context, so a
plain `http://192.168.x.x` origin will not work — localhost or HTTPS only. The
file input works anywhere. This is documented in the README.

## Open questions

1. **Generic vs specific decode error** in `scan` — worth a `qr-decode` dep to
   surface the real `DecodeError`? (See the `scan` section.)
2. **`ec="D"`** — keep the micro-only detection-only selection, or drop it and
   accept that M1 is not demonstrable?
3. `demo-wasm/Cargo.lock` is committed, matching `esp-selftest`. Confirm that is
   the convention you want for excluded crates.
4. Nothing is committed or staged — per BUILDER.md the commit waits on review.
   The working tree now holds exactly the files listed above and nothing else.
