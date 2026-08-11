# REVIEW-REQUEST — KG-12 external interop verification (Micro QR M1/M3 half codeword)

**Builder:** Bob-IV
**Ready for Review:** YES
**VERDICT: REFUTED — and now FIXED.** Our M1/M3 half-codeword convention was wrong and
was an interoperability defect on both the encode and the decode side. Arch reversed the
ruling on the evidence in §1–§5 and authorised the fix; it is in §8, and the post-fix
re-verification in §9 is clean on every cell that previously failed.

Read §1–§5 as the investigation that produced the verdict, §8–§9 as the remedy.

---

## 1. The question

For Micro QR M1 and M3 the final *data* codeword is 4 bits, not 8. Reed-Solomon operates
on bytes, so the 4 bits must be widened to a byte before the generator polynomial runs.
Two candidate conventions:

| name | RS input byte | our code | references |
|---|---|---|---|
| **low nibble** (zero-prefixed, `0000nnnn`) | `n` (0..=15) | **ours** | — |
| **high nibble** (left-justified, `nnnn0000`) | `n << 4` | — | **segno, zxing-cpp** |

ISO/IEC 18004:2024 does not settle it. 7.4.11 (bit stream to codeword conversion) and
7.5.2 (generating the error correction codewords) both describe the 4-bit final codeword
but neither states how it is widened for RS, and there is no worked M1/M3 example
anywhere in the standard.

The precedent our implementation followed is **Annex N.1**, which reads:

> "The first codeword is 4 bits in length. All subsequent codewords are 8 bits in length.
> The first 4-bit data codeword is prefixed with 0000 to make its length 8 bits for
> generating the error correction codewords."

**That note is about QR code model 1** (AIM ITS 97-001), not Micro QR, and it is about the
**first** codeword, not the last. Read as a byte-alignment rule rather than a nibble-position
rule it is actually *consistent* with what the references do: the message bit stream is
laid out MSB-first and the gap is filled with zeros at the codeword boundary. For model 1's
*leading* partial codeword the gap is on the left (`0000nnnn`); for Micro's *trailing*
partial codeword the gap is on the right (`nnnn0000`). Transplanting N.1 literally onto the
trailing codeword inverts it.

---

## 2. Method

Three independent implementations compared, in both directions, plus a bit-exact
matrix-level discriminator.

1. **Ours → reference.** `encode_micro` → `qr_render::render_image` (scale 12, quiet zone 4)
   → PNG → `zxingcpp.read_barcodes(formats=MicroQRCode)`.
2. **Reference → ours.** `segno.make(...)` PNG → `qr_detect::detect_micro` (full pipeline,
   `decode` feature on), and segno/zxing module grids fed straight into
   `qr_decode::micro::decode_micro` to take the detector out of the loop.
3. **Bit-exact discriminator.** segno's own internals (`prepare_data`, `write_segment`,
   `write_terminator`, `write_padding_bits`, `write_pad_codewords`, `make_blocks`,
   `add_codewords`, `find_and_apply_best_mask`, `add_format_info`) were re-driven to build
   *two* matrices per case for the same payload / version / EC / mask — one with RS over
   `n << 4` ("high"), one with RS over `n` ("low") — and both were compared module-by-module
   against our encoder's matrix. The "high" replica was first validated to be 0-diff against
   real `segno.make()` output, so it is a faithful stand-in.
4. **Third opinion.** zxing-cpp's *writer* (`write_barcode(MicroQRCode, ...)`, 2-module quiet
   zone stripped) produced module grids that were compared against the same two replicas and
   fed to our decoder.

Payloads were chosen so the final 4-bit codeword is **non-zero** — with `n == 0` the two
conventions coincide (`0 << 4 == 0`) and the case proves nothing. Note that any M1 payload
of 4 digits or fewer, and any M3 byte-mode payload, lands on `n == 0`; only 5-digit M1 and
capacity-filling M3 numeric/alphanumeric payloads discriminate.

### Tool versions

