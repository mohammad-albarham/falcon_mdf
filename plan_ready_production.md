# falcon_mdf — Path to a Production-Ready Rust MDF Library

Status of this document: living plan. Written 2026-08-02 against commit `8de61b9`.
Last updated 2026-08-02 — **Phases 0–3 complete; Phase 4 partial (VLSD + metadata done).**

---

## 0. Progress tracker

Legend: `[x]` done · `[~]` in progress · `[ ]` not started

### Phase 0 — Repo hygiene ✅ **COMPLETE**

- [x] **0.1** `LICENSE-MIT` + `LICENSE-APACHE` added (crate claimed a dual
      license it did not ship — was blocking `cargo publish`)
- [x] **0.2** `Cargo.toml`: real repository URL, real author, `rust-version =
      "1.80"`, `exclude` list, `thiserror` 1.0 → 2.0
- [x] **0.3** Deleted 5 stray root files (9.1 MB): `bootstrap.sh`, `install.sh`,
      `install.sh.sha256`, `export_CAN_DataFrame.csv`, `output.csv`
- [x] **0.4** Build warnings **38 → 0**; clippy **11 → 0**; `warn(missing_docs)`
      promoted to `deny`; whole tree rustfmt'd
- [x] **0.5** `.github/workflows/ci.yml` — 5 jobs, all verified green locally

### Phase 1 — Correctness ✅ **COMPLETE**

- [x] **1.0** Golden-value regression test — `tests/golden.rs` + `tests/data/golden.json`,
      1,775 verified channel entries across the 8-file corpus
- [x] **1.1** Unsorted data group demultiplexing — **the blocker, cleared**
- [x] **1.2** Channel naming (`CAN_DataFrame.CAN_DataFrame.ID` → `CAN_DataFrame.ID`)
- [x] **1.3** Typed signal values (`SignalValues` enum) — plus two bugs it
      surfaced: a VLSD channel silently returning zeros, and a shift-overflow
      panic on full-width unaligned fields
- [x] **1.4** Conversions 3, 6–8 implemented; 9–11 explicitly unsupported;
      `_ => raw` fallthrough gone — plus a reversed rational-polynomial formula
      fixed
- [x] **1.5** Invalidation bits — `Signal::validity`, `is_valid`, `valid_count`

### Phase 2 — Robustness ✅ **COMPLETE**

- [x] **2.1** Unchecked slicing replaced with checked lookups (9 sites)
- [x] **2.2** Sizes validated before allocating; allocation hints clamped
- [x] **2.3** Link-chain cycle detection — `LinkChain` + composition depth bound
- [x] **2.4** `cargo-fuzz` harness, 11 robustness tests, CI smoke job
- [x] **2.5** `mmap` soundness contract documented; `deny(unsafe_code)` crate-wide

### Phase 3 — Performance ✅ **COMPLETE** (uncompressed target missed: 2.5× of 3×)

- [x] **3.1** Record buffers cached per channel group and shared via `Arc`;
      R1 closed without needing the lazy walk
- [x] **3.2** Strided fast path for byte-aligned channels, with a differential
      test against the general path
- [x] **3.3** `criterion` benches with throughput; CI compiles them
- [x] **3.4** Strided path generalised to packed bitfields, after profiling
      showed decode was 88% of the cost and bitfields missed the fast path

### Phase 4 — Format coverage (partial)

- [x] **4.1** VLSD — variable-length payloads, both storage forms
- [x] **4.2** CA arrays — contiguous arrays decode to their elements, verified
      against a synthetic file
- [x] **4.3** AT, EV, CH, SR — all verified end to end against synthetic files
- [~] **4.3-old** AT, EV, CH, SR — parsers present and corrected against the
      reference; still no file to validate end to end. SR is parsed but not
      wired into the reader.
- [x] **4.4** MD metadata parsed into comment + named properties

### Phase 4.5 — Pre-1.0 correctness and coverage

Found by auditing the crate against what 1.0 makes permanent. Ordered by cost of
getting it wrong.

- [x] **4.5.1** **Silent truncation above 4 GB.** `VlsdPayloads` stores payload
      positions as `u32` via `as` casts, and `SignalValues::VarBytes` indexes
      with `Vec<u32>`. A file whose variable-length payloads exceed 4 GB — routine
      in automotive testing — produces wrong offsets with no error. Same class of
      defect as B4 and B8, still present.
- [~] **4.5.2** **Large files are fully materialised.** A data group is read
      into one buffer and the record cache retains it, so a multi-gigabyte group
      is a multi-gigabyte allocation held after use. The cache needs a size
      budget, and oversized groups should not be retained.
- [x] **4.5.3** **`#[non_exhaustive]` on public enums.** `Mf4Error`,
      `SignalValues`, `ValueKind`, `Conversion`, `ChannelType`, `DataType` and
      `UnreadableReason` are all frozen the moment 1.0 is tagged. CA arrays,
      AT/EV/CH/SR and MDF 3.x will each want new variants; without the attribute
      every one of them is a major-version break.
- [x] **4.5.4** **Big-endian channels are never executed.** No test, no corpus
      data. The general path's big-endian branch was left untouched during
      Phase 3 precisely because its bit-offset semantics could not be verified —
      it could be as wrong as B7 was, and nothing would say so. Fixable with
      synthetic round-trip tests; needs no new files.
- [x] **4.5.5** **Only version 4.11 is tested.** All eight corpus files are
      4.11; 4.0 and 4.2 are advertised and unexercised.
- [x] **4.5.6** `OpenOptions::max_alloc` still a compile-time constant, deferred
      from Phase 2.2.
- [x] **4.5.7** README corrections: it claims zero-copy (there is a copy per
      data group, measured at 8% of a read), claims conversion support that
      types 9–11 do not have, and its quickstart predates `SignalValues`,
      `validity()` and `metadata()`. No CHANGELOG exists.

### Phase 4.8 — Remaining format features

Taken from what the code itself declares unimplemented, ordered by the cost of
leaving it as is rather than by size.

- [x] **4.8.1** **Sync and maximum-length channels decode as fixed-length.**
      Neither is special-cased, so a file containing one is decoded as though
      its samples were ordinary fixed-width values — silently, with no error and
      no test. That is the same failure mode as B4 and B8. Making them report
      `Unsupported` costs little and removes a whole class of quiet wrongness.
- [x] **4.8.2** **Conversion types 9 and 10** (text→value, text→text). Both map
      a *string* sample through a table. The tables are simple; the prerequisite
      is that string channels are not currently fed into conversions at all.
      Done: `Conversion::input()` reports which conversions are keyed by text,
      and `Signal::values` decodes those channels as strings — from the record
      or from a VLSD payload stream — before the lookup. Layouts taken from the
      reference: type 9 has one `cc_ref` per key and its default in the *last
      `cc_val`*, unlike type 7's default link; type 10 alternates key and
      replacement references with a single default at the end, so an even
      reference count is malformed and is now rejected.
- [x] **4.8.3** **Conversion type 11** (bitfield→text), which decodes a flag word
      into labels and needs nested conversions resolved.
      Done: each `cc_ref` resolves to a label or to a nested CC block, read by
      peeking at the block id, with a depth limit so a self-referential link
      errors instead of overflowing the stack. Type 11 alone stores its `cc_val`
      as `u64` masks rather than doubles — confirmed by both references.
      **The two references disagree on presentation**, so the rendering here is
      a judgement, documented on `Conversion::render_bitfield`: fragments joined
      with `" | "`, a named nested table rendered as `name = text`, and a bare
      label treated as a flag emitted only when its mask selects a set bit.
      asammdf emits such a label unconditionally, which makes its mask dead;
      mdfr skips bare labels entirely.
- [x] **4.8.4** **Column/row array storage.** Elements of the same index grouped
      across records rather than within one. Reported unreadable today, which is
      honest but limiting.
      **This item was based on a wrong premise, and scoping it found B20.** MF4
      has no "column/row" storage: `ca_storage` selects which block templates
      the elements — 0 = CN (adjacent in the record), 1 = CG, 2 = DG — and the
      codes for 0 and 1 were inverted here. The effect was the opposite of what
      this item assumed: ordinary array channels were rejected, and the one form
      whose elements really are stored elsewhere was decoded as though they were
      adjacent. Codes corrected against both references, the fixture that
      encoded the inversion fixed, and CG/DG-template arrays now stay unreadable
      with an accurate reason. Gathering elements across groups remains
      unimplemented — but that is a genuinely rare form, not the common one.

      A lesson worth keeping: **a synthetic fixture written from the
      implementation tests the implementation against itself.** The array tests
      passed throughout because the fixture set `ca_storage = 1` for the case
      the code called contiguous. Fixtures for a format must take their field
      values from the specification or a reference, never from the reader they
      are meant to check.

### Phase 4.9 — Completing 4.11

Scope is now **4.11 only**. That closes 4.5.5 and the 4.2-era blocks as out of
scope, and leaves the list below as everything between the crate and complete
read coverage of the target version. Taken from §8, ordered by the cost of
leaving it rather than by size.

B21 sets the standard for all of them: the corpus is not evidence for a feature
no corpus file exercises, and three of these four are in exactly that position.
Each needs a synthetic fixture built from the specification, and each fixture
must be shown to fail against the current behaviour before it is trusted.

- [x] **4.9.1** **`DataType::Unknown(v)` decodes as a byte blob.**
      `Channel::value_kind` catches it in the `_ => ValueKind::Bytes` arm, so an
      unrecognised type code presents as a byte array rather than erroring. This
      is the B8 failure mode — a silent, plausible-looking answer to a question
      the reader cannot actually answer — in the last place still open to it.
      Cheapest item here and the one whose absence is least defensible.
- [x] **4.9.2** **Invalidation bits have never been executed.** Implemented in
      1.5, but no corpus file uses them and `validity_is_reported_consistently`
      only asserts self-consistency — its own assertion records that every
      corpus channel is wholly valid. So the masking path has never run on input
      that exercises it. This is B21's shape exactly: a feature believed correct
      on the strength of a test that could not have failed.
      **No defect found — the implementation was right.** Two synthetic files
      now drive it end to end, through `cg_inval_bytes`, `cn_flags` bit 1 and
      `cn_inval_bit_pos` rather than a hand-built layout, with two channels
      sharing one invalidation byte at different bit positions. Since both
      passed on the first run, the tests were checked for power by mutating the
      implementation three ways — inverted polarity, ignored bit position,
      dropped `inval_start` — and each was caught. That check is the point: a
      test that passes against a correct implementation and a broken one is
      what left this item open in the first place.
- [x] **4.9.3** **CANopen date/time and complex numbers return raw bytes.** All
      four are 4.11 data types — a 7-byte date, a 6-byte time, and a complex
      number as two floats — falling into the same `_ => Bytes` arm as 4.9.1.
      Returning the bytes is honest, unlike 4.9.1, so this is a coverage gap
      rather than a defect. Layouts must come from the standard: the CANopen
      fields are packed with reserved bits inside them, not plain integers.
      Done: `CanopenDate` and `CanopenTime` are structs, not timestamps. The
      standard defines these as records with named fields, two of which —
      day-of-week and the summer-time flag — no instant can represent, so
      collapsing them to nanoseconds would be interpretation rather than
      decoding. `to_unix_nanos` gives the instant where that is what is wanted.
      Complex becomes `SignalValues::Complex { re, im }`, split rather than
      interleaved so taking the real part is a slice.
      **Not verifiable against a real file**, which is the risk this plan warns
      about: no corpus file carries any of the three, so the layouts come from
      the CiA 301 records the MF4 standard refers to, documented field by field
      at each decoder. The date arithmetic is checked against an independent
      calendar implementation, and the fixtures set every reserved bit so a
      decoder that reads bytes whole instead of masking fails — verified by
      removing the masks, which turns year 2026 into 2154.
