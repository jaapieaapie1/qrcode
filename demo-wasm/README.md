# Browser demo — QR + Micro QR in WebAssembly

A self-contained page that scans QR and Micro QR symbols from the camera (or an
image file) and generates them as SVG. Everything — binarisation, finder
detection, sampling, Reed-Solomon decoding, encoding and rendering — runs in
`wasm32-unknown-unknown` compiled from the crates in this repository. No CDN, no
framework, no JavaScript QR library.

This crate is **excluded from the root workspace** (`exclude = [..., "demo-wasm"]`)
so its `wasm-bindgen` dependency never enters the product crates' graph.

## Build

Requires the `wasm32-unknown-unknown` target:

```sh
rustup target add wasm32-unknown-unknown
```

The build path used here is **wasm-pack** (`brew install wasm-pack`, or
`cargo install wasm-pack`). From this directory:

```sh
wasm-pack build --release --target web --out-dir www/pkg
```

That writes `www/pkg/qr_demo.js` and `www/pkg/qr_demo_bg.wasm`, which `app.js`
imports as an ES module. `www/pkg/` is generated output and is not committed
(wasm-pack drops a `.gitignore` in it).

<details>
<summary>Without wasm-pack</summary>

```sh
cargo install wasm-bindgen-cli   # version must match the wasm-bindgen dependency
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir www/pkg \
  target/wasm32-unknown-unknown/release/qr_demo.wasm
```

The only difference from the wasm-pack path is that `wasm-opt` is not run, so
the `.wasm` is larger.
</details>

## Serve

The page loads the module with `fetch`, so it must be served over HTTP —
opening `index.html` from the filesystem will not work. Camera access
additionally requires a secure context, which `localhost` counts as.

```sh
cd www
python3 -m http.server 8080
```

Then open <http://localhost:8080/>. Python's `http.server` has served
`application/wasm` for `.wasm` since 3.7, which is the MIME type the streaming
instantiation needs; any static server that does the same works equally well.

To reach the camera from a phone on the same network you need HTTPS (or a
tunnel) — a plain `http://192.168.x.x` origin is not a secure context and
`getUserMedia` will refuse. The "Scan an image file" input works anywhere.

## WebAssembly API

Three surfaces, all from `src/lib.rs`:

### `scan(luma: Uint8Array, width: number, height: number) -> string`

`luma` is `width * height` bytes, row-major, one 8-bit sample per pixel; the
page computes it from a canvas `ImageData` with the integer approximation of
`0.299R + 0.587G + 0.114B`. Returns a JSON array (hand-serialised — no serde in
the dependency list):

```json
[
  {
    "kind": "qr",
    "text": "https://example.com",
    "bytes_hex": "68747470733a2f2f...",
    "corners": [[12.0, 14.5], [180.2, 15.1], [181.0, 183.4], [13.1, 182.0]],
    "version": 2,
    "error": null
  }
]
```

* `kind` — `"qr"` or `"micro"`.
* `text` — the payload as UTF-8, or `null` if it is not valid UTF-8.
* `bytes_hex` — the raw payload bytes, hex-encoded; `null` when undecoded.
* `corners` — image pixel coordinates, top-left, top-right, bottom-right,
  bottom-left.
* `version` — `1..=40` for QR, `1..=4` for Micro (the `n` in `Mn`). Derived from
  the sampled grid size when the symbol did not decode; `null` if that size is
  not a legal symbol size.
* `error` — `null` on success, otherwise a message. **Located-but-undecodable
  symbols are included**, so the overlay can outline them as "found but
  unreadable" instead of silently dropping them.

Never throws: a dimension mismatch or an empty frame returns `[]`.

Used for the file-input path only. The camera goes through `Scanner`.

### `class Scanner`

Stateful scanning across a stream of frames, wrapping `qr_video::VideoDetector`.
What the state buys over calling `scan` per frame:

* **ROI fast path** — after a hit, the next frames are scanned only inside a
  padded box around the last detection.
* **Full-scan cadence** — every 30th frame covers the whole image anyway, so a
  code appearing outside the tracked box is still found.
* **Temporal de-duplication** — a payload held in view is reported once as
  `first`; disappearance is reported as `lost`.

`qr-video` covers QR only, so `Scanner` runs its own full-frame Micro QR pass
every third frame, with a matching tracking table.

```js
const s = new Scanner();
const events = JSON.parse(s.scan_frame_rgba(rgba, w, h, performance.now()));
s.roi();    // "[x,y,w,h]" or "null" — is the fast path engaged?
s.reset();  // drop all cross-frame state (camera restarted)
```

`scan_frame_rgba` takes packed RGBA — canvas `ImageData` order, `w * h * 4`
bytes — and reduces it to luma inside wasm. That quadruples the bytes crossing
the boundary and is still faster: on a 1440x810 frame the JavaScript pixel loop
costs 1.96 ms, the wider copy plus the Rust loop 0.99 ms (node 22, release
build). The boundary copy is a `memcpy`; the pixel loop is not.

Each event is:

```json
{
  "kind": "qr",
  "event": "first",
  "key": "8b1c4f0a2d3e5f60",
  "text": "https://example.com",
  "bytes_hex": "68747470733a2f2f...",
  "corners": [[12.0, 14.5], [180.2, 15.1], [181.0, 183.4], [13.1, 182.0]],
  "version": null,
  "unreadable": false,
  "via_roi": true
}
```

* `event` — `"first"` (not currently tracked), `"repeat"` (still in view) or
  `"lost"` (gone; `corners` is `null`).
* `key` — stable per payload for as long as it is tracked; the overlay uses it
  as its map key.
* `version` — always `null` for `"qr"`: qr-video's observations carry the
  payload but not the symbol geometry, so the streaming path cannot label the
  version. Micro events do carry it.
* `unreadable` — the symbol was located but did not decode.
* `via_roi` — the detection came from the ROI fast path rather than a full scan.

### `generate(text: string, ec: string, micro: boolean) -> string`

Returns a standalone SVG document, or a message beginning with `ERROR:` that
the page shows verbatim. `ec` is `"L" | "M" | "Q" | "H"` for QR. Micro QR
accepts `"L" | "M" | "Q"` plus `"D"` — the M1-only error *detection* selection —
and rejects `"H"`, which no micro version offers.

The smallest version that fits is chosen automatically. Error correction is
never silently downgraded: an impossible combination is reported rather than
quietly re-encoded at a weaker level.

## Page

`www/index.html`, `www/app.js`, `www/style.css` — no build step, no bundler.

* **Scan** — `getUserMedia` asks for `facingMode: environment` at 1920x1080
  ideal, then requests continuous focus and exposure where the track advertises
  them. Frames are scanned on a `requestAnimationFrame` loop throttled to
  ~15 fps, downscaled to a 1440 px longest edge: more pixels per module is the
  main defence against poor light, since sensor noise averages out over a larger
  module area. Detected symbols are outlined on a canvas over the video (green =
  decoded, amber = found but unreadable), each box held for 300 ms past its last
  sighting so the slower Micro pass and the occasional ROI miss do not make it
  blink. Distinct payloads are listed with timestamps, and the status line
  carries the effective frame size and a rolling median scan time. The file
  input runs the one-shot `scan` through `drawImage`, for testing without a
  camera.
* **Generate** — live regeneration on input (120 ms debounce), EC select, Micro
  checkbox, inline error messages, and a download link for the SVG.