| tool | version |
|---|---|
| segno | 1.6.6 (pure Python encoder) |
| zxing-cpp (python wheel) | 3.1.1 (C++ reader **and** writer) |
| Pillow | 12.3.0 |
| numpy | 2.x (image array access only) |
| Python | 3.14.3, venv at `_temp/interop-kg12/venv` |
| Rust harness | `_temp/interop-kg12/rs` (own `[workspace]`, path-deps on our crates; workspace membership untouched) |
| ISO text | `ISO_IEC_18004_2024(en).pdf` (repo copy), extracted to `_temp/interop-kg12/iso.txt` |

---

## 3. Byte-level evidence

Data bit stream and both candidate EC blocks, computed with segno's own GF(256) routines:

**M1, error-detection, payload `12345`** — 20 data bits, 2 whole codewords + 1 nibble

```
bits (20)    10100011 11011010 1101
data cws     A3 DA
final nibble D    ->  reference RS input D0   |   our RS input 0D
EC (high)    6E C7      <- segno AND zxing-cpp both emit this
EC (low)     14 60      <- ours
```

**M3-L, payload `8885393363387500474395`** — 84 data bits, 10 whole codewords + 1 nibble

```
data cws     2D BC 43 6A 82 A5 77 05 ED BA
final nibble 8    ->  reference RS input 80   |   our RS input 08
EC (high)    DD C1 EC 65 65 CA
EC (low)     CE 49 67 B5 EA 0D
```

**M3-M, payload `F8GB26F21DE`** — 68 data bits, 8 whole codewords + 1 nibble

```
data cws     6D 55 AD B0 C0 A9 41 D1
final nibble C    ->  reference RS input C0   |   our RS input 0C
EC (high)    AE CF 0A 6F 8F C9 DC 82
EC (low)     8D A8 F7 19 F0 5E 45 F0
```

Every EC codeword differs. The data region is identical between the two conventions — only
the EC region diverges, which is exactly the signature of an RS-input disagreement.

---

## 4. Results

### 4.1 Matrix-level discriminator (sharpest test)

`diff` = number of differing modules between our encoder's matrix and the replica, same
version / EC / mask. Every case run at all four masks 0–3; results were identical at every
mask, so one row per payload.

| version | EC | payload | nibble | ours vs **high** (segno/zxing) | ours vs **low** | real segno vs high replica |
|---|---|---|---|---|---|---|
| M1 | Det | `01234` | 2 | **6** | **0** | 0 |
| M1 | Det | `12345` | D | **10** | **0** | 0 |
| M1 | Det | `98765` | 1 | **6** | **0** | 0 |
| M1 | Det | `99999` | 3 | **8** | **0** | 0 |
| M1 | Det | `00001` | 1 | **6** | **0** | 0 |
| M3 | L | `8885393363387500474395` | 8 | **22** | **0** | 0 |
| M3 | L | `75513137353799075116372` | 8 | **22** | **0** | 0 |
| M3 | L | `C26J9HJB4FFI4A` | C | **18** | **0** | 0 |
| M3 | M | `003761436537056563` | 6 | **36** | **0** | 0 |
| M3 | M | `F8GB26F21DE` | C | **40** | **0** | 0 |

Our encoder is **bit-identical to the low-nibble variant** and differs from both references
in the EC region. There is no ambiguity left about which convention our code implements.

### 4.2 Ours → zxing-cpp (encode side)

| version | payload | nibble | masks | zxing-cpp reading **our** PNG | zxing-cpp reading **segno's** PNG |
|---|---|---|---|---|---|
| M1 | `01234` | 2 | 0–3 | no barcode found | `01234` ✓ |
| M1 | `12345` | D | 0–3 | no barcode found | `12345` ✓ |
| M1 | `98765` | 1 | 0–3 | **`98764` — silently wrong** | `98765` ✓ |
| M1 | `99999` | 3 | 0–3 | no barcode found | `99999` ✓ |
| M1 | `00001` | 1 | 0–3 | **`00000` — silently wrong** | `00001` ✓ |
| M3 | all five above | ≠0 | 0–3 | no barcode found (20/20) | correct payload (20/20) |