- [x] **4.9.4** **MLSD channels report `Unsupported`.** Correct as a stopgap
      from 4.8.1, and decodable: the record holds a per-sample length beside the
      data rather than an offset into a separate block, which is what makes it
      unlike VLSD. Largest of the four.
      Done, and the earlier description of it was imprecise in a way worth
      correcting: the length is not simply "beside the data". `cn_data` on an
      MLSD channel points at a **CN block** — another channel of the same group,
      whose value counts the bytes each sample uses — where the same link on a
      VLSD channel points at a signal data block. Resolving that link is the
      whole of the work; both halves are already in the record.
      Decodes to `VarBytes` even when every sample is the same length: a channel
      declaring itself maximum-length is saying its samples vary, and a fixed
      width would erase the difference between "eight bytes used" and "eight
      bytes available". A count past the declared maximum is rejected rather
      than clamped, since clamping hands back the neighbouring channel's bytes.
      A channel with no `cn_data` link stays `Unsupported`, which is now the
      accurate reason rather than a blanket refusal.

Left undone deliberately, and recorded so the gap is not mistaken for an
oversight:

- **Sync channels (`cn_type` 4)** index a media stream rather than carrying
  measurements. `Unsupported` is the right answer, not a placeholder.
- **CG/DG-template arrays** gather one sample's elements across separate record
  streams. Genuinely rare, and the error it reports is accurate about why.

### Phase 4.10 — Auditing 4.9's claim that nothing is left

Phase 4.9 closed with "What this leaves for 4.11: nothing." Reading the code
against the standard's own field tables rather than against the phase list found
five gaps, two of them B20's exact shape: **the implementation and the fixture
that verifies it share one misreading, so every test passes.** Ordered by cost
of leaving them.

- [x] **4.10.1** **The `cn_data_type` table is off by one from code 6 up.**
      `DataType::from_u8` reads 6 as UTF-8, 7 as UTF-16LE, 8 as UTF-16BE, 9 as
      byte array, and so on to 14 as complex. The standard assigns 6 to
      **string SBC (ISO-8859-1)** — a variant this enum does not have at all —
      then 7 UTF-8, 8 UTF-16LE, 9 UTF-16BE, 10 byte array, 11 MIME sample,
      12 MIME stream, 13 CANopen date, 14 CANopen time, 15/16 complex.
      So a UTF-8 channel is decoded as UTF-16 and a UTF-16LE channel is
      byte-swapped — both silent garbage — a MIME stream is decoded as a
      CANopen date, and every layout 4.9.3 masked reserved bits for is applied
      to the wrong channel. `DataType` is deliberately not `#[non_exhaustive]`,
      which makes this the last cheap moment to fix it.
      **Done.** Codes renumbered, `DataType::StringSbc` added with an
      ISO-8859-1 decoder — a widening cast per byte, not a UTF-8 decode, since
      0xD6 is Ö in one and the start of a sequence in the other — and the
      4.9.3 fixtures moved onto the codes they always meant. Two tests, chosen
      because the failure mode here is *plausible output*, not an error:
      a synthetic file carrying one channel per string encoding, and a unit
      test naming all seventeen codes.
      **The fixture text is non-ASCII on purpose.** ASCII survives all four
      string encodings unchanged, so a fixture written in it passes against the
      shifted table and proves nothing — the same trap as B21's zero conversion
      factor. Each channel carries text that only its own encoding renders
      correctly. Shown to fail first, then checked for power by three
      mutations: shifting the table back caught all four channels
      independently, decoding SBC as UTF-8 caught the SBC channel, and ignoring
      UTF-16 endianness caught the big-endian one.
- [x] **4.10.2** **CA flags, link partition and data section are all misread.**
      The parser maps bit 0 to "has axis" and invents an "axis name" flag and a
      precomputed min/max region in the data section. The standard's bits are
      0 dynamic size, 1 input quantity, 2 output quantity, 3 comparison
      quantity, 4 axis, 5 fixed axis, 6 inverse layout, 7 left-open interval,
      8 standard axis; the links those flags introduce are **triples**
      (dg, cg, cn), not one per dimension, and the only optional data after the
      dimension sizes is the fixed-axis values. Element decoding survives —
      `ca_composition` is link 0 and `ca_dim_size` precedes every optional
      field — so what is lost is the whole of the axis metadata.
      **Done.** Flags renumbered, `ca_axis_name` and the precomputed min/max
      data region deleted as things the standard does not have, `ca_scale_axis`
      replaced by `ca_axis: Vec<AxisRef>` carrying the (dg, cg, cn) triple the
      standard actually stores, and the link walk taught to *count* the
      sections it does not use — DG-template data links, dynamic size, and the
      input, output and comparison quantities — since skipping one by the wrong
      width is what hands back a dynamic-size link as an axis. `ca_type` 2 is
      the look-up table, not a "type template", and 3 is not defined; the
      composition link is link 0 whatever `ca_type` says, where the parser used
      to suppress it for a scale axis.
      **The fixture was the defect.** `create_ca_block` derived every optional
      section from `CaFlags`, so it emitted whatever layout the parser expected
      and all six tests passed against a block no writer would produce. It now
      spells the bit values out as literals and lays the block out from the
      standard, which is the only version of this fixture worth having. Power
      checked by three mutations: the old flag numbering failed five tests, an
      axis read as one link per dimension failed exactly the triple test, and
      dropping the DG-template data links failed exactly the DG test —
      reporting links 2000 and 2001 as axis conversions, which is the defect in
      miniature.
      **One consequence worth its own line: dynamic-size arrays now report
      unreadable.** `ca_dim_size` on such an array is the largest shape a
      sample may take, not the shape any sample has, so decoding it as fixed
      returns the unused tail of the field as data — plausible numbers in the
      right dtype that are not the array. Nothing here could have known that
      before, because bit 0 was being read as "has axis".
- [x] **4.10.3** **`cn_flags` bit 0, "all values invalid", is parsed and
      dropped.** It never reaches `Channel`, and `validity()` consults only the
      per-sample invalidation bit. A channel the file declares wholly invalid is
      reported wholly valid and its values handed back as measurements — B10
      again, in the one place 4.9.2 did not look.
      **Done.** `Channel::all_invalid` carries the flag, and `validity()`,
      `is_valid` and `valid_count` answer it before consulting the record —
      which they must, because the flag stands alone: it needs no per-sample
      invalidation bit and no `cg_inval_bytes` in the group, so every existing
      early return would have skipped it. A flagged channel now reports
      `Some(vec![false; n])` rather than `None`.
      **The fixture carries two channels, one flagged and one not**, because
      the obvious wrong fix — treating the flag as a property of the group, or
      returning all-invalid unconditionally — passes any test that only looks
      at the flagged one. Power checked by three mutations: dropping the flag
      on the way to `Channel` (the original defect), honouring it in
      `validity()` but not `is_valid`, and marking every channel invalid. The
      third is caught only by the unflagged neighbour.
- [x] **4.10.4** **An unrecognised data block yields zero samples, not an
      error.** `build_data_block_index` falls through to an empty index, so a
      data group pointing at a block this reader does not know reports every
      channel as empty rather than unsupported; the DL walk drops unknown links
      the same way, mid-stream. Compounded by `Mf4Version::is_supported`, which
      still answers `true` for 4.0 and 4.2 although the scope is 4.11 — so a
      4.2 file with `##LD` data blocks opens and reads as empty.
      **Done.** Both fallthroughs now error and name the block. Two tests, one
      per path: a data group pointing at `##LD` (which is what a 4.2 file uses
      in place of a DL), and a data list naming one. The second is the worse
      case and is worth stating plainly — a list holds one block per segment of
      a group's records, so skipping an entry drops a slice out of the middle
      of the stream and shifts every segment after it. What survives is real
      values at the wrong times, which no caller could detect.
      **The version check was left alone, deliberately.** Gating `open` on 4.11
      would refuse 4.0 and 4.2 files that use nothing this build lacks — most
      of them — while still not catching a 4.11 file that uses something it
      does. A version number says which blocks a file *may* contain, not which
      it does. Failing at the unreadable block, by name, is both stricter and
      more permissive in the right directions. `is_supported` keeps its
      behaviour and loses its overstated documentation.
- [x] **4.10.5** **Unfinalized flags are collapsed to one boolean.** Only
      "DT length is zero, so read to end of file" is acted on. The individual
      `id_unfin_flags` bits — SR cycle counts, last DL update, VLSD CG byte
      counts, VLSD offset values — are neither distinguished nor surfaced. A
      reporting gap rather than wrong data, since the record walk recomputes
      cycle counts regardless.
      **Done.** `UnfinalizedFlags` carries all seven bits plus the writer's own
      custom word, and `Mf4File::unfinalized()` returns it — `None` for a
      finalized file. Not acted on beyond the two already compensated for, and
      that is the answer rather than a shortfall: inventing the missing values
      would be guessing, and refusing the file would withhold the channels that
      are fine. What the caller gets is the ability to tell the two apart —
      stale sample counts, which this reader fixes by taking counts from the
      data, against VLSD offsets that were never written, which it cannot.
      Each flag's documentation says which of the two it is.
      Power checked by three mutations: a shifted bit table, a dropped custom
      word, and reporting flags for a finalized file — the last caught only by
      the second test, which is why there are two.

**How these were missed, and it is the same answer twice.** The corpus carries
only data types 0, 4 and 10, and 10 lands on `MimeSample`, which `value_kind`
funnels into `Bytes` exactly as `ByteArray` does — the output is identical, so
the reference comparison had no power to disagree. The CA fixture takes its flag
bits from `CaFlags` rather than from the standard, so all six CA tests pass
against a parser that contradicts the specification. §8's data-type table
carries the same off-by-one as the code, which is what happens when a coverage
table is written from the implementation it is meant to audit.

**Sources.** asammdf's `v4_constants.py` for both tables, and — independently —
the ASAM e.V. DataPlugin for MDF4 readme, which states its support as "channels
with data types #0-9 and #13-14" and names #10-12 as byte array, MIME sample and
MIME stream. Neither is the standard itself; the spec PDF is worth consulting
before 4.10.2 is called done.

### Phases 5–6
- [x] **4.6** FH (file history) — parsed and verified against the corpus
- [x] **4.7** Sample reduction — descriptors and reduced values, both verified
- [ ] **5** Write support
- [ ] **6** API freeze and 1.0

---

## 0.5 Bug register

Every defect found so far, with how it was found and whether it is fixed.
All twenty-six are closed. B22–B25 came from the 4.10 audit; B26 from the first
third-party files, which is the validation this plan had been calling for.

Only two of these (B1, and B11/B12 as a pair) were visible in the original
assessment. The rest surfaced while building the regression net and the fuzz
harness — which is the argument for having built them first.

### Fixed

