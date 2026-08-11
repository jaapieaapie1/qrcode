# Review Request — KG-6 full encoder segmentation (optimal mixed-mode, Kanji, ECI)

**Builder:** Bob-SEG
**Scope:** `crates/qr-encode/` only
**Round:** 2 (round 1 verdict: BLOCKED — 1 BLOCKER, 5 MINOR)
**Ready for Review: YES**

> **Round 2 readers start at "Round 2 — response to review" at the bottom.**
> Sections 1–4 below describe the design as it now stands, with the round-2
> changes folded in.

---

## What changed

| File | Lines | What / why |
|---|---|---|
| `crates/qr-encode/src/segment.rs` | rewritten, 460 | `Segment`, the segmentation dynamic program (now gated on `allow_kanji`), Kanji encoding and the ECI header. Kept `alnum_value`/`payload_bits`/`write_payload` semantics; removed the now-unused `select_mode` and `total_bits`. |
| `crates/qr-encode/src/lib.rs` | 23–39, 92–94, 112–161, 168–235, 271–281 | `Options.eci`, `Options.assume_shift_jis`, `EncodeError::InvalidEci`, `resolve_plan` (replaces `resolve_mode` + `resolve_version`), `build_data_codewords` now takes segments + eci. Crate docs rewritten for the new scope. |
| `crates/qr-encode/src/micro.rs` | 138–150 | 4-line `mode != MicroMode::Kanji` guard in `select_mode` — behaviour-freezing only (see "Micro" below). Approved by Arch. |
| `crates/qr-encode/src/tests.rs` | +380 | New unit tests incl. the brute-force optimality check; two `build_data_codewords` call sites adapted to the new signature; `eci:` / `assume_shift_jis:` added to `Options` literals. |
| `crates/qr-encode/tests/roundtrip.rs` | 7 literals | `eci:` / `assume_shift_jis:` added to `Options` literals. No assertion changed. |
| `crates/qr-encode/tests/segmentation.rs` | new, 329 | Encode → `qr_decode::decode` round-trips for mixed-mode, Kanji and ECI, plus the Kanji opt-in regressions. |

Public API delta is three additions: `Options.eci: Option<u32>`,
`Options.assume_shift_jis: bool` and `EncodeError::InvalidEci`.
`encode(&[u8], Options)` is unchanged in signature.

---

## 1. Optimal mixed-mode segmentation

`segment::optimal_segments(data, version, allow_kanji) -> Vec<Segment>`
(segment.rs:253).

A shortest-path dynamic program over states `(byte offset, mode)` with
`mode ∈ {Numeric, Alphanumeric, Byte}`, plus `Kanji` when `allow_kanji` is set.
Two edge kinds:

- **continue** — encode the next character in the current mode. Cost is the
  per-character width; the step is 1 byte, or 2 for Kanji.
- **switch** — close the open segment and open one in another mode. Cost is the
  running total rounded up to a whole bit, plus the new segment's
  `4 + char-count-indicator` header.

Costs are carried in **sixths of a bit** (`STEP = [20, 33, 48, 78]`) so numeric's
10/3 and alphanumeric's 11/2 stay exact; rounding a run of *L* characters up to a
whole bit reproduces the ISO formulas exactly (numeric `⌈20L/6⌉` = `10⌊L/3⌋ + {0,4,7}`,
alphanumeric `⌈33L/6⌉` = `11⌊L/2⌋ + 6(L mod 2)`). Switch edges are relaxed from a
snapshot of the arrival costs, so two switches at the same offset cannot chain —
never beneficial, since header costs are strictly positive.

**Optimality.** Every single-mode encoding is itself a path through this graph, so
the DP result can never exceed it (`dp_never_loses_to_single_mode`, 600 random
payloads × 3 version groups × both Kanji settings). Optimality proper — DP cost
== true minimum over *all* segmentations, and the backtracked segments costing
what the DP reported — is checked by exhaustive brute force in
`dp_matches_brute_force_optimum` (see m2 below).