**The two silent misreads are the worst finding in this report.** M1 has two EC codewords
and zero correction capacity per ISO Table 9, but zxing-cpp runs a general RS decode over
them and, for those payloads, "corrects" the final data codeword into a self-consistent but
different value. Our symbol decodes to a wrong payload with no error raised. 8 of 20 M1
cells produced corrupted data rather than a clean failure.

### 4.3 Reference → ours (decode side)

| source | version/EC | payload | our detector | our decoder |
|---|---|---|---|---|
| segno PNG | M1 Det | `01234`,`12345`,`98765`,`99999`,`00001` | 1 detection each ✓ | `DecodeError::BlockCorrection` (5/5) |
| segno PNG | M3 L/M | the five M3 payloads above | 1 detection each ✓ | `DecodeError::BlockCorrection` (5/5) |
| segno grid → `decode_micro` direct | M1 Det | `12345` | n/a | `BlockCorrection` |
| segno grid → `decode_micro` direct | M3 L | `8885393363387500474395` | n/a | `BlockCorrection` |
| segno grid → `decode_micro` direct | M3 M | `F8GB26F21DE` | n/a | `BlockCorrection` |
| **zxing-cpp writer** grid → `decode_micro` | M1 | `12345`,`01234`,`99999` | n/a | `BlockCorrection` (3/3) |
| **zxing-cpp writer** grid → `decode_micro` | M3 | `8885393363387500474395`,`F8GB26F21DE` | n/a | `BlockCorrection` (2/2) |

Detection and sampling are fine — the symbol is found and the grid is read correctly every
time. The failure is purely the RS check, on the decode side too. Our decoder rejects
standards-conformant symbols produced by two independent encoders.

### 4.4 Third-implementation cross-check

zxing-cpp's writer, compared against the two replicas across masks:

| version/EC | payload | zxing-cpp writer grid |
|---|---|---|
| M1 Det | `12345` | **0-diff vs high replica** (mask 2); 10-diff vs low |
| M1 Det | `01234` | **0-diff vs high replica** (mask 1); 6-diff vs low |
| M1 Det | `99999` | **0-diff vs high replica** (mask 0); 8-diff vs low |
| M3 L | `8885393363387500474395` | **0-diff vs high replica** (mask 0); 22-diff vs low |
| M3 M | `F8GB26F21DE` | **0-diff vs high replica** (mask 2); 40-diff vs low |

A C++ implementation and a pure-Python implementation, with no shared lineage, emit
**byte-identical symbols** under the high-nibble convention. This is not one reference's
quirk.

### 4.5 Controls (must pass regardless — all pass)

| case | version | EC | payload | ours vs segno (modules) | zxing reads ours | ours reads segno |
|---|---|---|---|---|---|---|
| half codeword present but **zero** | M1 | Det | `1234` (n=0) | **0** | ✓ `1234` | ✓ `1234` |
| half codeword present but **zero** | M1 | Det | `007` (n=0) | **0** | ✓ `007` | ✓ `007` |
| no half codeword | M2 | L | `1234567` | **0** | ✓ | ✓ |
| no half codeword | M2 | M | `ABC12` | **0** | ✓ | ✓ |
| no half codeword | M4 | L | `HELLO WORLD` | **0** | ✓ | ✓ |
| no half codeword | M4 | M | `hello-42` | **0** | ✓ | ✓ |
| no half codeword | M4 | Q | `12345` | **0** | ✓ | ✓ |
| zxing writer → our decoder | M1 | Det | `1234` (n=0) | n/a | n/a | ✓ `1234` |
| zxing writer → our decoder | M2 | — | `ABC12` | n/a | n/a | ✓ `ABC12` |

M2 and M4 are byte-for-byte identical to segno across mode selection, terminator, padding,
pad codewords, RS, interleaving, placement, masking and format information. The M1 `n == 0`
controls are identical too. **Every other part of the Micro QR pipeline — encoder and
decoder — is externally validated as correct.** The defect is isolated to a single nibble
position in one code path.

---

## 5. Scope of the defect

Affected: **M1 (all EC — it is detection-only) and M3-L / M3-M**, and only when the final
4-bit data codeword is non-zero. In practice that means M1 payloads of exactly 5 digits, and
M3 numeric/alphanumeric payloads that fill the symbol capacity. M3 byte-mode and all short
payloads land on `n == 0` and happen to interoperate by accident.

