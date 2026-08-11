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

Two exports, both from `src/lib.rs`:

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

* **Scan** — `getUserMedia` with `facingMode: environment` preferred, scanned on
  a `requestAnimationFrame` loop throttled to ~10 fps. Frames are downscaled to
  a 720 px longest edge before detection. Detected symbols are outlined on a
  canvas over the video (green = decoded, amber = found but unreadable) and
  distinct payloads are listed with timestamps. The file input runs the same
  path through `drawImage`, for testing without a camera.
* **Generate** — live regeneration on input (120 ms debounce), EC select, Micro
  checkbox, inline error messages, and a download link for the SVG.
