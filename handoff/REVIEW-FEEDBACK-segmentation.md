# Review Feedback — KG-6 full encoder segmentation (optimal mixed-mode, Kanji, ECI)

**Reviewer:** Richard-SEG
**Builder:** Bob-SEG
**Date:** 2026-08-11
**Scope reviewed:** `crates/qr-encode/src/segment.rs`, `src/lib.rs`, the 4-line
`select_mode` guard in `src/micro.rs`, `src/tests.rs`, `tests/roundtrip.rs`,
`tests/segmentation.rs`. The M1/M3 half-codeword change in `src/micro.rs` +
`tests/micro.rs` (Bob-IV) and the `qr-detect` change were excluded by instruction.

**Verdict: KG-6 CLEAR** (round 2 — see the round-2 section at the end of this file.
Round-1 verdict was BLOCKED; the record below is kept unchanged.)

---

# Round 1 — BLOCKED

**Verdict: KG-6 BLOCKED**

Counts: **1 BLOCKER, 0 MAJOR, 5 MINOR.**

The dynamic program is correct and provably optimal — I proved it independently,
see "Verified" below. Every regression gate is green. The block is not a coding
defect; it is a behaviour the code implements exactly as documented, where the
documentation's claim that the behaviour is harmless is false outside this repo.
The fix requires a product decision, so it is also escalated to Arch.

---

## BLOCKER

### B1 — Kanji is auto-selected for byte payloads that are not Shift-JIS, which makes the symbol unreadable to any external scanner
`crates/qr-encode/src/segment.rs:296` (the `is_kanji_pair` DP edge),
reached from `crates/qr-encode/src/lib.rs:162`.
Docs asserting the behaviour is harmless: `segment.rs:8-17`, `lib.rs:24-27`,
REVIEW-REQUEST note 2.

**What is wrong.** The DP admits a Kanji edge for *any* byte pair that happens to
land in `8140`–`9FFC` / `E040`–`EBBF`. Ordinary UTF-8 text hits those ranges
constantly, and once a run is long enough to amortise the two mode-switch headers
the DP takes it. Verified against a faithful port of the shipped algorithm:

| payload | UTF-8 bytes | v1 segmentation |
|---|---|---|
| `"世界"` × 8 | 48 | **one Kanji segment, 0..48** |
| `"日本語のテキストをここに書きます。これは長い文章のテストです。"` | 93 | Byte 0..73, **Kanji 73..91**, Byte 91..93 |

(Short CJK strings stay in byte mode because the 24-bit switch cost is not
recovered — which is why `eci_26_tagged_utf8_decodes_to_text` in
`tests/segmentation.rs:179` passes without ever exercising this. It is a
length threshold, not a safe boundary.)

ISO/IEC 18004 §7.4.6 *defines* Kanji mode as carrying Shift-JIS values; a
conformant reader converts those 13-bit values back to Shift-JIS characters. So a
scanner reading the first row above reproduces the Shift-JIS characters at code
points `E4B8`, `96E7`, `958C`, … — mojibake, not `世界世界…`.

**Why the tests do not catch it.** `qr-decode` is byte-transparent by design
(`crates/qr-decode/src/segment.rs:87-96` emits the raw Shift-JIS bytes; BUILD-LOG
KG-4 records this deliberately). So every in-repo round-trip is byte-exact and
tells us nothing about external readers. There is no third-party interop harness
on the full-QR path — `zxing`/`segno` cross-checks exist only for Micro
(`tests-integration/tests/micro_golden.rs`).

**Why the caller cannot work around it.** `Options::eci = Some(26)` does not
help: ECI governs 8-bit byte data, not Kanji segments, and `resolve_plan`
(`lib.rs:172-201`) computes the ECI header entirely independently of the DP, so
Kanji segments are still emitted. The only escape is
`mode: Some(Mode::Byte)`, which switches off all mixed-mode gain — the caller
must give up the whole feature to get a readable symbol.