| # | Defect | Site | Severity | Found by | Fixed in |
|---|---|---|---|---|---|
| **B1** | Unsorted data groups strided with one channel group's record size, reading across record boundaries. Produced garbage on **every** real logger file (`Timestamp` = −1.58e300). | `file.rs` `signal()` | **Critical** | Reference comparison | 1.1 |
| **B2** | Sample counts fabricated from data size when `cycle_count == 0` — 40,523 invented samples for groups that were genuinely empty. A stale declared count also won over what the stream actually held. | `file.rs` | High | Golden test | 1.1 |
| **B3** | Composition children double-prefixed: `CAN_DataFrame.CAN_DataFrame.ID`. Breaks any script ported from another tool. | `file.rs` `expand_composition_channels` | Medium | Reference comparison | 1.2 |
| **B4** | VLSD channels returned eight zero bytes. The record holds an *offset* into a signal-data block; the decoder read the offset as if it were the payload. | `model/signal.rs` | High | Golden byte comparison | 1.3 (now errors) |
| **B5** | Shift-overflow **panic** on a 64-bit field at a non-zero bit offset: the nine-byte window does not fit a `u64`, and the mask `1u64 << 64` overflowed too. Reachable from any file declaring that layout. | `parser/binary.rs:78` | High (crash) | Reasoning, then a failing test | 1.3 |
| **B6** | Byte-array channels forced through `f64` — a `u8[8]` CAN payload arrived as `1.8e19`. | `model/signal.rs` | High | Reference dtype comparison | 1.3 |
| **B7** | Rational conversion evaluated with **reversed coefficients**: `(p0 + p1·x + p2·x²)/(p3 + p4·x + p5·x²)` instead of the specified `(P1·x² + P2·x + P3)/(P4·x² + P5·x + P6)`. Would have produced confidently wrong physical values. | `blocks/conversion.rs` | High | Checked against the reference's own expression | 1.4 |
| **B8** | Conversion types 3 and 6–11 fell through `_ => raw` and returned raw values silently. A text-table channel yielded meaningless numbers that looked like measurements. | `blocks/conversion.rs:165` | High | Code review | 1.4 |
| **B9** | Empty conversion tables fell through to the raw value, i.e. a silent identity. | `blocks/conversion.rs` | Low | Code review | 1.4 |
| **B10** | Invalidation bits parsed but never applied: samples the file marks invalid were returned as if they were measurements. | `model/signal.rs` | Medium | Code review | 1.5 |
| **B11** | **Panic** on unchecked slicing of malformed blocks — 6 crashes per 400 structural mutations. | `blocks/header.rs:233` + 8 more sites | **Critical** | Mutation fuzzing | 2.1 |
| **B12** | **Process abort** from unbounded allocation: `memory allocation of 7638104968021014462 bytes failed`. Uncatchable, so worse than a panic. Three distinct sources — an unvalidated `link_count`, a block `length` exceeding the file, and a `cycle_count` larger than the data could hold. | `blocks/common.rs`, `parser/mod.rs`, `file.rs` | **Critical** | Mutation fuzzing | 2.2 |
| **B13** | **Infinite loop** on a self-referential link, with unbounded memory growth. A crafted `dg_next` never terminated. | `file.rs`, 5 link walks + 1 recursion | High | Crafted input, verified | 2.3 |
| **B19** | Embedded attachment bytes were read from `offset + length` — past the payload, since a block's declared length already covers it. Reading an embedded attachment returned whatever followed the block, or failed outright when it was the last thing in the file. | `blocks/attachment.rs` | High | Synthetic end-to-end fixture | 4.3 |
| **B18** | `channel_count()`, `find_channel`, `has_channel` and `channel_names` all read from the name index rather than the groups, so opening with `build_channels_db: false` reported zero channels and found none — a documented memory/speed trade-off silently became a switch that changed which channels existed. | `file.rs` | Medium | Read-path system test | Phase 3 second pass |
| **B16** | `comment()` returned the raw XML of a metadata block — 877 characters of markup — instead of the comment inside it, leaving every caller to parse XML. | `blocks/text.rs` | Low | Inspecting corpus metadata | 4.4 |
| **B17** | An array channel was left in the channel list with its CA composition skipped, so reading it returned the first element while presenting as the whole channel. | `file.rs` | Medium | Corpus block scan during Phase 4 | 4.2 (now fails loudly) |
| **B15** | Variable-length payload offsets read using the channel's declared endianness. A channel's type describes its *payload*, not the byte order of the offset pointing at it, so every VLSD channel whose payload type was not explicitly little-endian resolved a byte-reversed offset — `0x0C00000000000000` for `12` — and returned empty payloads for all but the first sample. | `model/signal.rs` | High | Golden byte comparison after implementing VLSD | 4.1 |
| **B20** | **`ca_storage` codes were inverted.** The standard assigns 0 = CN template (elements adjacent in the record), 1 = CG template, 2 = DG template; the code had 0 = column/row (rejected) and 1 = contiguous (decoded). So every *ordinary* array channel was refused as unreadable, while a CG-template array — whose elements are in other channel groups entirely — was strided as though they were adjacent, returning whatever bytes followed the field. The synthetic fixture encoded the same inversion, which is why it passed. | `blocks/channel_array.rs`, `file.rs` | High | Cross-checking `ca_storage` against both references while scoping 4.8.4 | 4.8.4 |
| **B21** | **Virtual channels decoded as constants.** A virtual channel (`cn_type` 3 and 6) occupies no bytes: `cn_bit_count` is 0 and its raw value is the sample's zero-based index, which the conversion scales. The reader had no rule for them, so they fell through to fixed-length decoding and read a zero-bit field — raw 0 for every sample. A virtual master, whose whole purpose is a regularly-spaced time base stored as nothing but a factor, returned a flat line. | `model/signal.rs`, `model/mod.rs` | High | Probing the corpus while scoping 4.11 | 4.11 |
| **B14** | `memmap2::Mmap::map` unsound if the file is externally truncated — SIGBUS, uncatchable. A safe-looking public API with an undocumented obligation, on the **default** backend. | `io/mmap.rs:62` | Medium | By construction | 2.5 |
| **B22** | **`cn_data_type` read one code too low from 6 upwards.** The standard assigns 6 to string SBC (ISO-8859-1), which this enum has no variant for; every code above it therefore shifts. A UTF-8 channel decodes as UTF-16LE and a UTF-16LE channel byte-swaps — silent garbage text — a MIME stream decodes as a CANopen date, and a CANopen date as a CANopen time. The corpus could not show it: it carries only types 0, 4 and 10, and 10 lands on `MimeSample`, whose output is byte-for-byte what `ByteArray` produces. | `blocks/channel.rs`, `model/signal.rs`, `model/mod.rs` | **Critical** | Checking the enum against asammdf and the ASAM DataPlugin readme | 4.10.1 |
| **B26** | **A numeric conversion turned a text channel into numbers.** `value_kind` consulted the conversion before the data type, so a string channel carrying any non-identity numeric conversion was decoded as a number — the text bytes read as an integer and pushed through the conversion. `ASAP2_Demo_V171.mf4` hangs an identity *rational* on a 256-byte SBC field, which came back as 0.0 for every sample; a synthetic 8-byte case returns 8.09e18, the ASCII of the text read as a `u64`. Same shape as B6. | `model/mod.rs` | High | The first third-party file with a string channel, compared against asammdf | 4.11 |
| **B23** | **CA flags misnumbered, and the link and data layouts that follow from them wrong.** Bit 0 read as "has axis" where the standard has dynamic size; an "axis name" flag and a precomputed min/max data region invented outright; the links each flag introduces read as one per dimension where the standard has (dg, cg, cn) triples. A spec-layout fixed-axis array parses with its axis values dropped, one of them reported as a precomputed minimum, and the axis *conversion* link reported as a scale axis. Element values are unaffected. **The fixture encodes the same misreading**, which is why six tests pass. | `blocks/channel_array.rs` | High | Cross-checking `ca_flags` against asammdf while auditing 4.9 | 4.10.2 |
| **B24** | **`cn_flags` bit 0 ("all values invalid") parsed and dropped.** It never reaches `Channel`, so `validity()` reports a channel the file declares wholly invalid as wholly valid and returns its values as measurements. | `blocks/channel.rs`, `model/signal.rs` | Medium | Code review against the flag table | 4.10.3 |
| **B25** | **An unrecognised data block reads as zero samples.** `build_data_block_index` falls through to an empty index rather than an error, and the DL walk skips unknown links mid-stream. A 4.2 file with `##LD` blocks — which `Mf4Version::is_supported` still accepts — opens successfully and reports every channel empty. | `file.rs`, `parser/version.rs` | Medium | Code review of the fallthrough arms | 4.10.4 |

### Open

None.

### Known regression, accepted

| # | Change | Effect | Why | Status |
|---|---|---|---|---|
| **R1** | The unsorted record walk runs for every unsorted data group, not only when `cycle_count == 0`. | `open` was 2.25 ms → 6.95 ms on the large corpus file | The index it builds is what makes reads correct | **Closed in Phase 3.** `open` is now 1.99 ms — better than the original baseline — without the lazy walk the plan called for. Phase 2's allocation clamping and 3.1's buffer sharing more than paid for the walk. |

### Reproducing

Each of B11–B13 is now pinned by a test in `tests/robustness.rs`, so they cannot
return silently:

| Defect | Test |
|---|---|
| B11 | `mutated_files_never_panic` — 300 deterministic mutations |
| B12 | `a_block_longer_than_the_file_is_rejected`, `an_absurd_link_count_is_rejected`, `an_inflated_cycle_count_cannot_exceed_the_data` |
| B13 | `a_self_referential_{data_group,channel_group,channel}_link_is_rejected` |

Beyond that, `fuzz/fuzz_targets/parse.rs` drives the whole read path — open, then
decode every channel — under `cargo +nightly fuzz run parse`.

---

## 0.6 Verification logs

What was measured as each item landed, kept so the claims above can be checked
rather than taken on trust.

### Phase 0 verification log

All five CI gates confirmed passing locally before the workflow was committed:

| Gate | Result |
|---|---|
| `cargo build --all-targets` (`-D warnings`) | clean |
| `cargo test` | 115 passed, 0 failed |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `cargo fmt --all --check` | clean |
| `cargo doc` (`RUSTDOCFLAGS=-D warnings`) | clean |
| `cargo publish --dry-run` | 47 files, 309.9 KiB |
| `cargo +1.80 build` (MSRV) | clean |

Behaviour was re-verified unchanged after the refactor: sorted-file output is
still byte-exact (`Timestamp` 186.90885–743.58405, `CAN_DataFrame.ID` 2015–2024).

### Phase 1.0–1.2 verification log

Golden-test mismatch count against ground truth, as each fix landed:

| After | Mismatches | Files failing |
|---|---|---|
| 1.0 — net in place (baseline) | 1,780 | 8 / 8 |
| 1.2 — naming fixed | 1,809 * | 8 / 8 |
| **1.1 — demux landed** | **0** | **0 / 8** |

\* The count rose at 1.2 because channels that previously failed as "not present
in ground truth" started being *compared*, converting one name mismatch into
several value mismatches. Total mismatch count is therefore not monotonic; files
failing is the honest headline.

Effect on the previously-broken unsorted corpus:

| Channel | Ground truth | Before 1.1 | After 1.1 |
|---|---|---|---|
| `Timestamp` | 186.90885 – 743.58405 | −1.58e300 – 1.49e299 | ✅ exact |
| `CAN_DataFrame.ID` | 2015 – 2024 | 0 – 536870911 | ✅ exact |
| `CAN_DataFrame.BusChannel` | 1 – 1 | 0 – 3 | ✅ exact |
| `CAN_DataFrame.DLC` | 8 – 8 | 0 – 15 | ✅ exact |

Performance side effects, large unsorted file (`00000014-64BBA8AF.MF4`):

| | Before | After | |
|---|---|---|---|
| Sample count | 81,614,022 (fabricated) | 1,135,638 (correct) | — |
| `open` | 2.25 ms | 6.95 ms | slower — see below |
| `read_all` | 423.93 ms | 67.95 ms | 6.2× faster |

`open` regressed because the record walk now runs for *every* unsorted data
group, not only when `cycle_count == 0`. It has to: the index it produces is
what makes reads correct. Making the walk lazy (built on first `signal()` call,
behind a `OnceCell`) restores fast opens and is folded into Phase 3.1.

Sorted-file performance is unchanged (2.04 ms / 12.82 ms read_all), confirming
the sorted path was not disturbed.

### Phase 1.3 verification log

`SignalValues` / `ValueKind` landed, and the golden test was widened to assert
the *decoded type* of all 1,775 channels plus the literal first-sample bytes of
byte-array channels. Every type expectation matched on the first run: 29-bit CAN
identifiers decode to `u32`, 2-bit bus numbers to `u8`, converted timestamps to
`f64`, frame payloads to `Bytes`.

Widening the test surfaced two real bugs that the numeric-only comparison had
been hiding:

**1. `CAN_DataFrame.DataBytes` returned eight zero bytes.** The channel is
`ChannelType::VariableLength` — its record holds a 64-bit *offset* into a
signal-data block, not the payload. The decoder was reading the offset field as
if it were data. VLSD decoding is Phase 4 work and stays deferred, but returning
zeros is precisely the silent-wrongness this plan argues against, so
`Signal::values` now fails with `Mf4Error::Unsupported` for VLSD channels. The
golden test asserts they *error*; when Phase 4 lands, that assertion flips to
comparing real payloads.

**2. Shift-overflow panic in `read_uint`.** A 64-bit field at a non-zero bit
offset spans nine bytes, so the assembly window does not fit in a `u64`:

```
thread '...' panicked at src/parser/binary.rs:78:22:
attempt to shift left with overflow
```

