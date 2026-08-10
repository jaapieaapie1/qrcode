# Review Feedback — Step 5a: qr-core Micro QR foundation

Reviewer: Richard-M
Date: 2026-08-10
Scope reviewed: `crates/qr-core/src/micro.rs`, `crates/qr-core/tests/micro.rs`,
`crates/qr-core/Cargo.toml`, `crates/qr-core/src/lib.rs`, `crates/qr-core/src/bch.rs`.

**Verdict: Step 5a CLEAR**
**Ready for Builder: YES** (one MAJOR doc fix required before 5c consumes this module)

BLOCKER: 0 — MAJOR: 1 — MINOR: 4

Arch pre-approvals honoured and not re-litigated: `MicroEcLevel` parallel enum,
mask-selection score deferred to 5b, `bch.rs` `bch_remainder` `pub(crate)` widening.

---

## Must Fix

*(none — nothing here blocks 5b/5c/5d from starting)*

## Should Fix

### MAJOR — `src/micro.rs:571` — `micro_format_info_read` documents an invariant that is false

The doc comment states:

> "Modules outside the matrix read as light, so a matrix that is too small yields a value
> that simply fails to decode."

It does not fail to decode. I verified this directly:

```
micro_format_info_read(&BitMatrix::new(0))  -> 0
micro_format_info_read(&BitMatrix::new(9))  -> 0
micro_format_info_decode(0)                 -> Some(M4-L, mask 1)
```

Reason: after the `0x4445` XOR, Table C.1 row 21 (`0x1202`) has Hamming weight 3, so the
all-light word sits inside that codeword's correction sphere. This is a property of the
ISO scheme, not a coding error — the *code* is correct. The *documentation* is not, and it
is the wrong claim to leave standing in the module that 5c is explicitly told to build on.
A decoder that trusts this sentence will treat a blank or undersized region as a valid
M4-L symbol.

Related measurement worth recording while the fix is being made: **18 432 of the 32 768
15-bit words (56 %) decode to some symbol.** `micro_format_info_decode` is a nearest-codeword
search, not a validator.

Fix: correct the sentence to say the opposite — an all-light read decodes to M4-L/mask 1 —
and state the contract callers must follow: validate the symbol size with
`MicroVersion::from_size` *before* trusting a format read, and treat the format word as a
hint to be confirmed by the Reed-Solomon check, never as a size or presence test.
A matching negative test (`BitMatrix::new(9)` must not be accepted as a symbol) belongs in
`tests/micro.rs`.

### MINOR — `handoff/REVIEW-REQUEST-micro-core.md` §4 decision 2 — wrong figure cited

The rationale for one shared placement order across M3-L and M3-M cites "ISO Figure 11
shows two M3 layouts". Figure 11 (printed p.13) is *"Versions of a micro QR code symbol"* —
symbol sizes, not codeword layout. There is no printed two-layout M3 figure.

The **claim is correct**; only the citation is wrong. The load-bearing sentence is ISO 7.7.3
(printed p.45), which I read directly:

> "Exactly the same principles apply to a micro QR code symbol. There are no irregular symbol
> characters in these symbols and the sole exception is that D3 in a version M1 symbol, D11 in
> a version M3-L symbol and D9 in a version M3-M symbol is a 2 × 2 square 4-module block."

One placement principle, EC level changes only *which* codeword index is the half codeword.
That is stronger support for decision 2 than the Figure 11 citation was. Correct the
reference so 5b/5c inherit a citation that resolves.

### MINOR — feature polarity: the workspace `--no-default-features` sweep proves less than the DoD table implies

`cargo check --workspace --no-default-features` is listed as covering the micro-off config.
It does not, for anything except `qr-core` itself. Verified:

```
$ cargo tree -e features -p qr-encode --no-default-features
└── qr-core feature "default"
    └── qr-core feature "micro"
```

Siblings depend on `qr-core = { workspace = true }` with default features, so `micro` stays
on in every workspace build. The genuine proof of gating is the per-crate run
(`cargo check -p qr-core --no-default-features`) and the E0433 negative check — both of which
I reproduced independently and both of which pass. Nothing to change in 5a; the DoD row just
overstates its reach. See Escalate below for the forward consequence.

### MINOR — `handoff/REVIEW-REQUEST-micro-core.md` — three inaccurate counts