**Net effect.** Before this change, `encode(utf8_cjk, Options::new(L))` produced a
byte-mode symbol that readers decode correctly via their UTF-8 heuristic (or
unambiguously with ECI 26). After it, the same call silently produces a symbol
that reads as garbage. That is a customer-visible regression on the default path
for a common payload class.

**How to fix.** Make Kanji participation in the DP opt-in rather than inferred
from byte values. Concretely: add a field to `Options` (e.g. `kanji: bool`, or a
`charset`/`Shift-JIS` marker) that defaults to *off*, and only push the
`M_KANJI` edges in `optimal_segments` when it is set. Keep
`mode: Some(Mode::Kanji)` working unchanged for callers who genuinely hold
Shift-JIS — that path is already correct and well-tested. Then delete the
"harmless" wording at `segment.rs:14-16` and `lib.rs:26-27` and state plainly
that Kanji mode reinterprets the bytes as Shift-JIS for any reader that is not
this crate's own decoder.

**Escalation to Architect.** The *shape* of the opt-in is a product decision, not
a code decision: an `Options` flag, a separate `encode_shift_jis` entry point, or
a rule such as "Kanji only when the entire payload is valid Shift-JIS" (which
still misfires on long CJK UTF-8 and which I do not recommend). Arch also needs
to rule on whether third-party scanner interop is in scope for the full-QR encode
path — KG-12 established that it is for Micro. If Arch rules interop out of
scope, B1 downgrades to a documentation fix; until that ruling, it blocks.

---

## MINOR

### m1 — DP memory footprint in the request note is understated ~2.4x
`segment.rs:256-257`. `cost` is `Vec<[u32; 4]>` (16 B/offset) and `back` is
`Vec<[u8; 4]>` (4 B/offset), so peak is 20 B per byte offset — **~58 KB** for a
2953-byte v40 payload, not the "~24 KB" in REVIEW-REQUEST note 4. Also, the
suggested halving by packing the cost array is not available: the worst-case
cost is 2953 × 48 = 141,744 sixths, which does not fit `u16`. Correct the figure
in the handoff note / BUILD-LOG. 58 KB transient on the ESP32 is survivable but
should be recorded honestly.

### m2 — `dp_never_loses_to_single_mode` proves non-regression, not optimality
`src/tests.rs`. The property asserted is `DP <= single-mode`, which a merely
*sane* segmenter also satisfies. 1800 assertions (600 payloads × 3 groups,
lengths 1–90) is meaningful coverage of the guard it does state. I closed the gap
out-of-band (see "Verified" V2) but that proof lives in this transcript, not in
the repo. Add a small brute-force optimality test — exhaustive segmentations for
lengths ≤ 8 over a 5–6 symbol alphabet, all three anchors — asserting the DP cost
equals the true minimum *and* that `segments_bits` of the backtracked segments
equals that same cost. It runs in well under a second.

### m3 — `Options` is still not `#[non_exhaustive]`
`lib.rs:103-121`. Bob's rationale (it would break the literals in the separate
`tests/roundtrip.rs` and `tests/segmentation.rs` crates) is accurate, but the
consequence is that the *next* field addition breaks every downstream literal
again — this change already forced 24 mechanical `eci:` edits. **Escalate to
Arch**: this is an API-stability call to make before 1.0, not a code decision.
If literals are to stay legal, that should be a recorded decision rather than a
side effect.

### m4 — `unwrap_or(0)` fallbacks silently paper over invariant violations
`segment.rs:346` (`char_count_bits`), `:382`, `:387` (`alnum_value`), `:397`
(`kanji_value`). All four are currently unreachable — `mode_supports` and
`segments_bits` gate every path that reaches them — but if an invariant ever
breaks, these emit *wrong bits* rather than failing. Make the invariant explicit
with `debug_assert!` at each site.

### m5 — `value > 0x1FFF` guard in `kanji_value` is dead code
`segment.rs:76-78`. The maximum reachable value is exactly `0x1FFF`: code
`0xEBBF` → diff `0x2A7F` → 42 × 0xC0 + 0x7F = 8191, and every smaller diff in
range 2 yields at most 41 × 0xC0 + 0xBF = 8063. Harmless defence in depth, but the
doc comment at `:60-61` implies it rejects real inputs. Add a one-line comment
saying it is unreachable given the two range checks and the `lo > 0xBF` guard, so
nobody later "fixes" the ranges to make it fire.