Both the accumulator shift and the mask (`1u64 << 64`) overflowed. Fixed by
accumulating in `u128` and narrowing after the shift. Reachable from any file
declaring that layout, so this was a genuine crash on untrusted input — a Phase 2
class of bug found early.

Coverage note, stated plainly: only 11 byte-array channels in the corpus carry
byte-level ground truth (the rest have zero samples), and 5 of those are the
deferred VLSD ones. So 6 channels are byte-compared. That is thin, and worth
strengthening with synthetic fixtures in Phase 4.

Test count: 121 → **140**.

### Phase 1.4 verification log

Every MF4 conversion type now maps to an explicit variant. The `_ => raw`
fallthrough is gone, and so is `CcBlock::convert` — the public method that
carried it.

| Type | Name | Status |
|---|---|---|
| 0 | identity | implemented |
| 1 | linear | implemented |
| 2 | rational | implemented — **coefficient order corrected** |
| 3 | algebraic | implemented (formula parser, `src/blocks/formula.rs`) |
| 4 | value→value, interpolated | implemented |
| 5 | value→value, no interpolation | implemented |
| 6 | range→value | implemented |
| 7 | value→text | implemented, decodes to `SignalValues::Str` |
| 8 | range→text | implemented, decodes to `SignalValues::Str` |
| 9 | text→value | `Unsupported` — needs string channel input |
| 10 | text→text | `Unsupported` — needs string channel input |
| 11 | bitfield→text | `Unsupported` — needs nested conversion resolution |

**The rational conversion was computing the wrong answer.** The old code
evaluated `(p0 + p1·x + p2·x²) / (p3 + p4·x + p5·x²)`, with the coefficients in
reverse. ASAM MDF4 defines it as:

```
y = (P1·x² + P2·x + P3) / (P4·x² + P5·x + P6)
```

Confirmed against the reference implementation's own expression rather than
assumed. No file in the corpus uses type 2, so nothing caught this — it would
have produced confidently wrong physical values on the first file that did.

Two design points worth recording:

- **Text conversions change the channel's type.** A value→text table makes the
  channel read as `ValueKind::Str` no matter how its raw bits are stored, so
  `Channel::value_kind` consults `Conversion::output` first.
- **Text is resolved at open time.** Tabular text conversions keep their strings
  in referenced TX blocks, so `Conversion` is now built by `build_conversion`,
  which has the file and block cache to hand. `CcBlock` alone cannot see those
  references, which is why it no longer offers a `convert` method at all.

Empty conversion tables are rejected at build time rather than falling back to
the raw value, on the same principle.

Corpus coverage is limited — only types 1 (736 channels) and 5 (108) appear —
so types 2, 3, 6, 7 and 8 are covered by 28 synthetic unit tests instead,
including operator precedence, associativity and rejection of malformed
formulas.

Test count: 140 → **161**.

### Phase 1.5 verification log

`cg_inval_bytes` was parsed but never read, so a sample the file marks invalid
was returned as if it were a measurement. `Signal` now exposes:

- `validity() -> Option<Vec<bool>>` — `None` when the channel has no
  invalidation bit, meaning every sample is valid
- `is_valid(index) -> bool`
- `valid_count() -> usize`

Three decisions worth recording:

- **The polarity is inverted at the API boundary.** The format sets a bit to
  mark a sample *invalid*; `validity()` returns `true` for *valid*, which is
  what callers expect. The file's polarity is documented on the method.
- **`values()` does not filter.** Invalid samples stay in the returned data, and
  a test pins that behaviour. Silently dropping or NaN-ing them would change the
  sample count and break alignment with the master channel; callers combine the
  two APIs themselves. This matches how the reference implementation separates
  `samples` from `invalidation_bits`.
- **An out-of-range bit position reports no validity rather than "all valid".**
  Claiming every sample is good on the basis of a malformed field would invent
  information.

The record layout the signal reads through moved into a `RecordLayout` struct,
since `Signal::new` would otherwise have taken seven positional arguments — the
invalidation area sits immediately after the channel data, and its offset has to
travel alongside the stride and record-ID offset.

Coverage: **no file in the corpus uses invalidation bits at all** (0 channel
groups, 0 channels), so there is no ground truth for this and it rests entirely
on 7 synthetic tests — bit positions beyond the first byte, multi-byte
invalidation areas, interaction with the record-ID offset, and the malformed
out-of-range case.

Test count: 161 → **168**.

### Phase 2 verification log

Measured by mutating real corpus files and opening the result in a subprocess,
so an abort or a hang counts as a failure rather than taking the harness with it.

| Campaign | Before Phase 2 | After |
|---|---|---|
| 400 structural mutations (seed 11) | 6 crashes | **0** |
| 3,000 mutations, 20 seeds, three mutation regions | 5 crashes | **0** |
| Self-referential `dg_next` | hung indefinitely, memory growing | rejected immediately |

The 3,000-case campaign after the fixes: `clean=2735 graceful_error=265
PANIC/ABORT/HANG=0`. The 265 are files rejected with a proper error, which is
the correct outcome for a mutated file — the number to drive to zero is the last
column, not that one.

**B12 had three distinct sources**, not one. Fixing the first two left the
campaign still failing, which is why it was worth re-running rather than
declaring victory after the obvious fix:

1. `link_count` was unvalidated, so `Vec::with_capacity(link_count)` allocated
   8 bytes per claimed link. Now `BlockHeader::parse` requires the links to fit
   inside the block's own declared length.
2. A block's `length` was never checked against the file, so a DT block could
   claim to hold 7.6 × 10^18 bytes. Now `parse_block_header` — the single point
   every block header passes through — rejects a block extending past EOF.
3. `cg_cycle_count` was believed as declared, and it sizes every read buffer.
   Now it is clamped to what the data can actually hold: the data is the
   authority, not the header.

Two general principles came out of this and are worth keeping:

- **A pre-allocation is an optimisation, never a requirement.** Every
  `with_capacity` fed by a file-declared number is clamped to `MAX_PREALLOC`
  (64 MiB). A genuinely large file pays a few reallocations; a corrupt one
  cannot turn a size field into an allocation.
- **Validate at the choke point.** Both `BlockHeader::parse` and
  `parse_block_header` are on the path of every block in the file, so a check
  there covers paths that have not been written yet.

Decompression is separately bounded (`MAX_DECOMPRESSED`, 1 GiB) because a DZ
block's expanded size is not limited by the file size — a small block can claim,
or genuinely produce, an enormous amount of data.

`deny(unsafe_code)` now applies crate-wide. Exactly one `unsafe` block remains,
in `io::mmap`, opted in at the site with the reasoning recorded. Its soundness
obligation — the file must not change while mapped — is documented on the method
rather than waved away, and matters because mmap is the **default** backend;
`open_buffered` is now the documented choice for untrusted input.

Tests 181 → **185**, plus a `cargo-fuzz` target driving open-then-decode and a
60-second CI smoke job.

**Not done:** the plan called for a configurable `OpenOptions::max_alloc`. The
limits are compile-time constants instead. That covers the safety case; making
them tunable is deferred to the API review in Phase 6, where the rest of
`OpenOptions` is being revisited anyway.

### Phase 3 verification log

Median of 10 runs each side, same protocol: open the file, then decode every
channel into its native type.

| Read | Reference | falcon_mdf | Ratio | Target |
|---|---|---|---|---|
| Uncompressed | 3.69 ms | **1.49 ms** | 2.5× | 3× — **missed** |
| DZ-compressed | 9.56 ms | **2.36 ms** | 4.1× | 3× — met |

> **These ratios were measured on unequal work and are superseded.** At the time,
> falcon_mdf decoded 296,930 samples against the reference's 326,623, because
> variable-length channels were reported as unsupported rather than decoded.
> Once Phase 4.1 implemented them, both sides decode the same 326,623 samples
> and the honest figures are **1.8× uncompressed and 2.7× compressed** — see the
> Phase 4 log. Neither meets the 3× target.

Against the state at the start of Phase 3:

| | Start of Phase 3 | End | |
|---|---|---|---|
| Read, uncompressed | 7.96 ms | 1.49 ms | 5.3× |
| Read, compressed | 32.08 ms | 2.36 ms | 13.6× |
| Read, large unsorted file | 49.45 ms | 9.66 ms | 5.1× |

That last file read in **423.93 ms** at the start of the session, so it is now
44× faster end to end — though most of that came from Phase 1.1 fixing what the
reads were doing, not from Phase 3 making them quicker.

**The uncompressed target was missed: 2.5× against a goal of 3×.** Two things
would close it, neither attempted here:

- The data group's bytes are still copied out of the mapping into an owned
  buffer. For an uncompressed file that copy is pure overhead — the mapping is
  already the bytes. Removing it means `Signal` borrowing from the mapping, or
  the mapping living behind an `Arc` that slices can be handed out from.
- Multi-channel reads are sequential. `rayon` is already a dependency; a
  `signals(&[&Channel])` entry point could decode channels in parallel, but that
  is new API surface and belongs with the Phase 6 review.

One caveat on the comparison, stated because it flatters these numbers:
falcon_mdf decodes 296,930 samples where the reference decodes 326,623. The
difference is the VLSD and composite channels that this crate reports as
unsupported rather than decoding, so it is doing about 9% less work.

**What actually made the difference:**

1. **Caching the assembled records** (3.1) was worth far more than the decode
   work. `signal()` used to re-read and re-decompress the entire data group for
   every channel — with 19 channels, nineteen times over. Keeping the last
   group's records and sharing them through an `Arc` took compressed reads from
   32.08 ms to 4.42 ms on its own. Only the most recent group is kept, which
   covers sequential access while bounding memory to one group.
2. **The strided fast path** (3.2) took uncompressed reads from 3.52 ms to about
   2.1 ms. It applies when a value occupies whole bytes, starts on a byte
   boundary and is little-endian — nearly every channel in practice. Anything
   else, including packed bitfields and big-endian, still takes the general
   path.

The fast path is guarded by a differential test: the same pseudo-random buffer
is decoded both ways across every width and signedness, and the results must be
identical. A decoder that is quick and disagrees with the slow one would be
worse than no optimisation at all. Writing it also caught a wrong assumption in
one of my own boundary tests, where 30 bytes really was enough for 4 samples at
stride 8.

**R1 is closed**, but not by the predicted mechanism. The plan called for making
the unsorted record walk lazy; that was never needed. `open` on the large
unsorted file is now **1.99 ms**, better than the 2.25 ms baseline from before
Phase 1 introduced the walk, because the allocation clamping in Phase 2 and the
sharing in 3.1 more than paid for it. The walk still runs eagerly.

Benchmarks live in `benches/read.rs` and report throughput in elements per
second, so a change in *what* is decoded cannot be mistaken for a change in
speed. Current figures: 95–235 Melem/s depending on file. CI compiles them but
does not gate on timings — a shared runner is too noisy for that, and the
corpus is not checked in.

Tests 185 → **190**.

### Phase 4 verification log

**4.1 — variable-length signal data (VLSD): done.** The five channels that
Phase 1.3 made fail loudly now decode, and match the reference byte for byte.
`CAN_DataFrame.DataBytes` returns `03410b1cffffffff` where ground truth says
`03410b1cffffffff`.

Two forms are supported: payloads in the channel's own signal-data block, and —
as bus loggers actually write them — payloads as records of a dedicated channel
group interleaved with the records pointing at them. The corpus uses the second.

**The bug worth recording.** The first payload resolved correctly and every
other one came back empty. The offsets being read were `0x0C00000000000000`
where they should have been `12`: byte-reversed. `DataType::MimeSample` is not
in the little-endian list, so the offset was being read big-endian.

A variable-length channel's data type describes its *payload* — `MimeSample` for
a bus frame — and says nothing about the byte order of the offset pointing at
it. That offset is always little-endian. Reading it through the channel's
declared endianness is wrong for every VLSD channel whose payload type is not
explicitly little-endian, which is most of them.