Both `qr-encode` (`crates/qr-encode/src/micro.rs`) and `qr-decode`
(`crates/qr-decode/src/micro.rs`) shared the convention, which is why our own round-trip
tests passed — they were self-consistent. This is exactly the blind spot KG-12 was opened
to find.

---

## 6. Artefacts

All under `_temp/interop-kg12/` (gitignored; nothing installed globally, no brew used):

| path | what |
|---|---|
| `venv/` | Python 3.14 venv: segno 1.6.6, zxing-cpp 3.1.1, Pillow, numpy |
| `kg12.py` | segno-internals driver: `data_bits`, `codewords`, `ec_of`, `build_matrix(convention=high\|low)`, `ours()` |
| `rs/` | throwaway cargo project (own `[workspace]`), path-deps on qr-core/encode/decode/detect/render |
| `rs/src/main.rs` | harness: `emit`, `decode-png`, `decode-matrix` |
| `png/` | ~130 PNGs, `ours_*` / `segno_*` (pre-fix) and `fix_*` (post-fix), per version/EC/payload/mask |
| `gen_golden.py` | emits `tests-integration/tests/micro_golden.rs`'s vector table; re-run it if the vectors ever move |
| `verify.py` | the post-fix verification run of §9 |
| `iso.txt` | pdftotext extraction of the ISO PDF (clause 7.4.11 at ~line 4485, 7.5.2 at ~8052, Annex N.1 note at ~12923) |

---

## 8. The fix

Authorised by Arch after the ruling was reversed. The four significant bits written into
the symbol's data region never change — only the byte handed to Reed-Solomon moves, from
`0000nnnn` to `nnnn0000`.

### 8.1 `crates/qr-encode/src/micro.rs`

| what | change |
|---|---|
| `build_data_codewords` | Deleted the `*last >>= 4` that pushed the half codeword down into the low nibble. `BitBuffer`'s MSB-first packing already leaves those four bits in the high nibble with a zero low nibble, which *is* the left-justified byte — so the fix is the removal of a step, not the addition of one. |
| `build_module_stream` | The 4-bit push now takes the high nibble: `push_bits((codeword >> 4) as u32, 4)` instead of `push_bits(codeword as u32, 4)`. |
| doc comments | Both rewritten. They now state the left-justified convention, cite ISO 7.4.11 (width) and 7.5.2 (byte-wise RS) and say plainly that the standard settles nothing in between, name zxing-cpp 3.1.1 and segno 1.6.6 as the agreeing implementations, and point here. The Annex N.1 citation is corrected rather than dropped: it is model 1's *leading* partial codeword, and read as a byte-alignment rule it agrees with us — zeros fill the unused positions at the codeword boundary, on the left for a leading partial codeword and on the right for a trailing one. |

### 8.2 `crates/qr-decode/src/micro.rs`

| what | change |
|---|---|
| `read_block` | `block[index] \|= 1 << (7 - bit)` instead of `1 << (width - 1 - bit)`. Every codeword now fills MSB-first from bit 7 regardless of width, so a 4-bit codeword lands left-justified by construction. `width` survives only as the codeword boundary. |
| `correct_block` | The post-correction sanity check flipped: a half codeword with anything in its **low** nibble (`& 0x0F != 0`) is a mis-correction. It was checking the high nibble. |
| `data_bit_stream` | **Removed.** It existed only to shift the half codeword from the low nibble up to the top of the trailing byte before parsing. Left-justified codewords already are the contiguous `data_bits`-long stream, so the repack — and its `Vec` clone — is gone. The caller passes `codewords` straight to `parse_micro_segments`, with a comment saying why no repacking is needed. |
| doc comments | Same treatment as the encode side. |

### 8.3 Test fixtures