---

## Verified (this is what passed, and how)

### V1 — Fractional cost encoding is exact, including the tail
`ceil_bits` (`segment.rs:220`) rounds the running sixths total up to a whole bit,
and it is applied at **both** places the tail can occur: on every switch edge
(`:269`) and in the final-state selection (`:304-313`). Header widths are whole
bits, so `head*6 ≡ 0 (mod 6)` and `ceil(header + fractional) == header +
ceil(fractional)` — the rounding lands on the right quantity, which is the exact
failure mode the review brief flagged.

Machine-checked identities: `ceil(20L/6) == 10⌊L/3⌋ + {0,4,7}` and
`ceil(33L/6) == 11⌊L/2⌋ + 6(L mod 2)`, both agreeing with `payload_bits`.
Hand case from the brief: **4 digits → 80 sixths → 14 bits**, not 13. Also
checked L = 1,2,3,5,7 numeric (4,7,10,17,24) and L = 1..5 alnum (6,11,17,22,28).

Keeping the *raw* sixths minimum at intermediate states is safe because
`ceil_bits` is monotone non-decreasing, so a lower raw arrival can never produce a
higher rounded total.

### V2 — The DP is genuinely optimal, and the backtrack agrees with the cost
I ported `optimal_segments` line-for-line (including the snapshot-based switch
relaxation, the `(ceil, raw)` end-state tie-break and the back-pointer walk) and
compared it against an exponential brute force over every valid segmentation.

**1,016,769 payloads, 0 failures** — exhaustive over `{'0','A','a',0x93,0x5F,0xE4}`
for lengths 0–7, plus 9,000 random payloads to length 14 over an 18-symbol
alphabet including both Shift-JIS range endpoints, each run against all three
version-group anchors (v1 / v10 / v27). Two properties asserted per case:
DP cost == true minimum, and `segments_bits(backtracked segments) == DP cost`.

The second property is the one worth calling out. The switch loop overwrites
`back[i][k]` in place, so a back-pointer naming mode `k` could in principle lead
into a *chained* switch and emit a segmentation cheaper or costlier than the cost
the DP reported. It cannot: if `cost[i][k]` had been switch-overwritten from some
`l`, then `ceil(before[l]) + head[k] < before[k]`, which with `head[k] > 0` makes
`l` a strictly better switch source for `j` than `k` — so the argmin would have
named `l`, not `k`. Bob's "two switches at the same offset cannot chain" comment
(`:261-262`) is load-bearing and correct. The brute force confirms it empirically.

Zero-length segments are dropped by the `at < end` guards (`:324`, `:330`); per
the above they are unreachable anyway.

### V3 — Kanji §7.4.6 arithmetic, injectivity, and the decoder inverse
ISO example values hand-verified: `935F − 8140 = 121F`, `0x12 × 0xC0 + 0x1F =
0x0D9F` ✓; `E4AA − C140 = 236A`, `0x23 × 0xC0 + 0x6A = 0x1AAA` ✓.

Injectivity holds because the two ranges map to disjoint difference spaces:
range 1 reaches at most diff `0x1EBC` (from code `9FFC`), range 2 starts at diff
`0x1F00` (from code `E040`). `qr-decode`'s split at `sjis < 0x1F00`
(`crates/qr-decode/src/segment.rs:90-91`) sits exactly on that boundary, and
`v/0xC0`, `v%0xC0` recover `diff>>8` and `diff&0xFF` precisely because the
`lo > 0xBF` guard holds. Endpoint checked both directions: `EBBF ↔ 0x1FFF`.

The `lo > 0xBF` guard is essential, not cosmetic: without it code `0x8200`
(diff `0x00C0`) and code `0x8240` (diff `0x0100`) would both map to value `0xC0`.
It only rejects byte pairs that are not valid Shift-JIS to begin with.