**Representation differs from the reference, deliberately.** Where payloads have
mixed sizes, the reference pads them all out to the longest so the result fits a
rectangular array. This crate returns `SignalValues::VarBytes` and keeps each
payload at its real length: one corpus file holds 716 three-byte payloads
alongside 144,818 eight-byte ones, and padding the short ones would put five
bytes into the data that are not in the file. Where every payload is the same
size — the common case — `SignalValues::Bytes` is returned, matching the
reference. The golden test knows about the difference and checks the payload
either way.

**Performance, corrected.** Both sides now decode 326,623 samples, so the
comparison is finally like for like:

| Read | Reference | falcon_mdf | Ratio |
|---|---|---|---|
| Uncompressed | 3.69 ms | **2.09 ms** | 1.8× |
| DZ-compressed | 9.56 ms | **3.48 ms** | 2.7× |

Getting there took two fixes that mirror Phase 3: the payload index was being
rebuilt on every `signal()` call (now cached alongside the record buffer), and
it was a `HashMap<u64, _>` with one entry per sample. Both construction paths
walk their input forwards, so the offset table is already ascending — replacing
the map with a binary search over a compact array took reads from 4.41 ms to
2.09 ms. Hashing 29,000 keys cost more than searching them.

**Not done in Phase 4:** CA arrays (`file.rs` still has the `TODO`),
attachments, events, channel hierarchy, sample reduction, and MD metadata as an
XML tree. Only VLSD was attempted, on the grounds that it was the one gap with
ground truth waiting in the corpus and the one blocking real bus-logging data.

Tests 190 → **201**.

### Phase 4.2 and 4.4 verification log

Before doing more of Phase 4 I scanned the corpus for the blocks it covers:

| Block | Occurrences in the 8-file corpus |
|---|---|
| MD (metadata) | 48 |
| CC (conversion) | 35 |
| SI (source info) | 25 |
| **CA, AT, EV, CH, SR, SD** | **0** |

That decided what was worth doing. Only MD had anything to verify against, so
only MD was implemented.

**4.4 — MD metadata: done and verified.** `Mf4File::comment()` was returning 877
characters of raw XML where a caller expects a comment. It now returns the
`<TX>` element's text, and `Mf4File::metadata()` exposes the properties, with
nested `<tree>` elements flattened into paths. Checked against a corpus file:

```
Device Information/serial number    = 0BFD7754
Device Information/firmware version = 01.07.03
Device Information/hardware version = 00.03
Device Information/device type      = 0000007D
File Information/comment            = CE3 EV6;TEST2
```

Every value matches the raw XML in the file. Adds one dependency, `quick-xml`;
hand-rolling XML is how entity and attribute handling goes subtly wrong.
Metadata parsing never fails — losing a description must not lose the file — and
the original markup stays available from `Metadata::xml`.

**4.2 — CA arrays: deliberately not implemented, but no longer silent.** An
array channel's values are described by a CA block. That block was skipped, and
the parent channel left in the list — where reading it returns *the first
element* while presenting as the whole channel. That is the same silent
wrongness as B4 and B8.

Implementing CA blind was the alternative, and B7 is the argument against it: a
reversed rational-polynomial formula sat undetected precisely because no corpus
file exercised it. Writing three more unverifiable block parsers is how that
recurs. So array channels are now marked `UnreadableReason::ArrayComposition`
and reading one fails with an explanation. The channel stays in the list,
because it does exist in the file and hiding it would misrepresent the contents.

**4.3 — not attempted.** Attachments, events, channel hierarchy and sample
reduction appear in no corpus file, so nothing could be verified and nothing
would be exercised. They are additive whenever a file that uses them turns up.

Performance is unchanged within noise (2.65 ms / 3.40 ms against 3.69 / 9.56).

Tests 202 → **214**.

### Phase 3 revisited — profiling the decode path

Returning to performance after Phase 4, with both sides decoding the same
326,623 samples. Median of 15 runs each:

| Read | Reference | Before | After | Ratio |
|---|---|---|---|---|
| Uncompressed | 3.74 ms | 2.09 ms | **1.5 ms** | 2.5× |
| DZ-compressed | 9.34 ms | 3.48 ms | **2.5 ms** | **3.7×** |

The compressed target is met. Uncompressed is at 2.5×, still short of 3×.

**Profiling first was the whole point.** The two fixes named at the end of
Phase 3 — zero-copy from the mapping, and parallel reads — were both guesses.
Breaking a read into phases showed where the time actually was:

| Phase | Time | Share |
|---|---|---|
| open | 0.087 ms | 4% |
| assemble records (the copy) | 0.182 ms | **8%** |
| decode | 1.905 ms | **88%** |

The copy that was next on the list is 8% of the cost. Eliminating it entirely
would not reach the 3× target. Decoding is where the time is, and within it:

| Kind | Time |
|---|---|
| `u8` | 0.880 ms |
| `bytes` (VLSD) | 0.683 ms |
| `u32` | 0.159 ms |
| `f64` | 0.044 ms |

**The `u8` channels were the find.** A bus log is mostly one-, two- and four-bit
fields — bus number, direction, data length, flags — and `bit_count % 8 != 0`
meant every one of them missed the strided fast path added in Phase 3. That path
required whole aligned bytes, which describes almost nothing in a CAN frame.

Generalising it to any little-endian field fitting in eight bytes once its bit
offset is counted — read the bytes it touches, shift, mask — took `u8` from
0.880 ms to 0.301 ms and `u32` from 0.159 ms to 0.064 ms. The differential test
was extended to eleven bitfield shapes taken from real frame layouts, including
a 29-bit identifier at bit offset 2, and both paths agree on all of them.

**The VLSD change did not measurably help, and is reported as such.** Reading
payload offsets by striding and resolving them with a sequential hint instead of
a binary search is algorithmically better — O(1) against O(log n) — but the
measurement moved within noise. The binary search was not the bottleneck;
copying the payloads is, and that is inherent. Kept for the asymptotics, not on
the strength of any number.

**A process note.** The VLSD edit initially failed to apply — it targeted a call
that Phase 4 had already renamed — and silently made no change. It was caught
only because the new helper showed up as dead code under `-D warnings`. The
measurement taken before that was therefore attributing the bitfield gain to the
wrong change. Edits now assert that their target exists rather than no-op.

**What is left on uncompressed.** Zero-copy is worth the 8% measured above,
taking 1.5 ms to roughly 1.4 ms — still short of 3×. The structural lever is
that nineteen channels each make a full pass over the record buffer; one pass
extracting every channel would cut that memory traffic by an order of magnitude.
That needs a group-oriented read API, which belongs with the Phase 6 review, as
does parallelising across channels — and parallelism would change the comparison
basis, since the reference is single-threaded.

Tests 214 → **218**.

### Phase 3 second pass — profiling, then a system test

Median of 21 runs each side, both decoding 326,623 samples:

| Read | Reference | Start of session | Now | Ratio |
|---|---|---|---|---|
| Uncompressed | 3.74 ms | 7.96 ms | **1.37 ms** | 2.7× |
| DZ-compressed | 9.34 ms | 32.08 ms | **2.45 ms** | **3.8×** |

Confirmed by `criterion` against its stored baseline, which reports the changes
as significant rather than noise: −10.5% on the file with variable-length data,
−4.2% and −2.9% on the others, all at p < 0.05.

**What worked, and what did not.** Three changes were made; only one clearly
paid:

| Change | Effect |
|---|---|
| Strided path generalised to packed bitfields | `u8` 0.880 → 0.301 ms, `u32` 0.159 → 0.064 ms |
| Fixed-width fast paths for byte and VLSD channels | small, confirmed by criterion |
| Sequential-hint payload lookup | **no measurable change** |

The hint replaced an O(log n) search with an O(1) check and moved the
measurement within noise. It is kept for the asymptotics, not on the strength of
any number, and this is recorded so nobody later mistakes it for a win.

**Two false starts worth recording.** Sizing the VLSD output by summing resolved
payload lengths first made things *worse* — the sizing pass repeated every
lookup. And the per-channel timing harness proved too noisy to trust: the same
channel measured 146 µs and 236 µs on consecutive runs, which is why the
conclusions above rest on `criterion` and on medians of 21, not on it. That
harness was deleted rather than kept as a misleading tool.

**Zero-copy was not attempted, deliberately.** Profiling put the copy at 8% of a
read. Eliminating it entirely would take 1.37 ms to about 1.26 ms — still short
of 3× — while requiring `Signal` to hold either a borrowed slice or an enum over
the two I/O backends. That is a type-system change to a crate whose API is about
to be reviewed, bought for a tenth of a millisecond. The README's "zero-copy"
claim should be corrected instead.

**The remaining gap is structural.** Nineteen channels each make a full pass
over the record buffer. One pass extracting every channel would cut that, but it
needs a group-oriented read API — a Phase 6 decision — and parallelising across
channels would change the comparison basis, since the reference is
single-threaded.

### Read-path system test

The existing suites each check one dimension: `golden` compares values against
an independent reference, `robustness` covers malformed input, unit tests cover
pieces in isolation. None of them exercises the reading features *together*,
which is exactly what an optimisation breaks.

`tests/read_system.rs` adds twelve tests over every corpus file: structural
consistency, every channel decoding or explaining itself, decoded type matching
the declared kind, all channels of a group agreeing on sample count, repeated
reads agreeing (which would catch a wrongly-keyed cache), the buffered and
memory-mapped backends agreeing, name lookup agreeing with iteration,
variable-length payloads resolving, validity being self-consistent, `f64`
round-tripping, and master channels never going backwards.

It found **B18** immediately: opening with `build_channels_db: false` made
`channel_count()` return 0 while `channels()` yielded 560. The count was being
read from the name index rather than from the groups, and `find_channel`,
`has_channel` and `channel_names` all failed the same way — turning what is
documented as a memory/speed trade-off into a switch that changes which channels
exist. All four now fall back to scanning the groups, so the option is purely a
performance choice.

Tests 218 → **231**.

### Phase 4.5.1 and 4.5.2 verification log

**4.5.1 — the 4 GB truncation is fixed.** `VlsdPayloads` and
`SignalValues::VarBytes` now index payloads with `usize`; there is no narrowing
cast left in either. The runtime case cannot be tested affordably — it needs
four gigabytes of payload — so the guard is a type-level one that stops
compiling if the width is ever narrowed again, and the test says so rather than
implying it proves more.

**4.5.2 — memory on large files, measured on a 416 MB file:**

| | Peak resident |
|---|---|
| Before | 1,291 MB |
| After, memory-mapped | 826 MB |
| After, buffered | **434 MB** |

Two changes account for the drop. The group buffer was being grown from a 64 MB
hint, so assembling 416 MB meant repeatedly reallocating and holding both the
old and new buffers — reserving the real total up front removes that. And
`Arc<[u8]>` was being built from a `Vec`, which copies every byte because the
reference count has to sit beside the data; `Arc<Vec<u8>>` moves it instead.

**The remaining figure is the design's floor, and mmap pays it twice.** A data
group is assembled into one buffer before its records are read, so the buffer is
about one times the file. Under the memory-mapped backend the data is *also*
resident as mapped pages — hence 826 against 434. That is now documented on
`Mf4File::open`: for large files the buffered backend uses roughly half the
memory. It is counter-intuitive, since mmap is the default and the faster
option.

**Zero-copy was built and then reverted.** Reading a single uncompressed block
straight from the mapping avoids the second copy. It was implemented, and then
removed, for one reason: **large files do not have a single block.** Writers
chunk anything large into a data-list chain — the 416 MB file is 100 blocks, and
a 112 MB one written with a 4 GB fragment size still came out as 27. Payloads
are separated by block headers, so they cannot be borrowed as one slice. The
optimisation only ever engaged on small files, where it was worth about 4%,
inside the noise band.

A correction worth recording: the revert was also prompted by a test showing the
two backends disagreeing, which looked like a correctness bug in the new path.
It was not. The test compared channels found by name, and these files carry 73
groups that each contain a `Timestamp` — it was comparing the first against the
fortieth. The code was right. The revert stands on the complexity argument
alone, not on that evidence, and the test now compares by position.

**The real fix for large files is decoding block by block** rather than
assembling the group first: memory would then be one block, a few megabytes,
instead of the whole group. That is a decode-path redesign and is left for
Phase 6, where the read API is being reconsidered anyway.