**Version groups.** Segmentation depends on the version only through the ISO
Table 3 character-count widths, i.e. the groups `1..=9 / 10..=26 / 27..=40`. So
`resolve_plan` (lib.rs:172) runs the DP once per group at that group's first
version and scans versions inside the group — still the smallest fitting version,
because the bit count is constant within a group. This is the same shape as
Nayuki's optimal algorithm.

**Forced mode.** `Options.mode` still produces exactly one segment covering the
whole payload (`segments_for`, lib.rs:179); the DP is not consulted.

## 2. Kanji mode — ISO/IEC 18004 §7.4.6

`segment::kanji_value(b1, b2) -> Option<u32>` (segment.rs:69), table-free:

1. Shift-JIS codes `8140`–`9FFC` have `8140` subtracted; `E040`–`EBBF` have `C140`.
2. The high byte of the difference is multiplied by `C0` and the low byte added.
3. Result is written as 13 bits.

The `low byte ≤ BF` guard is what makes
`qr_decode::segment::push_kanji` reproduce the input bytes exactly — the two range
branches map to disjoint difference spaces either side of the decoder's `0x1F00`
split point. (The round-1 `value ≤ 0x1FFF` guard was dead code and is gone; see
m5 below.)

**Shift-JIS contract — Kanji is opt-in** (documented at segment.rs:8–22 and in
the crate docs). Kanji mode transports *Shift-JIS character values*, not bytes: a
conformant reader converts the 13-bit values back to Shift-JIS characters, so
non-Shift-JIS bytes in a Kanji segment render as mojibake on every scanner other
than this workspace's own byte-transparent decoder. This crate performs no
character-set conversion and a `&[u8]` carries no encoding, so Kanji is never
inferred from byte values. It requires an explicit caller assertion:

- `Options::assume_shift_jis` (default `false`) — the DP may use Kanji segments.
- `Options::mode = Some(Mode::Kanji)` — one forced Kanji segment.

By default UTF-8 (and anything else) stays in byte mode even where its bytes fall
inside the Shift-JIS ranges, which ordinary UTF-8 CJK text does constantly.

`mode_supports(Mode::Kanji, ..)` now requires an even length whose every pair is
encodable, so a forced Kanji mode on non-Shift-JIS input is a clean
`ModeMismatch`, not a panic.

## 3. ECI — ISO/IEC 18004 §7.4.3.2, Table 4

`Options.eci: Option<u32>` emits, before the data segments, the `0111` ECI mode
indicator plus the shortest designator §7.4.3.2 permits:

| Assignment value | Codewords | Pattern |
|---|---|---|
| `0..=127` | 1 | `0bbbbbbb` |
| `128..=16383` | 2 | `10bbbbbb bbbbbbbb` |
| `16384..=999999` | 3 | `110bbbbb bbbbbbbb bbbbbbbb` |

`segment::eci_bits` (segment.rs:183) feeds the same 12/20/28 bits into version
selection, so the header is counted against capacity rather than silently
overflowing. Out-of-range values are `EncodeError::InvalidEci`.

## 4. Micro

`MicroOptions` has no ECI field and gained none, so `encode_micro` cannot be
handed an ECI — there is nothing to reject at runtime.

The one micro.rs edit is a behaviour freeze, not a feature: `segment::mode_supports`
had to start accepting `Mode::Kanji` for the QR forced-mode path, and micro's
`select_mode` relied on it returning `false` to keep Kanji unreachable. The guard
is now explicit (`mode != MicroMode::Kanji`) with the doc comment updated to say
why. Micro output is bit-identical to before. Approved by Arch after the
collision report; I did **not** touch the M1/M3 half-codeword logic or
`tests/micro.rs` — Bob-IV owns those.

---

## Test evidence

`cargo test -p qr-encode` — **65 passed, 0 failed** (27 lib + 15 micro + 11 roundtrip
+ 12 segmentation). Round-2 gate table at the bottom.

New tests:

**ISO vectors**
- `iso_7_4_6_kanji_example_values` — the §7.4.6 EXAMPLE verbatim: 点 `935F` → `0D9F`,
  茗 `E4AA` → `1AAA` (verified against the PDF, not from memory).
- `iso_7_4_6_kanji_example_bits` — the same pair as a 26-bit stream,
  `0 1101 1001 1111` + `1 1010 1010 1010` → `6C FE AA 80`.
- `kanji_range_boundaries_are_exact` — both range endpoints and five rejections.
- `eci_designator_{one,two,three}_codewords` — exact bytes for ECI 26 (`71 A0`),
  128 (`78 08 00`) and 999999 (`7C F4 23 F0`), plus the 127/128/16383/16384 boundaries.

**DP**
- `dp_never_loses_to_single_mode` — the required property: 600 seeded random
  payloads over a mode-mixing alphabet × 3 version groups × both Kanji settings,
  asserting DP bits ≤ single-mode bits, that segments are contiguous, cover the
  payload and each hold bytes its mode can represent, and that no Kanji segment
  appears with the flag off.
- `dp_matches_brute_force_optimum` — exhaustive optimality, added in round 2 (m2).
- `short_digit_run_stays_alphanumeric` — `"ABC123456789DEF"` stays **one**
  alphanumeric segment (96 bits); splitting the digits out would cost 104. A greedy
  splitter fails this.
- `long_digit_run_splits_out_of_alphanumeric` — 30 digits between two alnum runs
  splits 3-way at the exact offsets.
- `url_with_digits_splits_the_numeric_tail`, `pure_payloads_keep_their_single_mode`,
  `empty_payload_is_one_empty_byte_segment` (byte-identical output to the old encoder
  for empty input).
- `version_auto_select_is_still_the_smallest_fit` — for nine payload lengths, every
  version below the chosen one is proven not to fit.

**Round-trip through the independent decoder** (`tests/segmentation.rs`)
- `mixed_mode_strings_roundtrip` — 8 known mixed strings × 4 EC levels.
- `random_mixed_payloads_roundtrip_exactly` — 200 seeded random payloads including
  Shift-JIS bytes; decoded bytes must equal input exactly.
- `shift_jis_payload_roundtrips_through_kanji_segments` — 1–40 repeats × 4 levels.
- `kanji_beats_byte_mode_on_capacity` — 400 Shift-JIS bytes fit v12-L as Kanji
  (2614 bits) and are `DataTooLarge` forced into byte mode (3220 bits): proof Kanji
  is actually selected, not merely tolerated.
- `forced_kanji_mode_roundtrips_and_rejects_non_sjis`, `mixed_mode_never_needs_a_larger_version`.
- `eci_26_tagged_utf8_decodes_to_text` — `decoded.text == Some("Grüße, 世界 — 123")`.
- `every_eci_designator_width_roundtrips` — 10 assignment numbers across all three widths.
- `eci_header_is_counted_against_capacity`, `eci_out_of_range_is_a_clean_error`.

## Definition of Done

- [x] `cargo test -p qr-encode` green — default features
- [x] `cargo test -p qr-encode --no-default-features` green
- [x] `cargo clippy -p qr-encode --all-targets -- -D warnings` green, both configs
- [x] `cargo fmt -p qr-encode -- --check` clean
- [x] `cargo check --workspace --all-targets` clean
- [x] `cargo build -p qr-encode --target thumbv7em-none-eabihf` — both feature configs
- [x] `cargo build -p qr-encode --target riscv32imc-unknown-none-elf` — both feature configs
- [x] `cd esp-selftest && cargo build --release` green (path-dep on qr-encode intact)
- [x] `cargo test -p tests-integration` green (81 tests) — downstream consumers unaffected
- [x] No `unsafe`, no new dependencies, `no_std` + `alloc` preserved

## Notes for the reviewer