**Alignment / straddling.** The brief's concern does not arise. Kanji states are
entered by a switch edge at *any* byte offset and advance by 2 via
`is_kanji_pair`, so pair alignment is defined relative to the segment start, and
a pair that would straddle a boundary simply is not an edge in the graph. There
is no sync to lose. `mode_supports(Mode::Kanji, ..)` correctly requires even
length with every pair encodable, so forced Kanji on non-SJIS input is a clean
`ModeMismatch` (`tests/segmentation.rs:156-174`).

### V4 — ECI Table 4 bit patterns are exact
`write_eci` (`segment.rs:188-197`) emits `0111` then `0bbbbbbb` /
`10bbbbbb bbbbbbbb` / `110bbbbb bbbbbbbb bbbbbbbb`, matching §7.4.3.2. Byte
streams re-derived by hand: 26 → `71 A0` (12 bits), 128 → `78 08 00` (20 bits),
999999 (`0xF423F`) → `7C F4 23 F0` (28 bits). Boundaries 127/128/16383/16384 map
to the right widths; > 999999 → `InvalidEci`, checked before any other work in
`resolve_plan` (`lib.rs:173-176`).

Capacity interaction is correct: `eci_bits` (12/20/28) is added to the segment
bit count *before* version selection at both the forced-version and auto-version
sites (`lib.rs:181`, `:193`), so the header competes for capacity rather than
overflowing. `eci_header_is_counted_against_capacity`
(`tests/segmentation.rs:229`) proves the spill on an exact-fit payload.

ECI on Micro is structurally unreachable, not runtime-rejected: `MicroOptions`
has no `eci` field (zero occurrences of `eci` in `micro.rs`). Correct call — a
type-level exclusion beats a runtime check.

### V5 — Version-group scan still yields the smallest fitting version
Segment bit counts are constant within a Table 3 group, so running the DP at the
group anchor and scanning versions inside the group cannot skip a smaller fit
(`lib.rs:189-200`), and `version_auto_select_is_still_the_smallest_fit` proves
every version below the chosen one genuinely fails.

I separately checked the character-count-indicator overflow path, which this
design could have exposed: for all twelve (mode, group) combinations the maximum
segment length is bounded by that group's largest version's data capacity, which
is strictly below the count indicator's maximum (tightest case is byte mode in
group 1: cap 230 chars against an 8-bit indicator's 255). No truncation path
exists in `write_char_count`.

### V6 — Micro guard is a true behaviour freeze
`git show HEAD:crates/qr-encode/src/segment.rs:60` confirms the old
`mode_supports` returned `Mode::Kanji | Mode::Eci => false`, so micro's
`select_mode` never reached Kanji. The new explicit `mode != MicroMode::Kanji`
reproduces that exactly. No behaviour change from this work. (Bob-IV's
half-codeword change in the same file was excluded from this review.)

### V7 — API and existing assertions
24 `eci:` additions across `src/tests.rs`, `tests/roundtrip.rs` and
`tests/segmentation.rs`. `tests/roundtrip.rs` diff is purely mechanical — seven
one-line insertions, no assertion touched. Spot-checked
`forced_mode_incompatible_errors_not_panics` (`:204-217`, still asserts Kanji
rejects `\xff\xfe` and that `Mode::Eci` errors) and `exact_capacity_fits_and_roundtrips`
(`:164`, unchanged). The two `build_data_codewords` call sites in `src/tests.rs`
still assert against the same `ANNEX_I_DATA` / reference-EC constants — signature
changed, expectation did not. Nothing weakened.

### V8 — Gates, all re-run by me on the working tree

| Gate | Result |
|---|---|
| `cargo test -p qr-encode` | 62 passed (26 lib + 15 micro + 11 roundtrip + 10 segmentation), 0 failed |
| `cargo test -p qr-encode --no-default-features` | 47 passed, 0 failed |
| `cargo clippy -p qr-encode --all-targets -- -D warnings` | clean |
| `cargo clippy -p qr-encode --all-targets --no-default-features -- -D warnings` | clean |
| `cargo fmt -p qr-encode -- --check` | clean |
| `cargo check --workspace --all-targets` | clean |
| `cargo build -p qr-encode --target thumbv7em-none-eabihf` (both feature configs) | clean |
| `cargo build -p qr-encode --target riscv32imc-unknown-none-elf` | clean |
| `cd esp-selftest && cargo build --release` | clean |
| `cargo test -p tests-integration` | 84 passed, 0 failed |