Tests 231 → **234**.

### Phase 4.5.3, 4.5.4 and 4.5.7 verification log

**4.5.3 — `#[non_exhaustive]` applied to fifteen public enums**, and
deliberately withheld from five. `Mf4Error`, `SignalValues`, `ValueKind`,
`Conversion`, `UnreadableReason`, `Mf4Version`, `DataBlock`, `IoBackend`,
`Expr`, `Func` and the rest gain it, because each has a concrete pending
addition — array values, conversion types 9–11, further unreadable reasons, more
block types.

`ChannelType`, `SyncType`, `DataType`, `ConversionType` and `CompressionType`
keep exhaustive matching. Their variants mirror a byte in the file and every
undefined code already maps to an `Unknown(u8)` variant, so a reader can match
them all and stay correct against files this version has never seen. Marking
them would cost callers a wildcard arm for no freedom gained. The reasoning is
recorded on the types themselves, not only here.

**4.5.4 — big-endian is now executed.** It had no test and no corpus file, and
its bit-offset handling was left untouched during Phase 3 because it could not
be verified.

Rather than encode a guess, the semantics were taken from the reference
implementation, the same way the reversed rational formula (B7) was settled: it
views the field's bytes most-significant first, shifts right by the bit offset,
then masks. Its handling of fields narrower than a standard width — pad with
trailing zeros, shift by `extra_bytes * 8 + bit_offset` — is arithmetically the
same as assembling only the real bytes, which is what this code does.

Thirteen tests now cover it: whole-byte widths from 8 to 64 bits, byte offsets,
bit offsets, sub-byte fields, sign extension, reads running past the buffer, the
aligned shortcut agreeing with the general path, and four at the `Signal` level
including a signed channel and a big-endian float. **All passed on the first
run.** The implementation was correct; it had simply never been run. That is a
different and better outcome than finding a bug, and it is worth distinguishing:
the surface is now verified rather than merely present.

**4.5.7 — the README no longer claims things that are not true.** It advertised
"zero-copy parsing", which was removed as unhelpful in Phase 4.5.2; claimed
signal data "can be accessed without full file loading" and that "large files
(10+ GB) are handled efficiently", when memory scales with the largest data
group; and listed conversion support that types 9–11 do not have. Its quickstart
predated `SignalValues`, `validity()` and `metadata()`.

It now leads with what the library guarantees, carries an explicit **Not
supported** section, names the two areas that are implemented but untested
against a real file, and reports the measured performance and memory figures
including the counter-intuitive one — that the buffered backend uses about half
the memory of the default on large files.

Tests 234 → **247**.

### API surface review

The types added during this work — `SignalValues`, `ValueKind`, `Metadata`,
`UnreadableReason`, `VlsdPayloads`, `RecordLayout` — had each been designed for
its own problem and never looked at together. Reviewing them as one surface
found five faults, none of which any behavioural test would have caught.

| Fault | Fix |
|---|---|
| `Metadata::len() == 0` while `is_empty() == false` for a block holding only a comment | Renamed to `property_count`; `is_empty` now means no comment *and* no properties |
| `VlsdPayloads` fully public despite being an internal index | `pub(crate)` |
| `Metadata` and `UnreadableReason` returned by public methods but not exported | Exported from the root and prelude |
| `ValueKind` and `UnreadableReason` had `name()`/`detail()` but no `Display` | `Display` added, delegating to them |
| `Signal` was the only one of these types without `Debug` | Added, summarising rather than dumping millions of samples |

Two of these are worth drawing out.

**The `len`/`is_empty` mismatch is the kind of fault only a surface review
finds.** Every behavioural test passed with it present, because each method did
what its own body intended. What was wrong was the pair, against a convention
every Rust caller assumes. `tests/api_surface.rs` now asserts the contract
directly.

**Sealing `VlsdPayloads` immediately paid for itself.** Once it was
`pub(crate)`, dead-code analysis started applying and reported that `get`, `len`
and `is_empty` were never called — `get_from` had superseded `get` during the
performance work, and nothing had noticed because a public method is never dead.
They are now `#[cfg(test)]`, which states plainly that they exist for tests. A
public API is not just a promise to callers; it is a blind spot in every tool
that reasons about what the crate actually uses.

`RecordLayout` needed no change: it is `pub(crate)`, appears in no public
signature, and stays an implementation detail.

The new suite compiles against the crate as a dependent sees it — through
`falcon_mdf::`, never `crate::` — so a type that cannot be named from outside
fails to build rather than passing quietly.

Tests 247 → **256**.

### Reviewing a parallel contribution

Roughly 800 lines implementing CA, AT, EV, CH and SR arrived in the working tree
from outside this effort, while `max_alloc` and version coverage were in
progress. It did not compile, and its own tests failed.

The decoding suites — golden, read-system, robustness, API surface — all passed
throughout, which said the existing reader was untouched. The new parsers were
the problem, and the reason nothing else noticed is that **no corpus file
contains any of these blocks**, so none of the new code ever runs on real data.

Checking each against the reference implementation's own struct formats — the
technique that settled B7 and the big-endian semantics — found four of the five
misread their block:

| Block | Reference layout | As written | Consequence |
|---|---|---|---|
| EV | `5B 3s I 2H q d` | `ev_flags` read as `u16` | every field after it shifted a byte |
| SR | 2 links, `Q d 2B 6s` | 3 links, and min/max fields that do not exist | the cycle count was actually the third link |
| AT | `2H I 16s 2Q` | reserved read as `u16` | checksum and both sizes short by two bytes |
| CA | `2B H I i I` | `ndim` as `u8`, flags as `u16`, two fields unread | dimension count and flags both wrong |

All four are corrected and covered by fixtures built from the specified offsets.
Each also carries a test naming the specific error, so reintroducing it fails
with an explanation rather than a mismatched number — for instance, that a
one-byte flags field must not shift the fields after it.

Two things were removed rather than fixed. `SampleReductionInfo` described a
maximum, a minimum, a reduction kind and a comment, none of which an SR block
contains, and nothing ever constructed it — a public type that could not be
obtained, describing fields that do not exist. `SrType` enumerated reduction
kinds from the same misreading.

**The lesson stands unchanged.** This is precisely what the plan predicted for
format code written without a file to test it against, and it is why 4.2 and 4.3
remain marked partial: the parsers now agree with the reference's field layouts,
but nothing has read an actual CA, AT, EV, CH or SR block end to end. They
should not be trusted until something has.

Also fixed here: `Mf4Version` displayed a file spelling its version `4.00` as
`4.0`, while `4.11` and `4.20` printed in full. Now two-digit throughout.

Tests 256 → **279**.

## 8. Coverage against the ASAM MDF standard

Reference: <https://www.asam.net/standards/detail/mdf/wiki/>. Scope is **MDF
4.11 only**, per the current priority; 4.0, 4.2 and the 4.2-era blocks (LD, RI,
RV) are out of scope.

Status is measured, not assumed: "verified" means a real corpus file exercises
it and the result matches an independent reference.

### Blocks

| Block | Purpose | Status |
|---|---|---|
| ID | File identification | verified |
| HD | Header, measurement start | verified |
| MD | XML metadata | verified — comment and named properties |
| TX | Text | verified |
| DG | Data group | verified |
| CG | Channel group | verified |
| CN | Channel | verified |
| CC | Channel conversion | verified for the types the corpus uses |
| SI | Source information | parsed |
| DT | Data records | verified |
| DZ | Compressed data | verified, deflate and transposed deflate |
| DL / HL | Distributed data lists | verified |
| SD | Signal data (VLSD payloads) | verified |
| CA | Channel array | verified end to end — contiguous arrays decode to their elements |
| AT | Attachment | verified end to end against a synthetic file, embedded and external |
| EV | Event | verified end to end against a synthetic file |
| CH | Channel hierarchy | verified — layout corrected against an independent C++ implementation |
| SR | Sample reduction | verified — descriptors and reduced values |
| FH | File history | verified — creation time and tool, matching the reference |
| RD | Reduction data | verified — mean, minimum and maximum series |

### Channel types

| Type | Status |
|---|---|
| Fixed-length (0) | verified — 998 in corpus |
| VLSD (1) | verified — 41 in corpus, both storage forms |
| Master (2) | verified — 193 in corpus |
| Virtual data (6) | verified against a synthetic file — see B21; the corpus cannot check it |
| Virtual master (3) | verified against a synthetic file — see B21 |
| MLSD (5) | verified against a synthetic file — 4.9.4; no corpus file has one |
| Sync (4) | reports `Unsupported` by choice — it indexes a media stream |

**The corpus cannot verify virtual channels, and appeared to.** All 543 of them
carry a linear conversion whose factor is 0, so the sample index is multiplied
away and the channel is constant — which is exactly what the bug produced. The
earlier "matches the reference" claim was true and meaningless: the two
implementations agree on every input the corpus contains. Only a synthetic file
with a non-zero factor separates them, which is what B21's fixture is.

Worth generalising: **an oracle that agrees with you proves nothing until you
know it could have disagreed.** The corpus comparison was load-bearing evidence
for nine phases, and on this one feature it had no power at all.

### Conversions

Implemented: identity, linear, rational, algebraic, value→value with and without
interpolation, range→value, value→text, range→text, text→value, text→text,
bitfield→text — **all 12**, the last three added in 4.8.2 and 4.8.3.

### Other features

| Feature | Status |
|---|---|
| Sorted and unsorted data groups | verified |
| Unfinalized file handling | verified; the seven `id_unfin_flags` surfaced in 4.10.5, two of them compensated for |
| Invalidation bits | verified against synthetic files — per-sample bits in 4.9.2, the all-invalid channel flag in 4.10.3; no corpus file uses either |
| Structures / nested compositions | verified |
| Arrays | expanded — CN-template verified in 4.2 and 4.8.4; CG/DG-template and dynamic-size rejected. Flags, link partition and axis values corrected in 4.10.2 |
| Bus logging | frames decode as records; no DBC-level interpretation (out of scope) |
| Writing | not supported |

### Data types

| Type | Status |
|---|---|
Codes as the standard assigns them. **This table previously carried the same
off-by-one as the code it was auditing** — see B22 — which is what a coverage
table written from the implementation is worth.

| Type | Status |
|---|---|
| Integers and floats, both byte orders (0–5) | verified |
| String SBC / ISO-8859-1 (6) | decoded — variant added in 4.10.1 |
| Strings, UTF-8 and UTF-16 LE/BE (7–9) | verified against a synthetic file — codes corrected in 4.10.1 |
| Byte array, MIME sample, MIME stream (10–12) | verified — fixed-width blobs |
| CANopen date, CANopen time (13–14) | decoded in 4.9.3; synthetic fixtures only |
| Complex, both byte orders (15–16) | decoded in 4.9.3; synthetic fixtures only. A later-revision type — whether it belongs in a 4.11-scoped release is still open |
| Anything else | rejected in 4.9.1, rather than read as bytes |

### What this leaves for 4.11

**Phase 4.10, and the claim that used to stand here is the reason it exists.**
This section read "Nothing" — RD verified in 4.7, CH in 7245497, the last four
gaps closed by 4.9. Every one of those statements is still true, and the
conclusion drawn from them was still wrong, because the list was assembled from
the phase plan rather than from the standard's field tables. Reading the code
against those tables found B22–B25: a data-type enumeration off by one from code
6 upwards, a CA block whose flags and link layout are invented, an invalidation
flag parsed and dropped, and an unknown data block that reads as empty.

The generalisation is uncomfortable and worth keeping: **a coverage table is
only evidence if it was written from the specification.** §8 had carried the
same off-by-one as `DataType::from_u8` for nine phases, so consulting it
confirmed the bug instead of catching it — the documentation equivalent of the
fixture problem B20 recorded.

**Phase 4.10 is now complete**, and what it leaves is a shorter list than the
one it started from. What 4.9 closed stays closed. The items deliberately left
undone — sync channels, CG/DG-template arrays, and now dynamic-size arrays —
report accurate reasons rather than guessing, and a data block this build
cannot read fails by name instead of reading as empty.

