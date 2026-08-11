# Review Feedback — Step 12b, demo camera-path quality

Reviewer: Richard-CAM
Date: 2026-08-11
Scope reviewed: `git diff HEAD -- demo-wasm` (Cargo.toml, Cargo.lock, src/lib.rs,
www/app.js, README.md). `crates/qr-detect` deliberately not reviewed — sibling
Builder owns it this session.

**Verdict: Step 12b CLEAR**
Ready for Builder: YES — with the two MAJORs fixed before this is committed.

Counts: 0 BLOCKER · 2 MAJOR · 9 MINOR

---

## How I verified it

The working tree does not build. The sibling Builder's in-flight
`crates/qr-detect` edits fail to compile (`binarize.rs:201` and `:205`, E0061 —
`can_be_dark` and `adaptive` called with an extra argument), which takes
`demo-wasm` down with them. That is not Bob-CAM's doing.

So I re-ran everything in a detached worktree at HEAD with only Bob's five
`demo-wasm` files copied over it:

* `cargo fmt --check` — clean.
* `cargo clippy --release --target wasm32-unknown-unknown --all-targets -- -D warnings` — clean.
* `wasm-pack build --release --target web` — clean.
* `node --check www/app.js` — clean.

Bob's throwaway script did not survive, so I wrote my own native harness against
the real `Scanner` (temporarily adding `rlib` to the *worktree copy's*
crate-type; the repo's `Cargo.toml` was not touched). Seven cases, all pass:

* exactly one `qr:first` over 12 frames of the same code; repeats carry four
  corners; ROI engages and `via_roi` goes `true`;
* exactly one `qr:lost` once the code leaves;
* `reset()` clears the ROI (`roi()` returns `"null"`) and the payload comes back
  `first`;
* micro detected on the camera path with `"version":2`; exactly one
  `micro:first` and exactly one `micro:lost`;
* empty, short, oversized and transposed buffers all return `[]` without
  panicking; `NaN`, `-5.0`, `+inf` and `1e30` timestamps likewise;
* a zero-sized frame survives the micro-pass cadence frame (`w=h=0`,
  `luma.len()==0` passes the `well_formed` guard and reaches
  `detect_micro_luma` — it does not panic).

Bob's claims in the review request hold. The one probe that came back different
from what I expected is MAJOR 1 below.

---

## MAJOR

**M1 — `src/lib.rs:527` `fnv1a` uses the wrong prime, and says it uses the right one.**

The constant is `0x1000_0000_01b3` = 17,592,186,044,851. The FNV-1a 64-bit
prime — and the value `qr-video` actually uses (`FNV_PRIME` at
`crates/qr-video/src/detector.rs:490`, `0x0000_0100_0000_01b3`) — is
1,099,511,628,211. A digit group too many.

The docstring says *"matching the identity hash qr-video uses so both
symbologies key their tracking tables the same way."* They do not match. Proven,
not inferred: a QR and a Micro QR both carrying `12345` in one frame come back as

```
{"kind":"qr",   ...,"key":"e575e8883c0f89f8","bytes_hex":"3132333435"}
{"kind":"micro",...,"key":"503cd4883c0f89f8","bytes_hex":"3132333435"}
```

Same bytes, different key.

Nothing misbehaves today — any odd multiplier gives a stable per-payload table
key, and that is all the micro table needs. What I object to is a constant that
is wrong and carries a comment asserting it is right. The next person to touch
this will trust the comment.

Fix: `0x0000_0100_0000_01b3`. **Do not land that change on its own** — see M2.

**M2 — `www/app.js:250-268` keys the overlay map on `e.key` alone, ignoring `e.kind`.**

`live.set(e.key, …)` and `live.delete(e.key)` share one namespace across both
symbologies. The only thing currently keeping a QR and a Micro with an identical
payload apart is M1's wrong prime. Correct the prime and they collide: the
second `set` overwrites the first, so two symbols in frame draw one box, and a
`micro:lost` deletes a live QR's box.

Fix: key on `` `${e.kind}:${e.key}` `` in `applyEvents`. Cheap, and it makes M1
safe to fix. Land the two together.

---

## MINOR

**m1 — `www/app.js:378-390`.** The `catch` around `JSON.parse(scanner.…)` treats
every failure as "the wasm module failed" and stops the camera. A malformed
*number* in the JSON is a per-frame data problem, not a module failure — a
non-finite corner would be written by `{:.1}` as bare `NaN`, which is not JSON,
and the session dies. The previous `runScan` merely set an error status and kept
going. Either separate the two failure modes, or guard the float formatting in
`push_corners`.

**m2 — `www/app.js:303` `tuneTrack`.** `track.getCapabilities?.()` guards
*absence* but not a *throw*, and the call sits outside `startCamera`'s `try`
(which wraps only `getUserMedia`). A throwing implementation aborts `startCamera`
halfway: `stream` is already assigned and live (camera indicator on), `video`
stays hidden, `startCam` stays enabled — and the next click leaks the first
stream. Wrap the body of `tuneTrack`, or its call site.

**m3 — `src/lib.rs:319` / `README.md:122`.** `Scanner.roi()` is exported,
documented, and never called by `app.js`. Its own docstring says it is "exposed
so the page can say whether the fast path is engaged" — the page does not. Wire
it into the status line or drop the export. Bob dropped the unused luma entry
point for exactly this reason; be consistent.

**m4 — `www/app.js:185-199` `recordResults`.** On the camera path only `first`
events reach it, so a listed payload's timestamp now freezes at its first
sighting and the "refresh, but only once a second" branch is dead there. The
comment still describes the old behaviour. Pick one and make the comment true.

**m5 — `www/app.js:290-295`.** `started` is taken after `grabFrame`, so the
status line's "ms/scan" excludes `drawImage` + `getImageData` at 1440×810 —
a real per-frame cost, and the Owner is being asked to judge the loop by this
number. Move `started` above `grabFrame`, or relabel it.

**m6 — `src/lib.rs:352` `well_formed`.** The guard compares the *derived* luma
length against `w*h`, not `rgba.len()` against `4*w*h`. A buffer of the right
byte count but transposed geometry (100×200 declared as 200×100) passes as
well-formed and gets scanned as garbage. It never panics — verified — and
`app.js` always passes matching values, but the docstring's "`rgba` must be
`width * height * 4` bytes" is not actually enforced. A direct
`rgba.len() as u64 == 4 * w as u64 * h as u64` check up front is exact and
cheaper than the conversion it would skip.

**m7 — micro cadence vs `OVERLAY_TTL_MS`.** The arithmetic holds at the stated
settings: `SCAN_INTERVAL_MS` 66 with a 16.7 ms rAF quantises to ~66.7 ms, so the
micro pass lands every ~200 ms and the 300 ms TTL clears it with 100 ms of
margin. The margin is gone once the *effective* frame interval passes 100 ms —
i.e. once a scan costs ~35 ms on a phone and the loop self-throttles. The result
is a blinking micro box, **not** dropped tracking: `micro_tracks` only evicts a
key that was *absent* from a pass, so a continuously visible micro is never
lost. Acceptable. But the TTL is coupled to two constants it does not reference
— make it `SCAN_INTERVAL_MS * MICRO_SCAN_INTERVAL * 2`, or say so in the
comment. (Same class, further out: if the effective micro interval ever exceeds
qr-video's 750 ms debounce — about 4 fps — every micro pass re-reports `first`.
Harmless, `seen` dedups by `kind:text`, but worth knowing.)

**m8 — `src/lib.rs:539` `hash_matrix`.** One byte per module and an XOR of two
hashes, where qr-video packs bits into a single stream. It is only ever used for
an undecoded micro symbol, so it never has to agree with anything — but the
near-identical docstring invites the reader to assume it does. One line saying
it is deliberately independent settles it.

**m9 — pre-existing, flagged so it is not mistaken for new.** `grabFrame` draws
onto the work canvas without clearing, and the luma reduction ignores alpha, so
a transparent-background PNG on the file path reduces to a black field.
Identical to the old `toLuma`. Not introduced here; not for this step.

---

## Cleared

**Timestamps.** `clamp_timestamp` handles non-finite, negative and absurd values;
`as u64` is a saturating cast in Rust, so no UB and no panic. Truncating a
monotonic clock leaves it monotonic, and sub-millisecond rAF fractions collapse
without regressing. `app.js` feeds rAF's `now` to both the wasm timestamp and
the overlay TTL — same clock as `performance.now()`, so the two cannot drift.

**Event JSON contract.** `first`/`repeat` carry four corners; `lost` carries
`"corners":null` and is handled by an early `continue` before `live.set`, so a
null never reaches `drawOverlay`. Text goes through the existing
`push_json_string` — quote, backslash, `\n \r \t`, and everything under `0x20`
as `\u00xx`; bytes through `push_hex_string`. `key` is emitted as a 16-digit hex
*string*, correctly sidestepping JSON's 53-bit number limit. Shapes are
consistent across `qr` and `micro`.

**Micro tracking table growth.** Bounded. Every key inserted on a pass is either
re-seen on the next pass or evicted once `now - last >= lost_after_millis`
(500 ms), and the eviction runs inside the same pass as the insert. Same shape
as qr-video's own `emit_lost`. Undecoded symbols keyed by matrix hash are
evicted on the same rule, so a noisy scene cannot grow it either.

**`reset()`.** Clears both trackers and the counter — rebuilds the
`VideoDetector` from the stored `Config`, clears `micro_tracks`, zeroes
`frame_index`. Test-confirmed.

**RGBA → luma.** 77 + 150 + 29 = 256 exactly, so `>> 8` cannot overflow and the
maximum is 255. Truncation instead of rounding costs at most one level —
irrelevant ahead of binarization. Alpha ignored, correct for opaque video
frames. `chunks_exact(4)` cannot panic on a short buffer. The reused `luma` Vec
via `mem::take` and `reserve` after `clear` allocates once, not once a frame.

**Uint8Array aliasing.** `new Uint8Array(data.buffer, data.byteOffset,
data.length)` is a view over a JS `ArrayBuffer`, not over wasm linear memory.
wasm-bindgen copies it in at the boundary (`malloc`, then `set`), and its memory
accessor re-acquires the view after a growth. There is no detach and no
use-after-free path here. Each `getImageData` returns a fresh buffer besides.

**`willReadFrequently: true`.** Correct for this access pattern.

**Overlay letterboxing.** `overlayMapper` resizes the canvas to its client box,
scales by `Math.min(ew/sw, eh/sh)` and centres the remainder — matches
`object-fit: contain`. `drawOverlay` clears *after* the mapper may have resized,
which is the right order.

**Listener leaks / start-stop cycles.** All four listeners registered once at
module scope; start/stop touch only `rafId`, `stream` and the two disabled
flags. `loop` re-arms rAF *before* doing any work, so `stopCamera`'s
`cancelAnimationFrame` always cancels a live id — including when the error path
calls it from inside `loop`. No runaway loop, no double-scheduling.

**`seen` map eviction.** `renderResults` trims past `MAX_RESULTS` using a keep-set
built with the same key format `recordResults` writes. Bounded.

**Profile-retraction residue.** Zero. Nothing matching `profile` or a
performance panel in `src/lib.rs`, `www/app.js`, `www/index.html`,
`www/style.css` or `README.md`. (`--panel` in style.css is a pre-existing colour
token.) `index.html` and `style.css` are untouched by this diff.

**The two deferred gaps — correctly deferred, neither half-built.**
*Binarized view:* no reference to binarize anywhere in demo-wasm sources, no
stub control in `index.html` or `style.css`. `qr-detect` at HEAD still has
`mod binarize;` private and exports only `PerspectiveTransform` and `Point`, so
Bob's account is accurate.
*`Observation.version`:* emitted as a literal `null` for `kind:"qr"`, documented
as such in README, and `symbolLabel` degrades to plain `"QR"` with no JS-side
guess. Clean deferral. Both belong with Arch as qr-video / qr-detect API items.

**Cargo.lock.** Exactly the `qr-video` package node plus its line in `qr-demo`'s
dependency list. Nothing else moved.

**Scope.** No file outside `demo-wasm/` touched. Confirmed against `git status`.

---

## Escalate to Architect

1. **`qr_video::Observation` has no `version`** (Bob's risk 1). Agreed, and it is
   a qr-video API decision, not something to work around in the demo. Adding
   `version` — and `kind`, if micro ever moves into qr-video — is the right fix.
   Arch's call.
2. **Micro QR inside `qr-video`** (Bob's risk 3). The separate every-third-frame
   full-frame pass is the correct thing to have built given the crate boundary,
   but it is a workaround with a real cost. Worth a decision on whether qr-video
   grows a micro path.
3. **Phone frame budget** (Bob's risk 4). `MAX_EDGE` 1440 at 66 ms is untested on
   the target device. Self-limiting, visible in the status line, and Bob has
   named the dial to turn. Owner's browser decides this one.