`tests-integration` reports 84, not the 81 in the request; the delta is other
Builders' concurrent additions, not this work.

---

## Summary

The engineering here is good. The cost model is exact where a fractional-cost
segmenter usually is not, the tail rounding is applied in both places it has to
be, the back-pointer scheme is subtle and survives a million-case brute force,
the Kanji arithmetic and the ECI designator widths match the standard byte for
byte, and every gate is green. Nothing in the DP, the ECI path or the forced-mode
path needs to change.

What blocks it is one default: the encoder decides "this is Shift-JIS" from byte
values alone, on a `&[u8]` API that cannot know. For this repo's own decoder that
is invisible. For anyone else's it turns working CJK QR codes into garbage, and
the caller has no way to opt out without giving up mixed-mode entirely. Put Kanji
behind an explicit opt-in, fix the two doc paragraphs that call the behaviour
harmless, and KG-6 clears. The five MINORs are all fixable inline.

**Ready for Builder: NO** — B1 needs an Arch ruling on the opt-in shape first.

---
---

# Round 2 — CLEAR

**Date:** 2026-08-11
**Verdict: KG-6 CLEAR**
**Ready for Builder: YES**

Counts: **0 BLOCKER, 0 MAJOR, 0 MINOR outstanding.** All six round-1 items
closed. Two new observations, both non-blocking, recorded at the end.

Bob fixed what I found, and fixed it in the right place — the flag gates the
*graph*, not the output, so there is no filtering step to get wrong later. The
brute-force test he added is scored independently of the DP's own cost model,
which is the property that makes it worth having. I re-verified every claim
rather than taking the request's table on trust.

## B1 — Kanji opt-in — **VERIFIED CLOSED**

### Three-way unreachability holds

`Options.assume_shift_jis: bool` (lib.rs:145, default `false` at lib.rs:158) is
threaded through `segments_for` (lib.rs:192-195) into
`optimal_segments(data, version, allow_kanji)` (segment.rs:259). With the flag
off, `cost[i][M_KANJI]` is `INF` for every `i`, by three independent cuts:

| Cut | Site | Effect |
|---|---|---|
| Start state | segment.rs:279-281 | `cost[0][M_KANJI] = INF` after `cost[0] = head` |
| Switch edges in | segment.rs:288-293 | `j == M_KANJI` is `continue`d before any relaxation |
| Continue edges | segment.rs:325 | `allow_kanji &&` guards the `i → i+2` relax |

Switch edges *out* of Kanji need no separate cut — they are skipped by the
existing `from == INF` test (segment.rs:295). Given that, the state is
unreachable everywhere it could still surface: the end-state selection skips
`INF` (segment.rs:335), and the back-pointer walk can only name a mode `k` that
was finite in the `before` snapshot, so `mode` can never become `M_KANJI`
during backtracking. There is no path — dominated, zero-length, or otherwise —
by which a `Mode::Kanji` segment reaches the output. Claim verified as stated.

`head[M_KANJI]` is still computed and left unused; harmless.

`Options::mode = Some(Mode::Kanji)` is untouched and never went through the DP,
so the genuine Shift-JIS caller is unaffected — confirmed still passing with
`assume_shift_jis: false` (`forced_kanji_mode_roundtrips_and_rejects_non_sjis`,
tests/segmentation.rs:232-251).

### My own round-1 reproducers, re-run against round-2 code

I did not rely on Bob's test payloads. I built a throwaway crate outside the repo
with path deps on `qr-encode`/`qr-decode`/`qr-core` and ran the exact payloads
from my round-1 finding through the public API. **0 failures**, covering:

- `"世界"` × 8 (48 bytes) and the 93-byte Japanese sentence — the two payloads I
  demonstrated produced Kanji segments in round 1 — plus `"Grüße, 世界 — 123"`,
  each at all four EC levels. With default options the encoded matrix is
  **bit-identical** to the same payload forced into `Mode::Byte`. A Kanji segment
  changes the bit stream and therefore the matrix, so this is an exact test, not
  a statistical one.
- Byte and text round-trip through `qr_decode` for every case
  (`decoded.text == Some(original)`).
- **Non-vacuity on my payloads, not just Bob's.** For both 48-byte and 93-byte
  cases, `assume_shift_jis: true` produces a *different* matrix (`assert_ne`) that
  still round-trips byte-exact. So the default really is declining an available
  saving rather than the flag being inert on these inputs. (Bob's own
  `opt_in_flag_is_what_switches_kanji_on` proves this for a 30-byte payload and
  quantifies the declined saving at 207 vs 252 bits at v1; my check extends it to
  the longer reproducers, where the round-1 damage actually occurred.)
- **My round-1 "ECI does not protect Kanji segments" concern re-tested and now
  moot**: with `eci: Some(26)` *and* default options, output is still
  bit-identical to forced byte mode for all three payloads. The interaction I
  flagged cannot arise because Kanji never fires by default.
- Forced `Mode::Kanji` on the ISO example bytes still round-trips with the flag
  off.

### Documentation

Both "harmless" paragraphs are gone. segment.rs:9-23 and lib.rs (module docs)
now state that Kanji mode carries Shift-JIS *character values* rather than bytes,
that non-Shift-JIS bytes in a Kanji segment are mojibake on any reader that is
not this workspace's byte-transparent decoder, and that a `&[u8]` cannot
distinguish the two — which is precisely why it is a caller assertion. That is
an accurate description of the hazard. `optimal_segments`' own doc
(segment.rs:242-245) repeats the warning at the parameter.

## m1 — DP memory figure — **VERIFIED FIXED**
Request note 4 now reads 20 B per byte offset → ~58 KB for a v40 payload, and
records that packing to `u16` is unavailable (141,744 sixths worst case). Matches
my measurement.

## m2 — brute-force optimality test — **VERIFIED ADDED, AND GENUINELY INDEPENDENT**
`dp_matches_brute_force_optimum` (src/tests.rs:579). The point I care about:
`brute_force_bits` (src/tests.rs:543) enumerates every prefix split × every mode
that `mode_supports` accepts and scores each candidate with
`segment::segments_bits` — i.e. the ISO `4 + cc + payload_bits` formula, **not**
the DP's sixths model. A shared-model reference would have been circular and
worthless; this one is a real cross-check.

Scale: alphabet `{'0','A','a',0x93,0x5F,0xE4}`, lengths 1..=6 → 55,986 payloads ×
3 anchors × 2 flag states = **335,916 DP/brute comparisons**, and it exercises the
flag-off unreachability independently by dropping Kanji from the reference mode
list. Both assertions I asked for are present: DP cost == true optimum, *and*
`assert_well_formed` on the backtracked segments (with
`segments_bits(segs) == dp` established by the first assertion, since `dp` is
computed from the backtracked segments). Passes; the lib suite runs in 6.6 s.

`dp_never_loses_to_single_mode` (src/tests.rs:458-464) now also runs both flag
states and asserts directly that no `Mode::Kanji` segment appears when the flag
is off.

## m3 — `Options` / `#[non_exhaustive]` — **ARCH RULING ACCEPTED**
Stays not `#[non_exhaustive]` pre-1.0, revisit at publish. Recorded as a decision
in the request rather than left implicit, which is what I asked for. This round
added `assume_shift_jis:` to 24 literals, which is the cost being accepted
knowingly. No objection.

## m4 — `unwrap_or(0)` fallbacks — **VERIFIED FIXED, ALL FOUR DEGRADE SAFELY**
Each site now `debug_assert!`s the invariant and falls back to a value that
cannot emit an out-of-range bit group:

| Site | Fallback | Why it is safe |
|---|---|---|
| head cost, segment.rs:267-273 | widest indicator (16) | over-prices a segment; affects the *choice*, never the bit stream |
| `write_char_count`, segment.rs:380-384 | 0 bits | omits the indicator rather than emitting a wrong-width one |
| `write_payload` alnum, segment.rs:422-426 | value 0 (`'0'`) | in the Table 5 set, so `v*45+v` stays a legal 11-bit group |
| `write_payload` kanji, segment.rs:447-455 | value 0 (SJIS `8140`) | a real character, legal 13-bit group |

Confirmed: no path now produces a bit group outside its field width. Correct
resolution — the release behaviour degrades the *content* rather than corrupting
the *structure*.

## m5 — dead `value > 0x1FFF` guard — **VERIFIED REMOVED, `lo > 0xBF` RETAINED**
segment.rs:79-87. The dead guard is gone; the load-bearing `lo > 0xBF` guard is
kept and its doc comment (segment.rs:66-69) now names it as the injectivity
guarantee with the `8200`/`8240` → `0xC0` collision as the worked example. The
replacement comment records why the result always fits 13 bits (max difference
`0x2A7F` from code `EBBF` → `0x1FFF`; every smaller in-range difference → at most
`0x1F7F`) and warns against widening the ranges without re-checking. Both
statements are arithmetically correct — I re-derived them.

## Round-2 gates — all re-run by me

| Gate | Result |
|---|---|
| `cargo test -p qr-encode` | **65 passed**, 0 failed (27 lib + 15 micro + 11 roundtrip + 12 segmentation) |
| `cargo test -p qr-encode --no-default-features` | **50 passed**, 0 failed |
| `cargo clippy -p qr-encode --all-targets -- -D warnings` | clean |
| `cargo clippy -p qr-encode --no-default-features --all-targets -- -D warnings` | clean |
| `cargo fmt -p qr-encode -- --check` | clean |
| `cargo test -p tests-integration` | **84 passed**, 0 failed |
| Richard's external reproducer crate (round-1 payloads, public API) | 0 failures |

`tests-integration` builds and passes on the shared tree — the in-flight
`qr-detect` work does not touch `qr-encode`, so no detached worktree was needed.

## Non-blocking observations

1. **The default-path bit stream still changes for numeric/alphanumeric mixing**
   — that is the KG-6 feature working, not a defect. Unlike the Kanji case it is
   charset-neutral: numeric and alphanumeric modes carry no encoding assumption,
   so no reader can misinterpret them. Request note 2 now describes this
   accurately. No action.
2. **`utf8_cjk_never_produces_kanji_segments_by_default` is proven non-vacuous
   only for the payloads where the flag demonstrably flips the output.** Bob's
   `opt_in_flag_is_what_switches_kanji_on` covers one 30-byte payload; I extended
   the check to the 48- and 93-byte reproducers out-of-band and both flip. The
   two shorter cases in that test (`"こんにちは、世界。"`, and the 24-byte
   `"世界"` × 4) may be below the switch-cost threshold and so pass trivially.
   Harmless — the test's job is to catch a regression on the long payloads, and
   it does. If Bob wants it airtight, asserting `assert_ne!` under the flag for
   each case in the same loop would close it. Not required.

## Cleared

Optimal mixed-mode segmentation, Kanji mode and the ECI header for
`crates/qr-encode`. The dynamic program is provably optimal against an
independent reference (335,916 in-repo comparisons, plus 1,016,769 in my own
round-1 external port), the fractional-cost tail rounding is exact at both the
switch and the terminal state, the back-pointer walk is consistent with the cost
array, Kanji §7.4.6 arithmetic and its decoder inverse are exact to the `EBBF` /
`0x1FFF` endpoint, ECI Table 4 designators are byte-exact across all three widths
and are charged against capacity before version selection, and Kanji — the one
mode whose meaning depends on an encoding the crate cannot observe — is now
opt-in with the default proven byte-identical to the pre-KG-6 behaviour.

**KG-6 CLEAR.**
