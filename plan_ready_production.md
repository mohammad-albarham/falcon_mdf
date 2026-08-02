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
- [~] **4.2** CA arrays — not expanded, but no longer silently partial: an array
      channel is now marked unreadable and fails loudly
- [ ] **4.3** AT attachments, EV events, CH hierarchy, SR sample reduction —
      **no such block exists anywhere in the corpus**; see the log
- [x] **4.4** MD metadata parsed into comment + named properties

### Phases 5–6
- [ ] **5** Write support
- [ ] **6** API freeze and 1.0

---

## 0.5 Bug register

Every defect found so far, with how it was found and whether it is fixed. All
seventeen are now closed: ten in Phases 0–1, four in Phase 2, three in Phase 4.

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
| **B16** | `comment()` returned the raw XML of a metadata block — 877 characters of markup — instead of the comment inside it, leaving every caller to parse XML. | `blocks/text.rs` | Low | Inspecting corpus metadata | 4.4 |
| **B17** | An array channel was left in the channel list with its CA composition skipped, so reading it returned the first element while presenting as the whole channel. | `file.rs` | Medium | Corpus block scan during Phase 4 | 4.2 (now fails loudly) |
| **B15** | Variable-length payload offsets read using the channel's declared endianness. A channel's type describes its *payload*, not the byte order of the offset pointing at it, so every VLSD channel whose payload type was not explicitly little-endian resolved a byte-reversed offset — `0x0C00000000000000` for `12` — and returned empty payloads for all but the first sample. | `model/signal.rs` | High | Golden byte comparison after implementing VLSD | 4.1 |
| **B14** | `memmap2::Mmap::map` unsound if the file is externally truncated — SIGBUS, uncatchable. A safe-looking public API with an undocumented obligation, on the **default** backend. | `io/mmap.rs:62` | Medium | By construction | 2.5 |

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

Phases 0 and 1 are done. The crate now reads real logger files correctly; what
it does not yet do is survive files that are malformed or hostile. All four open
defects in §0.5 are that same problem, and all four close in Phase 2.

1. **B11 — unchecked slicing** (§2.1). Six known sites; each becomes a
   `.get(..).ok_or(...)`. Localised, mechanical.
2. **B12 — unbounded allocation** (§2.2). Validate every length and count
   against the remaining file size before allocating; add
   `OpenOptions::max_alloc` as a backstop. This is the worst of the four: an
   abort cannot be caught by a caller.
3. **B13 — link-chain cycles** (§2.3). Every `*_next` walk needs a visited-set
   or a bounded iteration count. Verified reproducible; a crafted file hangs
   forever while consuming memory.
4. **`cargo-fuzz` harness** (§2.4), seeded with the corpus and with the three
   crafted inputs that reproduce B11–B13, so these cannot regress.
5. **B14 — document the `mmap` contract** (§2.5) and make `open_buffered` the
   recommended entry point for untrusted input.

Phase 3 then addresses R1, the `open` regression accepted in exchange for
correct reads.