| file | change |
|---|---|
| `crates/qr-encode/tests/micro.rs` | `to_codewords` packs MSB-first from bit 7 (was `width - 1 - bit`), so the extracted half codeword is left-justified. `half_codeword_enters_reed_solomon_zero_prefixed` renamed to `..._left_justified`; its pins move from `data == [0xA3, 0xDA, 0x0D]` / `ec == [0x14, 0x60]` to `[0xA3, 0xDA, 0xD0]` / `[0x6E, 0xC7]`, and the negative assertion now rejects the zero-prefixed reading. The nibble-clear assertion moved from `& 0xF0` to `& 0x0F`. |
| `crates/qr-decode/tests/micro.rs` | The two fixture builders — `pad_to_codewords` and `build_symbol` — pack and unpack from bit 7 rather than `width - 1`, so hand-built fixture symbols carry the half codeword left-justified. |

**The `[0x6E, 0xC7]` pin was verified three independent ways** before it went in: segno's
GF(256) routines produced it; zxing-cpp's encoder produced a symbol containing it; and it
was derived by hand. `g(x) = (x - α⁰)(x - α¹) = x² + 3x + 2`; dividing
`A3·x⁴ + DA·x³ + D0·x²` by `g` over GF(2⁸)/0x11D:

```
A3·(x²+3x+2) = A3 x⁴ + F8 x³ + 5B x²   ->  x³: DA^F8 = 22   x²: D0^5B = 8B
22·(x²+3x+2) = 22 x³ + 66 x² + 44 x    ->  x²: 8B^66 = ED   x¹: 44
ED·(x²+3x+2) = ED x² + 2A x  + C7      ->  x¹: 44^2A = 6E   x⁰: C7
```

remainder `6E·x + C7`. The same division over `A3·x⁴ + DA·x³ + 0D·x²` gives `[0x14, 0x60]`
— the value this crate used to emit — which confirms the old pin was internally correct
and simply built on the wrong widening.

### 8.4 New: `tests-integration/tests/micro_golden.rs` (Suite 10)

The structural answer to how this survived a full round-trip suite. Seven module grids are
baked in as in-code `&'static [&'static str]` tables — no binary fixtures, no Python at
test time — every one produced by segno 1.6.6, read back by zxing-cpp 3.1.1's reader, and
for three of them byte-identical to zxing-cpp's *own* encoder output. Three tests:

| test | asserts |
|---|---|
| `decodes_every_external_vector` | our decoder reads all seven to the right payload/version/EC/mask, with `errors_corrected == 0` — an undamaged symbol must not be silently "repaired" |
| `reproduces_every_external_vector` | our encoder, with version/level/mask all forced, reproduces each grid module for module |
| `discriminating_vectors_really_discriminate` | guards the guard — reads the half codeword straight out of each discriminating grid (last four data modules, unmasked) and fails if it is `0000`, i.e. if someone has quietly swapped in a payload that no longer tells the two conventions apart |

Vectors: M1 `12345` mask 0 and `98765` mask 2 (discriminating), M3-L
`8885393363387500474395` mask 0 and M3-M `F8GB26F21DE` mask 2 (discriminating), plus
controls M1 `1234` mask 0 (zero nibble), M2-L `1234567` mask 0, M4-Q `12345` mask 1.
`98765` is deliberately the payload that previously produced a **silent misread** as
`98764`.

### 8.5 Annex N.1 citations

Corrected in the four places inside my scope: `crates/qr-encode/src/micro.rs` (×2),
`crates/qr-encode/tests/micro.rs` (×3, now one corrected note), `crates/qr-decode/src/micro.rs`
(×1). **Five stale citations remain in files I do not own** — flagged in §10 rather than
edited, since `BUILD-LOG.md` is being modified by a sibling right now:

| file | line | says |
|---|---|---|
| `handoff/SESSION-CHECKPOINT.md` | 20 | "LOW nibble zero-prefixed into RS (ISO Annex N.1 precedent)" |
| `handoff/BUILD-LOG.md` | 13 | same claim in Key Decisions |
| `handoff/BUILD-LOG.md` | 163 | the KG-12 Known Gap entry itself — now resolved |
| `handoff/REVIEW-REQUEST-micro-encode.md` | 244–252 | the original derivation |
| `handoff/REVIEW-REQUEST-micro-decode.md` | 115–119 | the original derivation |
| `handoff/REVIEW-FEEDBACK-micro-decode.md` | 244–245 | Richard's sign-off on the old convention |