1. **Existing tests were not weakened**, but two kinds of mechanical edit were
   unavoidable: `eci: None` added to 11 `Options` struct literals (adding a public
   field breaks literal construction), and the two `build_data_codewords` call sites
   in `src/tests.rs` now pass a `&[Segment]` instead of a `Mode`. No assertion
   changed. `Options` is deliberately *not* `#[non_exhaustive]` — that would break
   the literals in `tests/roundtrip.rs`, which is a separate crate.
2. **Behaviour change worth knowing:** default-options output changes for payloads
   containing numeric or alphanumeric runs (they now split into cheaper segments).
   Kanji does *not* fire by default, so no payload class silently changes character
   set. Round-trip is byte-exact throughout.
3. `Segment`, `optimal_segments` and friends are `pub` inside the private `segment`
   module — crate-internal, not exported.
4. Complexity is `O(n)` time and memory: per byte offset the DP holds `[u32; 4]`
   costs (16 B) and `[u8; 4]` back-pointers (4 B), so **20 B per offset — ~58 KB**
   for a 2953-byte v40 payload. Transient, freed before masking. Packing the cost
   array to `u16` is *not* available: the worst-case cost is 2953 × 48 = 141,744
   sixths, which overflows `u16`.

---

# Round 2 — response to review

Richard-SEG's round-1 verdict: **BLOCKED**, 1 BLOCKER + 5 MINOR. Arch ruled on the
two escalations. All six items are addressed; nothing was deferred.

## B1 — Kanji auto-selected for non-Shift-JIS bytes — **FIXED**

Arch ruled the opt-in shape: an `Options` flag, defaulting off.

`Options.assume_shift_jis: bool` (lib.rs:145) is threaded through
`segments_for` (lib.rs:179) into `optimal_segments(data, version, allow_kanji)`
(segment.rs:253). With it `false`:

- the `M_KANJI` continue edge is not emitted (segment.rs:319);
- the `M_KANJI` state is not reachable by a switch edge either (segment.rs:288),
  and its start state is `INF` (segment.rs:271) — so no Kanji segment can appear
  even as a dominated end state.

`Options::mode = Some(Mode::Kanji)` is unchanged and still works for callers who
genuinely hold Shift-JIS: that path never went through the DP.

Both "harmless" paragraphs are gone. segment.rs:8–22 and lib.rs:23–39 now state
plainly that Kanji mode carries Shift-JIS *character values*, that a
non-Shift-JIS payload in a Kanji segment is mojibake on any reader that is not
this workspace's own byte-transparent decoder, and that byte values cannot
distinguish the two — which is why it is a caller assertion, not an inference.
`Options::assume_shift_jis`'s own doc comment repeats the warning at the point of
use.

**Regression tests added** (`tests/segmentation.rs`):

- `utf8_cjk_never_produces_kanji_segments_by_default` — four UTF-8 CJK payloads
  including both of Richard's reproducers (`"世界"` × 8 and the 93-byte sentence)
  × 4 EC levels. The assertion is exact rather than statistical: with default
  options the encoded matrix must be **bit-identical** to the same payload forced
  into `Mode::Byte`. Any Kanji segment changes the bit stream and therefore the
  matrix, so this cannot pass with one present. Also asserts `decoded.text`
  round-trips to the original `&str`.
- `opt_in_flag_is_what_switches_kanji_on` — proves the test above is not vacuous:
  for the same payload the flag *does* change the matrix (`assert_ne!`), and the
  saving being declined is real (207 bits with Kanji vs 252 without, at v1).
- `dp_never_loses_to_single_mode` now runs both flag states and asserts directly
  that no `Mode::Kanji` segment is ever produced when the flag is off.
- `kanji_pairs_segment_as_kanji_only_when_asserted` (was
  `kanji_pairs_segment_as_kanji`) — the ISO example bytes give a Kanji segment at
  38 bits opted in, and a byte segment at 44 bits by default.
- `kanji_beats_byte_mode_on_capacity` now also asserts the 400-byte Shift-JIS
  payload is `DataTooLarge` at v12-L with default options, and fits only with the
  flag.