Two things are worth stating rather than filing as done. The **version check is
not a gate**: 4.0 and 4.2 files open and read as far as their contents allow,
because a version number says which blocks a file *may* use, not which it does,
and refusing them would withhold files that use nothing this build lacks.
And an **unfinalized file is reported, not repaired**: two of the seven flags
are compensated for, the rest are surfaced so a caller can tell stale sample
counts from offsets that were never written.

The honest qualification, unchanged and now larger: everything in 4.9 and 4.10
except the per-sample invalidation bits is verified against **synthetic files
only**, because no corpus file contains any of it. Those fixtures are built from the specification and each was checked by
mutating the implementation until it failed, which is a great deal better than
the corpus agreement that hid B21 — but it is not the same as a file written by
another tool. The first real file carrying an MLSD channel, a CANopen date, a
non-ASCII string channel or an array with an axis is worth checking against
these layouts — the last two most of all, since B22 and B23 were both cases
where the fixture and the implementation shared one misreading.

A correction worth recording: an earlier reading of this list claimed virtual
and master channels were undecoded. For master channels that was wrong — 193 in
the corpus decode and match the reference. For virtual channels it was right,
and the rebuttal was mistaken: see B21 and the note above on why the corpus
agreed with a broken decoder.

### Verifying blocks the corpus does not contain

Attachments, events and file history were parsed and surfaced, but nothing had
read one from an actual file — the corpus has none. Unit tests beside a parser
prove only that it agrees with the fixture next to it; they do not prove the
reader reaches the block, follows its links, or returns what it found.

Rather than wait for a file that contains one, `tests/synthetic_blocks.rs`
builds them: a small assembler emits a valid MF4 with an identification block, a
header, and whatever blocks a test needs, patching links once their targets are
placed. The file is then read through the public API like any other.

That immediately found **B19**, and it was not a small one. Embedded attachment
bytes were read from `offset + length` — *past* the payload, because a block's
declared length already covers the data inside it. The reference reads at
`address + 96`, the fixed size of an attachment block's header, links and
fields. Reading an embedded attachment therefore returned whatever happened to
follow the block, or failed outright when the attachment was the last thing in
the file. Every unit test passed with the bug present, because none of them read
a byte of payload.

Six tests now cover: an embedded attachment round-tripping its bytes exactly, an
external one correctly reporting none, a three-link attachment chain walked in
order, an event's position derived from its base value and factor, a two-entry
history chain in order with its tool identifiers, and a self-referential
attachment chain being rejected rather than looping.

Two of my own mistakes are worth recording, since they are the reason the
approach works: the first attempt built a header block with a 24-byte data
section where the format specifies 32, and hung attachments off the wrong header
link. Both failed loudly and immediately. A fixture that has to survive the real
parser cannot quietly encode a misunderstanding the way a hand-written unit test
can.

Tests 285 → **291**.

### Arrays verified; channel hierarchy cannot be

**CA — done.** An array channel now decodes to its elements. A synthetic file
carrying a three-element `f64` array over two samples reads back as
`[1, 2, 3, 4, 5, 6]`, flattened sample by sample, with `Channel::array_shape`
reporting `[3]` from the CA block. An array whose CA block names no element
template stays unreadable, because without it nothing says how wide an element
is — decoding would be guesswork.

Only contiguous storage is decoded. Column/row storage, where elements of the
same index are grouped across records rather than within one, is reported
unreadable rather than partially decoded.

**CH — stopped, deliberately.** The channel-hierarchy block is parsed and
listed, and its layout could not be verified against anything:

- The reference implementation does not implement CH at all — no `ch_next`,
  `ch_first` or `ch_element` anywhere in its source.
- Searching the standard's public material returned the section number but not
  the field layout.
- The independent Java reader that does implement it was not reachable.

The current parser reads three fixed links and then one link per element. A
block whose purpose is *hierarchy* would ordinarily also carry a link to its
first child, and the standard describes each element as a data-group,
channel-group and channel triple rather than a single link — so there is good
reason to think it is wrong, in the same way EV, SR, AT and CA all were.

Writing a test now would pin that guess rather than check it. Given four of five
parsers in this group turned out to be misread, and that this is the one with no
reference, **the honest position is that CH is unverified and should not be
relied on.** It needs the specification text, not more inference.

Tests 291 → **293**.

### Sample reduction: the verifiable half

A channel group can carry sample-reduction levels — condensed views of its data,
one record per interval holding a mean, minimum and maximum, meant for drawing
an overview without reading every sample. Nothing read them.

Checking verifiability first, as the channel-hierarchy work taught, split the
feature cleanly:

- **The SR descriptors can be verified.** Their layout comes from the same
  reference format string that corrected `SrBlock` earlier. `ChannelGroup::
  sample_reductions` now lists each level with its record count, interval and
  synchronisation domain, verified against a synthetic file carrying two levels.
- **The reduced values cannot.** They live in reduction-data blocks whose record
  layout — how the mean, minimum and maximum triples are arranged — no
  independent implementation reads. The reference explicitly discards sample
  reduction, setting `first_sample_reduction_addr = 0` when it opens a file.

So the descriptors are surfaced and the values are not, with the type itself
saying why. A caller can learn that a file has a ten-to-one reduction at
one-second intervals; it cannot yet read those values, and it will not be handed
numbers derived from a guess.

Three tests cover it: two levels listed with the right parameters, a group with
none reporting none, and a self-referential chain rejected rather than looping.

Version bumped to 0.2.0, which `Cargo.toml` had been lagging behind the
changelog and README on.

Tests 293 → **296**.

### CH and RD, resolved by a second reference

Both had been recorded as unverifiable: the Python reference implements neither,
and the public standard material gave section numbers without field layouts. The
answer was a source that had been named in the very first conversation and never
consulted — **mdflib**, an independent C++ implementation.

**CH was wrong, in exactly the way suspected.** The parser read three fixed links
then one per element. The standard has four fixed links — next sibling, *first
child*, name, comment — and then a **triple** per element: the data group,
channel group and channel needed to locate one channel.

So the previous parser lost the tree structure entirely, shifted the name and
comment links by one, and kept only a third of each element reference while
treating it as something it was not. `ChElement` now carries the triple, and
`ChannelHierarchyNode` reports whether a node has children.

**RD's record layout is a triple of whole records.** For reduced sample `s`,
with `block` the channel group's record size:

```
mean at s * 3 * block
min  at s * 3 * block + block
max  at s * 3 * block + 2 * block
```

`Mf4File::reduced_signal` reads any of the three by striding a record of
`3 * block` at the appropriate offset — the existing decoder needed no changes,
only the right layout. Reading the wrong third returns real numbers from the
wrong series, which is worse than failing, so a test pins all three.

**The lesson is about the search, not the format.** Two features were written off
as unverifiable while a second independent implementation sat unexamined, named
in the first message of this project. "No reference exists" was really "one
reference does not cover it". Before recording something as unverifiable it is
worth asking which other implementations exist, not just whether the familiar
one has the answer.

Tests 296 → **301**.

---

## 1. Scope

Build `falcon_mdf` into a crate that a company can depend on for reading (and
eventually writing) ASAM MDF v4 measurement files, in pure Rust, with no Python
in the toolchain or the test loop.

**In scope:** MF4 (4.0–4.2) read, then write. Correctness parity with the
reference implementations, no-panic guarantees on untrusted input, and a data
path that is actually faster than the incumbents rather than nominally faster.

**Out of scope for 1.0:** MDF 3.x, DBC/ARXML bus decoding, GUI tooling,
language bindings of any kind.

---

## 2. Where the crate stood at the start

> **This section is the original baseline, kept for comparison — it is not the
> current state.** Ten of the defects it lists have since been fixed; see the
> bug register in §0.5 and the tracker in §0 for what is true now. The numbers
> below were measured against commit `8de61b9`.

### Verified correct

On sorted MF4 files (plain and DZ-compressed), output is byte-exact against the
reference implementation across all channels tested:

```
Timestamp                n=29693  min=186.90885  max=743.58405   exact
CAN_DataFrame.ID         n=29693  min=2015       max=2024        exact
CAN_DataFrame.BusChannel n=29693  min=1          max=1           exact
CAN_DataFrame.DLC        n=29693  min=8          max=8           exact
```

Bit-level extraction, byte offsets, DT/DZ/DL/HL chain traversal, zlib and
transposed-deflate decompression, linear conversion, composition-channel
expansion, and the `channels_db` name index are all sound. 115 tests pass. The
io/blocks/parser/model layering is good and does not need rework.

### Broken or missing — the gap to production

| Area | Evidence | Severity |
|---|---|---|
| Unsorted data groups | `Timestamp` returns `-1.58e300`; `CAN_DataFrame.ID` range `0–536870911` vs correct `201326603–486458183` | **Blocker** |
| Panic on malformed input | 6 crashes / 400 structural mutations (1.5%) | **Blocker** |
| Unbounded allocation | `memory allocation of 7638104968021014462 bytes failed` → process abort | **Blocker** |
| Compressed read throughput | 22.25 ms vs 9.61 ms reference — 2.3× *slower* | High |
| Uncompressed read throughput | 3.87 ms vs 3.80 ms — at parity, no Rust advantage | High |
| Channel naming | emits `CAN_DataFrame.CAN_DataFrame.ID`, should be `CAN_DataFrame.ID` | High |
| Typed output | everything coerced to `f64`; `DataBytes` (`u8[8]`) becomes `1.8e19` | High |
| Conversions 3, 6–11 | `conversion.rs:165` falls through `_ => raw`, silently wrong | High |
| Invalidation bits | `inval_bytes` parsed, never applied | Medium |
| Arrays (CA) | `file.rs:794` — `TODO` | Medium |
| AT / EV / CH / SR blocks | not parsed | Medium |
| No LICENSE files | `Cargo.toml` claims `MIT OR Apache-2.0`, neither file exists | Blocks publish |
| No CI | no `.github/` | Blocks 1.0 |
| Placeholder crate metadata | `repository` points at nonexistent `github.com/falcon-mdf/falcon_mdf` | Blocks publish |

### Performance baseline (29,693 samples × 19 channels)

| | Reference | falcon_mdf | |
|---|---|---|---|
| Open + parse metadata | 6.56 ms | 0.32 ms | **20× faster** |
| Read all, uncompressed | 3.80 ms | 3.87 ms | parity |
| Read all, DZ-compressed | 9.61 ms | 22.25 ms | **2.3× slower** |

Metadata parsing is the crate's genuine strength. The data path is not yet
competitive, and this is the single biggest gap between the crate's marketing
and its behavior.

---

## 3. Definition of done

`1.0.0` ships when all of the following hold:

1. **Correctness.** Golden-value tests pass on sorted, unsorted, compressed,
   VLSD, and array files. No silently-wrong conversions — unimplemented types
   return an error, never raw values.
2. **No panics.** 24 h of `cargo-fuzz` on the parse entry point with zero
   crashes, zero OOM aborts, zero hangs.
3. **Performance.** Full-file read is ≥ 3× the reference implementation on both
   compressed and uncompressed input.
4. **API stability.** Public surface reviewed, documented, and frozen;
   `#![deny(missing_docs)]` clean.
5. **CI.** Test + clippy + fmt + fuzz-smoke + MSRV green on Linux, macOS,
   Windows.
6. **Legal.** `LICENSE-MIT` and `LICENSE-APACHE` present; metadata accurate.

---

## 4. Phased plan

### Phase 0 — Repo hygiene (½ day) — ✅ **DONE**

Cheap, unblocks everything downstream.

- Add `LICENSE-MIT` and `LICENSE-APACHE`. The crate currently claims a dual
  license it does not ship; this blocks `cargo publish` and is a legal defect.
- Fix `Cargo.toml`: real `repository`/`documentation` URLs, drop the
  `authors` placeholder, set `rust-version` (MSRV).
- Bump `thiserror` 1.0 → 2.0.
- Add `.github/workflows/ci.yml`: `cargo test`, `cargo clippy -- -D warnings`,
  `cargo fmt --check`, on Linux/macOS/Windows + MSRV job.