---

## 9. Post-fix verification

Same three implementations, same payloads, same method as §4 — re-run against the fixed
crates. Every previously failing cell now passes and no control regressed.

| check | before | after |
|---|---|---|
| our matrix == segno matrix (M1+M3 discriminating, 4 masks each) | 0/40 | **40/40** |
| ours → zxing-cpp reader | 0/40 (12 no-read, 8 **silent misreads**) | **40/40** |
| segno → our detector + decoder | 0/40 | **40/40** |
| controls (M1 n=0, M2-L/M, M4-L/M/Q — matrix + both directions) | 7/7 | **7/7** |
| zxing-cpp's own encoder → our decoder | 2/7 | **7/7** |

The two silent misreads (`98765` → `98764`, `00001` → `00000`) are gone; both now read
back exactly. `98765` is pinned as a golden vector so it cannot regress silently again.

### 9.1 Definition of done

| gate | result |
|---|---|
| `cargo test -p qr-encode -p qr-decode -p tests-integration` | **185 passed, 0 failed** (3 of them the new Suite 10) |
| same, `--no-default-features` | **green** (qr-encode's micro suite correctly compiles to 0 tests with `micro` off) |
| `cargo clippy --all-targets`, both feature configs | **zero warnings, zero errors** |
| `cargo fmt` | clean |
| `cargo check -p qr-encode -p qr-decode --target thumbv7em-none-eabihf` / `riscv32imc-unknown-none-elf`, both configs | **4/4 green** |
| `esp-selftest` build + QEMU `virt` riscv32imc | **ALL PASS**, QEMU exit 0, `cargo fmt --check` clean |
| external re-verification | table above |

No in-flight failures from the sibling Bobs surfaced in any config — the QR paths in
qr-encode and all of qr-detect were green throughout.

Two observations, neither actioned:

1. `esp-selftest` now reports `heap peak 123640 of 196608 bytes`, where `BUILD-LOG.md:108`
   records 75 348. My change only *removes* an allocation (`data_bit_stream`'s `Vec`
   clone), so this is not from me — most likely the in-flight qr-encode segmentation work,
   or a different peak-reporting mode (the same BUILD-LOG paragraph mentions a raw
   fragmentation figure of ~122 KB). Worth a glance from whoever owns that suite; still far
   inside the 192 KiB arena.
2. `cargo fmt -p qr-encode` necessarily visits the sibling's in-flight files. I verified
   afterwards that the index versions of `src/lib.rs`, `src/segment.rs`, `src/tests.rs` and
   `tests/roundtrip.rs` were **already rustfmt-clean**, so nothing I ran reformatted Bob-SEG's
   work; the working-tree differences there are entirely his own unstaged edits.

---

## 10. For Arch

1. **Stale Annex N.1 citations in files I do not own** — the six rows in §8.5. `BUILD-LOG.md`
   is being edited by a sibling right now, so I left all of them alone. `BUILD-LOG.md:163`
   is the KG-12 Known Gap entry and can be closed.
2. **`SESSION-CHECKPOINT.md:20` is now actively wrong** and is the file the next session
   reads first. Worth correcting before anything else.
3. **Sequencing** — my changes are confined to `micro.rs` in both crates plus their test
   files and one new integration suite. No import collision with Bob-SEG's `segment.rs` /
   `bits.rs` / `lib.rs` work: `micro.rs` uses `segment::write_payload` and `BitBuffer` and
   both signatures were untouched. Nothing of mine lands in `qr-detect`.
4. **The class of bug, not the instance.** Suite 10 covers Micro QR only. Every other
   symbology path in this workspace is still validated exclusively by our own encoder
   feeding our own decoder, which is precisely the shape of test that cannot detect a
   convention error. Full QR has ISO's worked Annex I examples as an anchor, so it is much
   better defended — but rMQR, Kanji/Shift-JIS and ECI handling have no external anchor at
   all. Extending the golden-vector approach to those is a plausible next gap; out of scope
   here and not started.