Existing Kanji round-trips (`shift_jis_payload_roundtrips_through_kanji_segments`)
were updated to set the flag, which is the payload contract they were always
testing.

## m1 — DP memory figure — **FIXED**

Note 4 now records 20 B per byte offset → **~58 KB** for a v40 payload, and states
that packing the cost array to `u16` is unavailable (worst case 141,744 sixths).

## m2 — brute-force optimality test — **ADDED**

`dp_matches_brute_force_optimum` (`src/tests.rs`). Exhaustive over the alphabet
`{'0', 'A', 'a', 0x93, 0x5F, 0xE4}` — a digit, an uppercase letter, a
lowercase-only byte and three Shift-JIS bytes that pair into both ISO ranges —
for every length 1..=6, against all three version-group anchors and both flag
states. 55,986 payloads × 6 configurations.

`brute_force_bits` enumerates every prefix split × every mode that can hold it and
scores with `segments_bits`, so the reference cost is computed independently of
the DP's own sixths cost model. Two assertions per case, as Richard specified:
DP cost == true minimum, **and** `segments_bits(backtracked segments) == DP cost`
— the second is what guards the back-pointer walk. Runs in ~3.3 s.

## m3 — `Options` and `#[non_exhaustive]` — **ARCH RULING RECORDED**

Arch ruled: `Options` stays **not** `#[non_exhaustive]` pre-1.0; revisit at
crates.io publish. Struct literals remain legal for now, and the cost is that a
future field addition again touches every downstream literal — this round added
`assume_shift_jis:` to 24 of them. Recorded here as a decision rather than a side
effect, per Richard's request.

## m4 — `unwrap_or(0)` fallbacks — **FIXED**

All four sites now assert the invariant and fall back to a value that degrades
safely rather than emitting wrong bits:

| Site | `debug_assert!` | Release fallback |
|---|---|---|
| `optimal_segments` head cost (segment.rs:262) | every mode in `MODES` has a char-count indicator | widest indicator (16) — over-prices a segment, never under-prices |
| `write_char_count` (segment.rs:388) | mode has a char-count indicator | emit no indicator rather than a wrong width |
| `write_payload` alphanumeric (segment.rs:419) | byte is in the Table 5 set | `'0'` — in range, so the group stays a legal 11-bit value |
| `write_payload` kanji (segment.rs:447) | pair is encodable Shift-JIS | value 0 (Shift-JIS `8140`) — a real character, legal 13 bits |

## m5 — dead `value > 0x1FFF` guard — **REMOVED**

Deleted (segment.rs:82–86). The `lo > 0xBF` guard is kept and its doc comment now
names it as the load-bearing one, with Richard's `8200`/`8240` collision as the
worked example. A comment in its place records why the result always fits 13 bits
(max difference `0x2A7F` from code `EBBF` → `0x1FFF`; every smaller in-range
difference → at most `0x1F7F`) and warns against widening the ranges without
re-checking.

## Round 2 gates

| Gate | Result |
|---|---|
| `cargo test -p qr-encode` | **65 passed**, 0 failed (27 lib + 15 micro + 11 roundtrip + 12 segmentation) |
| `cargo test -p qr-encode --no-default-features` | 50 passed, 0 failed |
| `cargo clippy -p qr-encode --all-targets -- -D warnings` | clean |
| `cargo clippy -p qr-encode --no-default-features --all-targets -- -D warnings` | clean |
| `cargo fmt -p qr-encode -- --check` | clean |
| `cargo check --workspace --all-targets` | clean |
| `cargo build -p qr-encode --target thumbv7em-none-eabihf` (both configs) | clean |
| `cargo build -p qr-encode --target riscv32imc-unknown-none-elf` (both configs) | clean |
| `cd esp-selftest && cargo build --release` | clean |
| `cargo test -p tests-integration` | 84 passed, 0 failed |

Net test delta from round 1: +3 (1 brute-force optimality, 2 Kanji opt-in
regressions), with 5 existing tests strengthened rather than replaced.

**Ready for Review: YES**