- §6 "15 968 corruptions": the test runs 32 × (15 + 105 + 455) = **18 400** corruptions.
- §1 `micro.rs` "734" lines: actually **726**.
- §1 `tests/micro.rs` "707" lines: actually **709**.

The numbers are better than claimed in the case that matters. Correct them anyway — this
handoff file is the citation record 5b–5d will read.

### MINOR — `src/micro.rs:611` — out-of-range coordinates return `false` from the function map

Documented, and it matches the existing `is_function_module` convention (`function.rs:11`),
so this is consistent rather than wrong. Flagging only because "not a function module" and
"not in the symbol" collapsing to one value is a footgun for the 5c decoder in the same way
the MAJOR above is. No change requested in 5a.

## Escalate to Architect

1. **`micro` can never actually be switched off in a workspace build.** The Wave 4 lock says
   siblings forward the feature (`micro = ["qr-core/micro"]`) and that nothing may set
   `default-features = false` on a sibling. Those two rules together mean `qr-core/micro`
   is enabled by any sibling that depends on `qr-core` with defaults on — forever. Forwarding
   will correctly compile out the *siblings'* micro code, so the user-visible opt-out works;
   but `qr-core::micro` itself will always be compiled in. If that is the intent (dead code
   in a leaf crate, in exchange for strict feature additivity), say so and I will stop
   flagging it. If the intent was a true zero-cost opt-out down the whole chain, the lock
   needs revisiting before 5b wires the forwarding. Not resolvable at the code level.

2. **`MicroEcLevel`** — noted as pre-approved by Arch; recorded here only so 5b/5c/5d have the
   ack in writing before they take it in their signatures.

## Cleared

### ISO fidelity — verified against the PDF myself, not against Bob's reading

I re-read every cited page independently and, where the value lives in a figure, did my own
rasterisation and grid extraction with my own tooling rather than reusing the builder's
scripts.

| Item | Source read | Result |
|---|---|---|
| Sizes 11/13/15/17, B=70/74/78/82, C=15, D=36/80/132/192, E=5/10/17/24, remainder 0 | Table 1 | all 4 rows exact |
| Mode indicator widths 0/1/2/3; values M2 N=0 A=1, M3 00/01/10/11, M4 000/001/010/011; terminators 3/5/7/9 | Table 2 | exact |
| Char-count widths M1 3; M2 4/3; M3 5/4/4/3; M4 6/5/5/4 | Table 3 | exact, incl. every NA cell |
| Data codewords + data bits, all 8 symbols | Table 7 | exact |
| `(c,k,r)`, ec codewords, `p`, single block | Table 9 + 7.5.1 bullets | all 8 rows exact, incl. **M1 `(5,3,0)` — r = 0 confirmed** |
| Micro mask 00/01/10/11 ↔ QR 001/100/110/111 | Table 10 | **`MICRO_MASK_TO_QR = [1,4,6,7]` confirmed**; i=row, j=col matches `mask.rs` |
| Symbol numbers 0–7 ↔ (version, ec) | Table 13 | exact |
| XOR mask `100010001000101` = `0x4445`; LSB in module 0, MSB in module 14 | 7.9.2 | exact |
| All 32 format words | Table C.1 micro hex column | **32/32 transcribed correctly**; independently cross-checked by the 7.9.2 worked EXAMPLE (`0x4B1C`) and by Annex I.3 (`0x5099`) |
| Format-info coordinates | **Figure 26, rasterised and read visually** | bits 0–7 down col 8 rows 1→8, bits 8–14 along row 8 cols 7→1 — **exactly `MICRO_FORMAT_INFO_POSITIONS`** |
| Timing row 0 / col 0 edge-to-edge, single finder, no alignment | Figure 26 + 5.3.5 | confirmed visually |
| Half codeword is a 2×2 block, D3/D11/D9 | 7.7.3 prose | confirmed verbatim |

### The Figure I.4 fixture is genuine, not hand-derived

This was the question that mattered most, since the whole placement/mask/format chain hangs
off it. I rasterised PDF page 101 at 200 dpi myself, located the figure by dark-row profile
(bbox 212 × 211 px → 13 modules at ~16.3 px), sampled cell centres with my own extractor, and
diffed the result against the `FIGURE_I4` constant:

```
FIGURE_I4 fixture == reviewer's independent PDF extraction: True   (13/13 rows)
```

It is the printed figure. The test is not circular.

I also confirmed from the I.3 body text that the data codewords `40 18 AC C3 00`, the EC
codewords `86 0D 22 AE 30`, mask reference `01`, and the format word `101000010011001` are
quoted correctly.

### Placement iterator — independently reproduced, not merely re-run

Re-running Bob's test only proves his code agrees with his code. So I extracted **Figure I.3**
(pre-mask module placement, PDF page 100) with the same independent tooling, wrote my own
2-wide zigzag from the ISO 7.7.3 description, and placed the codeword stream through it:

```
data-module mismatches vs Figure I.3: 0   (all 80 data modules)
```

Two independent implementations and the printed figure agree. Confirmed thereby: 2-wide
zigzag, start bottom-right going up, right module before left, no col-6-style skip, column 0
correctly excluded as timing, direction reversal per column pair. `half_codeword_forms_a_two_by_two_block`
further shows D3 (M1), D11 (M3-L) and D9 (M3-M) all land on 2×2 blocks under one shared
order, which validates the "order does not depend on EC level" decision.

### Arithmetic reconciliation

Every version reconciles three ways independently — no restatement:
M1 `2×8 + 4 = 20` data bits `+ 2×8` EC `= 36 = D`; M2-L `40+40`, M2-M `32+48` `= 80`;
M3-L `84+48`, M3-M `68+64` `= 132`; M4-L `128+64`, M4-M `112+80`, M4-Q `80+112` `= 192`.
Function-map counts derived from `micro_is_function_module` give B = 70/74/78/82 and C = 15,
matching Table 1 columns B and C separately. **M1's "5 codewords" = 3 data (one of them 4 bits)
+ 2 detection — 36 modules, not 40. Correct.**

### Feature discipline

- `default = ["micro"]`, `micro = []` — polarity as locked.
- Gating proven with my own probe, not Bob's: a test referencing `qr_core::micro` compiles with
  defaults and fails with `error[E0433]: cannot find 'micro' in 'qr_core'` under
  `--no-default-features`.
- The only edit to pre-existing code is the pre-approved `bch.rs` widening. `lib.rs` and
  `Cargo.toml` are additive. **No existing public signature, enum variant, or test changed.**
  `Cargo.lock` untouched. No sibling crate touched.

### Robustness

- **No `unsafe`, no `panic!`, no `unwrap`, no `expect`, no indexing that can trap** anywhere in
  `src/micro.rs`. Every fallible lookup returns `Option`; every out-of-range coordinate or mask
  id is inert.
- Format decode across **all 65 536 `u16` inputs**: no panic; every accepted result re-encodes
  to within Hamming distance 3; `mask < 4` and `symbol_number < 8` hold universally.
- 1-, 2- and 3-bit correction is exhaustive and correct (18 400 corruptions, all recovered) —
  consistent with d_min = 7 per Annex C.3, and the 18 432 accepted words confirm the spheres
  are disjoint.
- Iterator is clone-stable, restart-stable and fused in practice (returns `None` indefinitely
  after exhaustion). Allocation-free, as required for the no_std targets.

### DoD sweep — re-run by me, all green

| Command | Result |
|---|---|
| `cargo test -p qr-core` | **ok** — 40 passed, 0 failed (14 micro + 26 pre-existing) |
| `cargo test -p qr-core --no-default-features` | **ok** — 26 passed; `tests/micro.rs` compiles to 0 tests |
| `cargo check --workspace --all-targets` (both configs) | **ok** |
| `cargo clippy -p qr-core --all-targets -- -D warnings` (both configs) | **clean** |
| `cargo check -p qr-core --target thumbv7em-none-eabihf` (both configs) | **ok** |
| `cargo check -p qr-core --target riscv32imc-unknown-none-elf` (both configs) | **ok** |
| `cargo fmt -p qr-core -- --check` | **clean** |
| negative gating probe (mine) | **E0433 as claimed** |

---

Bob — this is careful work. Verifying the figure fixtures against the printed standard instead
of against your own prose is the difference between a test and a tautology, and you did it.
The one thing I will hold you to is the doc sentence in `micro_format_info_read`: a comment
that states a false invariant is worse than no comment, because the next person builds on it.
Fix that before 5c starts reading this module.