- Clear the 38 existing build warnings; promote `#![warn(missing_docs)]` to
  `deny` once clean.
- Delete stray root files: `bootstrap.sh`, `install.sh`, `install.sh.sha256`,
  `export_CAN_DataFrame.csv`, `output.csv`.

**Acceptance:** CI green, zero warnings, `cargo publish --dry-run` succeeds.

---

### Phase 1 — Correctness (2–3 weeks) — *highest value*

#### 1.1 Unsorted data group demultiplexing — **the blocker**

Every CAN/LIN logger writes unsorted files. All 8 sample files are unsorted, and
all 8 produce garbage today.

The bug is a split-brain in `src/file.rs`. The counting path (`file.rs:330-430`)
correctly scans record IDs and demultiplexes. `signal()` (`file.rs:1023-1051`)
discards that work:

```rust
let raw_data = self.read_raw_data_indexed(dg)?;   // flat, all CGs interleaved
let record_size = cg.record_size(dg.rec_id_size); // one CG's stride
```

It strides an interleaved stream with a single record size, reading across
record boundaries into neighbouring channel groups' bytes.

**Fix:** during the existing record-ID scan, persist a per-CG `Vec<u64>` of
record start offsets (or a per-CG contiguous sorted buffer). Have `signal()`
gather from those offsets instead of striding.

Also remove the `raw_data.len() / record_size` fallback for `cycle_count == 0`
in the unsorted case — it currently fabricates 40,523 samples for groups that
are genuinely empty.

**Acceptance:** all 8 sample files match golden values channel-for-channel.

#### 1.2 Channel naming

Drop the duplicated parent prefix in composition expansion
(`CAN_DataFrame.CAN_DataFrame.ID` → `CAN_DataFrame.ID`). This is a
compatibility break for anyone porting existing scripts, so land it before 1.0.

#### 1.3 Typed signal values

Replace `Signal::values_f64()`-only with:

```rust
pub enum SignalValues {
    U8(Vec<u8>), U16(Vec<u16>), U32(Vec<u32>), U64(Vec<u64>),
    I8(Vec<i8>), I16(Vec<i16>), I32(Vec<i32>), I64(Vec<i64>),
    F32(Vec<f32>), F64(Vec<f64>),
    Bytes(Vec<Vec<u8>>), Str(Vec<String>),
}
```

Keep `values_f64()` as a lossy convenience wrapper. This fixes the `DataBytes`
corruption (a `u8[8]` array currently crammed into an `f64` as `1.8e19`) and is
the prerequisite for the zero-copy fast path in Phase 3.

#### 1.4 Conversions

Implement types 3 (algebraic — needs a small expression parser), 6
(`TabRangeLookup`), 7–10 (text tables → `SignalValues::Str`), 11
(`BitfieldToText`).

Critically: replace the `_ => raw` fallthrough at `conversion.rs:165` with
`Err(Mf4Error::UnsupportedConversion(..))`. Silently returning raw values for a
text-table channel is worse than failing.

#### 1.5 Invalidation bits

`inval_bytes` is parsed (`model/mod.rs:85`) but never applied. Add
`Signal::validity() -> &BitSlice` and honour `cn_inval_bit_pos`.

---

### Phase 2 — Robustness (1–2 weeks)

A library that parses untrusted files from field loggers must not crash the
host process. It currently does.

Two confirmed crash classes from 400 structural mutations:

1. **Unchecked slicing** — `src/blocks/header.rs:233`
   (`let data_section = &data[data_start..]`). Same pattern at
   `conversion.rs:255`, `channel.rs:349`, `text.rs:33`, `text.rs:74`,
   `text.rs:149`.
2. **Unbounded allocation** — a corrupt length field reaches
   `Vec::with_capacity` (`file.rs:1060`, `data_index.rs:168`) and aborts the
   process with `memory allocation of 7638104968021014462 bytes failed`. An
   abort cannot be caught; this is strictly worse than a panic.

Work items:

- Replace every unchecked slice in `src/blocks/` and `src/parser/` with
  `.get(..).ok_or(Mf4Error::Truncated)?`.
- Validate all lengths/counts against remaining file size **before** allocating.
  Add a configurable `OpenOptions::max_alloc` ceiling.
- Add cycle detection for DL/HL/CN/CG link chains — a self-referential link
  currently hangs.
- Set up `cargo-fuzz` with a seed corpus from `test_data/`; wire a 5-minute
  smoke run into CI and a nightly long run.
- Audit the single `unsafe` block (`io/mmap.rs:62`). `memmap2::Mmap::map` is
  unsound if the file is modified or truncated by another process — SIGBUS, not
  a Rust error. Document this contract on `open_mmap`, and make
  `open_buffered` the default for untrusted input.
- Add `#![forbid(unsafe_code)]` to every module except `io::mmap`.

**Acceptance:** 24 h fuzz, zero crashes/aborts/hangs.

---

### Phase 3 — Performance (1–2 weeks)

This is where the Rust advantage has to materialise. Two root causes.

#### 3.1 Data is re-read and re-decompressed per channel

`signal()` calls `read_raw_data_indexed()` fresh every time, with no caching —
O(channels × filesize). This is the entire 22 ms on compressed input.
`BlockCache` caches CC/TX/SI blocks but not decompressed data.

**Fix:** cache decompressed DG payloads behind `Arc<[u8]>` with an LRU budget.
Expose `Mf4File::read_group(dg) -> GroupData` so callers can decode many
channels from one decompression pass.

#### 3.2 Scalar per-sample decode

`values_f64()` loops `value_at(i)` → bounds check → `bytes_to_f64()` →
conversion match, one `f64` at a time. The reference implementation does a
strided vector copy.

**Fix:** add a fast path for byte-aligned, byte-multiple channels
(`bit_offset == 0 && bit_count % 8 == 0`) — `chunks_exact(record_size)` +
`from_le_bytes`, which autovectorises. Keep the current bit-twiddling path as
the general fallback. Hoist the conversion match out of the loop and specialise
per conversion type.

Additional wins: parallelise multi-channel reads with the `rayon` dependency
already in `Cargo.toml` but currently unused on this path; decompress DL block
chains in parallel.

**Acceptance:** ≥ 3× reference on both compressed and uncompressed full-file
reads, tracked by a `criterion` suite in `benches/` with CI regression alerts.

---

### Phase 4 — Format coverage (2–3 weeks)

- **CA (arrays)** — `file.rs:794` is a `TODO`. Implement at minimum
  `CaArrayType::Array` and `ScaleAxis`, exposed as `SignalValues` with a shape.
- **VLSD** — currently partial. Complete variable-length reads via SD blocks.
- **AT (attachments)** — embedded and referenced, with hash validation.
- **EV (events)**, **CH (channel hierarchy)**, **SR (sample reduction)**.
- **MD metadata** — currently a raw string (`text.rs:59`). Parse to a queryable
  XML tree with `quick-xml`.
- **Bus logging** — expose `CAN_DataFrame`/`LIN_Frame` as structured records
  (no DBC decoding; that is a separate crate).

---

### Phase 5 — Write support (4–6 weeks)

Only start once Phases 1–3 are done. Writing MDF is substantially harder than
reading; a half-finished writer produces files that other tools reject.

1. `Mf4Writer` — create from scratch, sorted single-DG output, DT blocks.
2. DZ compression on write.
3. `MDF::save()` round-trip: read → modify → write, preserving metadata.
4. Editing operations: `cut`, `filter`, `concatenate`, `resample`.
5. Round-trip conformance: every file in the corpus must survive
   read → write → read with identical values, and must load cleanly in an
   independent MDF tool.

---

### Phase 6 — API freeze and 1.0 (1 week)

- Review the public surface. Current concerns: `signal()` returns an owned
  `Vec<u8>` of the entire data group; `Channel` is cloned into every `Signal`;
  `OpenOptions` needs `max_alloc`, `validate_checksums`, `lazy` knobs.
- Consider a builder-style reader API for zero-copy iteration over records.
- `#![deny(missing_docs)]`, full rustdoc with runnable examples.
- `cargo-semver-checks` in CI.
- `CHANGELOG.md`, documented MSRV and semver policy.

---

## 5. Test strategy — replacing the reference oracle

With Python removed, correctness must be locked into Rust. Three layers:

1. **Golden-value tests.** Freeze the verified-exact values from the sorted-file
   comparison into `tests/golden/*.json`, asserted by an integration test.
   `test_data/` is gitignored, so gate on file presence and skip cleanly in CI —
   or check in a handful of small (< 1 MB) fixtures.
2. **Fuzz corpus.** `fuzz/corpus/` seeded from the sample files, grown by
   `cargo-fuzz`, with regressions checked in as unit tests.
3. **Conformance.** Test against the ASAM reference file set if licensing
   permits; otherwise generate synthetic files covering each block type,
   conversion type, and data type combination.

The `examples/bench.rs` and `examples/dump_one.rs` harnesses already in the repo
are the starting point for layers 1 and 3.

---

## 6. Sequencing

| Phase | Effort | Gate |
|---|---|---|
| 0 — Hygiene | ½ day | publishable |
| 1 — Correctness | 2–3 wk | **0.2.0 — actually usable** |
| 2 — Robustness | 1–2 wk | safe on untrusted input |
| 3 — Performance | 1–2 wk | **0.3.0 — claims are true** |
| 4 — Coverage | 2–3 wk | 0.4.0 — feature-competitive |
| 5 — Write | 4–6 wk | 0.9.0 — read/write |
| 6 — API freeze | 1 wk | **1.0.0** |

Roughly 3–4 months of focused work to 1.0; **5–6 weeks to a genuinely useful
0.3.0** that reads real logger files correctly and fast.

Phases 1 and 2 are non-negotiable and should be done in order. Phase 3 can run
in parallel with Phase 2. Phase 4 can be delivered incrementally after 0.3.0.
Do not start Phase 5 early — a broken writer is worse than no writer.

---

## 7. Immediate next actions

Ordered by how expensive it is to get wrong, not by how much work each is.

### Do now — half a day, and two of them are time-sensitive

1. **`#[non_exhaustive]` on every public enum** (4.5.3). `Mf4Error`,
   `SignalValues`, `ValueKind`, `Conversion`, `ChannelType`, `DataType`,
   `UnreadableReason`. These freeze permanently the moment 1.0 is tagged, and CA
   arrays, AT/EV/CH/SR and MDF 3.x will each want new variants. The cheapest
   irreversible mistake available.
2. **Synthetic big-endian tests** (4.5.4). The largest wholly unexecuted surface
   in the crate: no test, no corpus file. Its bit-offset handling was left alone
   during Phase 3 precisely because it could not be verified — it could be as
   wrong as B7 was and nothing would say so. Needs no new files.
3. **README and CHANGELOG** (4.5.7). The front page still claims zero-copy,
   claims conversion coverage that types 9–11 do not have, and predates
   `SignalValues`, `validity()` and `metadata()`.
4. **Publish 0.2.0.** Not 1.0 — the API is explicitly unfrozen, which is honest
   and buys room. The reason to publish now is not polish: the corpus contains
   no CA, AT, EV, CH or SR block and no version other than 4.11, which is
   precisely why 4.2, 4.3 and 4.5.5 are stalled. Users send files. Publishing is
   the only route to the data that unblocks them.

### Then — Phase 6, before Phase 5

Freezing the read API also unblocks the two things still outstanding on
performance and memory, both of which need API decisions:

- Decoding block by block instead of assembling a whole data group, which is the
  remaining lever for *both* peak memory and the unmet 3× speed target.
- A group-oriented read so channels are not each walked over the same buffer.
- `OpenOptions::max_alloc` made configurable (4.5.6).

A writer would otherwise double an API that has never been reviewed as a whole.

### Deferred, with reasons

| Item | Why it waits |
|---|---|
| 4.2 CA arrays, 4.3 AT/EV/CH/SR | No corpus file contains one. Building format parsers blind is how B7 happened. |
| 4.5.5 versions 4.0 / 4.2 | Same: every corpus file is 4.11. |
| Phase 5 write support | 4–6 weeks, and better as 2.0 against a frozen API. A half-finished writer emits files other tools reject. |
