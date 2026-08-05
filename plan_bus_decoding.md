# falcon_mdf — Bus decoding plan (DBC/ARXML)

Turning bus-logged MF4 files into named, physical signals: recognising the
frames a logger recorded, and decoding them against a CAN database the user
supplies at runtime.

Survey current as of 2026-08-05. Versions and maintenance status below were
read off the registry on that date and should be re-checked before work
starts.

---

## 1. What this is for

Bus logging is the dominant real-world use of MF4. A fleet logger writes CAN,
LIN or FlexRay traffic into a measurement file as *raw frames* — an identifier,
a timestamp, a payload of bytes — and the meaning of those bytes lives outside
the file, in a DBC or ARXML database. A reader that stops at the frame gives
back `CAN_DataFrame.DataBytes` as eight opaque numbers. A reader that goes one
step further gives back `EngineSpeed = 2130 rpm`.

That step is what asammdf is mostly used for, and it is the largest single gap
between this crate and the tools it is compared against.

**Non-goals.** No J1939 transport-protocol reassembly in the first pass, no
ISO-TP, no UDS or diagnostic decoding. No writing bus data. No LIN or FlexRay
until CAN is complete and correct — they share the frame-extraction half but
have their own database semantics, and doing three at once is how the frame
model ends up wrong for all of them.

---

## 2. The problem splits in two, and only one half is outsourceable

This is the finding that determines the size of the work.

**The MDF half — nobody else can do this.** Recognising that a channel group
holds bus events rather than measurements, then pulling each frame's
identifier, payload, bus channel and timestamp out of the records. It depends
on how MF4 lays bus data out, which is this crate's domain and no library's.

**The database half — fully solved elsewhere.** Parsing a DBC or ARXML file
into messages and signals, then extracting a signal's bits from a payload and
applying its scaling. The file formats have mature Rust parsers; the bit
extraction is small and well-specified.

Reusing the parsers removes roughly the file-format half of the work. It does
not remove the MDF half, and it does not remove the decoder. Budget for about
60% of a from-scratch implementation, not 10%.

---

## 3. What we already have

More than expected, from work done for other reasons.

- `CgFlags::bus_event` and `plain_bus_event` are read, and as of the 0.3.0
  flag-audit so are the 4.2 bits `remote_master` and `event_group`
  (`src/blocks/channel_group.rs`). The signal that a group is bus-logged is
  already available; nothing consumes it yet.
- `CnFlags::bus_event` likewise.
- `qualify_channel_name` (`src/file.rs`) already resolves composition channels
  such as `CAN_DataFrame.ID` and `CAN_DataFrame.DataBytes` to sensible names,
  handling the disagreement between writers that pre-qualify member names and
  writers that do not. **The frame fields are therefore reachable today as
  ordinary channels** — a caller who knows the layout can already read them.
- Bit extraction over arbitrary offsets and widths exists in
  `src/model/signal.rs` (`read_uint`, `read_int`). The primitives transfer; the
  formulas do not, because DBC's Motorola bit numbering is not MDF's.

What was missing is the layer that recognises a group as bus-logged and
assembles frames from it. That is now `src/bus.rs` — see Phase B1 below.
Everything downstream of it is still missing.

---

## 4. Dependency survey

| Crate | Version | Licence | Status | Verdict |
|---|---|---|---|---|
| [`can-dbc`](https://lib.rs/crates/can-dbc) | 10.0.0 (2026-07-15) | MIT/Apache-2.0 | Active, 23 releases, ~68.6k downloads/mo | **Use it** |
| [`autosar-data`](https://lib.rs/crates/autosar-data) | 0.22.0 (2026-06-18) | MIT/Apache-2.0 | Active since 2022 | **Use it, later** |
| [`canparse`](https://lib.rs/crates/canparse) | 0.1.4 (2019-07-29) | MIT/Apache-2.0 | Abandoned | Read it, don't depend on it |
| [`dbc-codegen`](https://lib.rs/crates/dbc-codegen) | 0.3.0 (2023-10-31) | MIT/Apache-2.0 | Experimental, inactive | Wrong shape |
| [`dbc-data`](https://crates.io/crates/dbc-data) | — | — | Embedded-focused | Wrong shape |

Every candidate is dual MIT/Apache-2.0, matching this crate's licensing
exactly. Licence compatibility is not a constraint here.

**`can-dbc` is the pick.** Pest-based grammar, three direct dependencies
(`can-dbc-pest`, `serde`, `thiserror`), and the healthiest maintenance record
in the category. Its limitation is stated plainly in its own README: it
**parses, it does not decode**. It yields message and signal definitions —
identifiers, start bits, lengths, byte order, factor and offset, value tables,
multiplexing — and stops. Turning a frame into a number is ours to write.

**`canparse` does exactly what we want and cannot be used.** It decodes at
runtime from a DBC loaded at runtime, which is precisely the shape we need. It
has had no release since July 2019, is still on Rust 2015, and depends on
`rustc-serialize` (deprecated), `nom` 4.2 and `socketcan` 1.7. Worth reading
for its approach to signal extraction; not worth building on.

**`dbc-codegen` is the wrong shape, not merely stale.** It generates Rust from
a DBC inside `build.rs` — at compile time. Our users hand us a DBC at runtime
alongside the MF4 file, so generated code cannot help. `dbc-data` has the same
problem plus a deliberate embedded-subset focus.

**ARXML is covered but costs more.** `autosar-data` models ARXML generically
across all 22 AUTOSAR 4 revisions rather than exposing a bus-shaped API, with
`autosar-data-abstraction` layered above it. Its `businfo` example extracts bus
settings, frames, PDUs and signals from an ECU extract, which is the traversal
we would need. The signal-to-PDU-to-frame indirection makes this materially
more work than DBC, which is why it comes second.

---

## 5. Plan

### Phase B1 — Frame extraction (no database) — **done**

Recognise bus-logged channel groups and expose their frames. Deliverable: an
API returning, per frame, a timestamp, an identifier, a bus channel and a
payload — with no interpretation of the payload at all.

This phase is independently useful. It makes a bus log inspectable without any
database, and it is the layer every later phase sits on.

Shipped as `Mf4File::can_frame_groups` and `Mf4File::can_frames` in `src/bus.rs`,
about 200 lines, no new dependency. `CanFrame` carries the four fields above
plus `extended`, read from the `IDE` channel. Frames are stored field-by-field
with payloads borrowed rather than copied, because one allocation per frame
would dominate the cost of reading a log of millions.

Verified in `tests/bus_frames.rs` against the CANedge corpus, eight tests over
11 non-empty frame groups. asammdf turned out not to be needed as the oracle,
because the corpus carries stronger external checks:

- The OBD2 log decodes to exactly `{0x7DF, 0x7E8}` — the ISO 15765 broadcast
  request and first ECU response — all standard 11-bit. Those numbers come from
  the protocol, not from this repository.
- The J1939 log decodes to extended identifiers only, all within 29 bits, whose
  parameter groups include EEC1 (`0xF004`) and total engine hours (`0xFEE5`).
- The multi-bus logger names each group after its bus (`CAN9_Rx`), and the
  decoded `BusChannel` field matches that number. A text block and a two-bit
  record field agreeing is two unrelated paths telling the same story.
- `tests/data/golden.json` already pinned every frame *channel*, so assembly is
  checked against readings locked down before it existed.

Each of the three load-bearing steps — identifier, payload trim, bus channel —
was mutated and the suite was watched to fail before it was trusted.

**Layouts confirmed on real files:** payloads arrive both as VLSD
(`VarBytes`, the J1939 and OBD2 logs) and as fixed byte arrays (`Bytes`, some
internal-bus groups); both are exercised. `DataLength` is what trims a padded
fixed-width payload back — the J1939 log carries 716 three-byte frames among
144 818 eight-byte ones, so a reader that skips the trim returns five bytes of
padding as data.

**`plain_bus_event` does not mean what this plan assumed.** Every CANedge group
in the corpus sets it *and* carries the full set of composition channels, so it
does not signal a reduced layout. It says only that no decoded signal channels
share the group with the raw frames. Detection therefore keys on the frame
channels being present, not on the flag — which also means a writer that omits
the bus-event bit is still read correctly.

### Phase B2 — DBC decoding — **done**

Add `can-dbc` behind a feature flag. Write the decoder: start bit, bit length,
Intel versus Motorola byte order, signedness, factor and offset, minimum and
maximum, and multiplexed signals. A few hundred lines.

Deliverable: given an MF4 file and a DBC, named physical signals with units.

Shipped as `src/dbc.rs` behind the `dbc` feature: `CanDatabase::from_path` /
`from_bytes`, and `decode(id, payload) -> Vec<DecodedSignal>`. About 240 lines
including tests. The estimate held — `can-dbc` parses and does not decode, exactly
as its README says, so bit extraction and scaling were ours to write.

~~**Pinned to `can-dbc` 7.x, not the 10.0.0 surveyed above.** 10.0 requires rustc
1.83 and this crate declares 1.80, so taking it would raise the MSRV for every
consumer in order to add a feature most of them will not enable. 7.x is nom-based
rather than pest-based and pulls `nom`, `encoding_rs` and `derive-getters`; the
decoder does not care which parser produced the definitions, so this is
revisitable whenever the MSRV moves for its own reasons.~~

**Superseded — 10.0.0 is now the dependency.** The MSRV moved for its own
reasons, exactly as the last sentence anticipated: `arxml` was requiring 1.88
regardless, so the pin was protecting a floor the crate no longer stood on. See
§9 for how that was discovered and §11 for the decision.

**Verified against the OBD2 log and an outside specification, not asammdf.**
asammdf is not installed here, and the corpus turned out to offer something
better than a value dump. The Audi log carries mode `0x41` responses for six SAE
J1979 PIDs, so a database written from the published PID table gives checks whose
answers come from ISO 15031-5 rather than from this code:

- 9337 decoded values match the published formulas exactly — `(A*256+B)/4` for
  engine speed, `A-40` for coolant, and so on — computed in the test from the raw
  bytes rather than taken from the decoder.
- **Run time since engine start tracks the file's own clock.** PID `0x1F` is a
  16-bit big-endian counter of seconds; the MF4 master channel counts the same
  seconds independently. They advance together to within 1% over 556 s. This is
  the check a wrong decoder cannot pass: byte-swapping that field turns 167
  seconds into 42752.
- Every decoded range is physically plausible: 808–3586 rpm, 49–99 °C coolant,
  6–66 km/h, 11–83% throttle.
- Multiplexing selects exactly one signal per PID, and the multiplexor's own
  value equals the PID byte.

Mutating the big-endian bit order and the multiplexor selection each fail the
suite, so both are load-bearing rather than incidentally correct.

**Not covered:** J1939 source-address matching (a J1939 DBC keys messages by PGN,
while this matches the whole 29-bit identifier), extended multiplexing
(`SG_MUL_VAL_`), and value-table decoding to text. None is needed by the corpus,
and each would be guesswork without a file that exercises it.

### Phase B3 — ARXML — **done**

`autosar-data`, same decoder, different database front end. Only after B2 is
correct against real files.

That "same decoder" turned out to be the design point worth insisting on. B2 first
shipped with the decoder built around `can_dbc::Signal`; adding ARXML meant either
duplicating it or lifting it out. It was lifted into `src/candb.rs` as a
front-end-neutral `SignalDef`/`MessageDef`/`CanDatabase`, so `src/dbc.rs` and
`src/arxml.rs` are now nothing but mappings onto it. One decoder, two file formats,
and the bit-extraction tests cover both.

`CanDatabase::from_arxml_path` walks CAN and J1939 clusters through the chain the
plan predicted would be the expensive part, and it was: identifier and addressing
mode on the frame triggering, name and length on the frame, start position and
byte order on the signal-to-PDU mapping, width on the I-signal, signedness on its
base type, and factor, offset and unit on the system signal's compu method. Six
elements to assemble what a DBC writes on one line.

**Verified against cantools**, an independently written reader, using
`system-4.2.arxml` from its own test corpus (fetched by
`scripts/fetch_arxml_files.sh`; the file is gitignored like the rest of the
corpus). `tests/arxml_database.rs` transcribes cantools' assertions about that
file, which is the same standard `reference.rs` holds measurement decoding to.

It found three defects that nothing else would have, all silent:

1. **`PACKING-BYTE-ORDER` and `CAN-ADDRESSING-MODE` are AUTOSAR enums.**
   `CharacterData::string_value()` returns `None` for an enum rather than failing,
   so reading them as text reported *every* signal little-endian and *every*
   message standard-addressed. This is the same shape of defect as the 0.3.0 flag
   audit: a value that decodes to something plausible and wrong.
2. **A `SCALE_LINEAR_AND_TEXTTABLE` compu method puts a text-table scale first**,
   with no rational coefficients. Taking the first scale found no factor and
   reported `signal6` unscaled — 1 instead of 0.1.
3. **A unit's `SHORT-NAME` is an identifier, its `DISPLAY-NAME` the symbol.**
   Reading the former gave `wizepoo` where the unit is `wp`.

One deliberate divergence is recorded at the assertion: cantools reports no unit
for a `UNIT` whose display name is `NoUnit`; this reports `NoUnit`, because the
file does name a unit and encoding another reader's special case is not the same
as reading the file.

**Not covered:** multiplexed PDUs. A multiplexed message's dynamic parts are left
out rather than reported, since which part applies is chosen by a selector field
this build does not resolve, and reporting all of them would hand back signals
that occupy the same payload bits as though they were simultaneously present. The
static part is read. `MultiplexedMessage` in the fixture therefore comes back with
no signals where cantools reports six, and a test pins that so it cannot change
silently.

---

## 6. Dependency posture

**Feature-gated, off by default.** This crate currently has seven direct
dependencies and that restraint is a feature. `can-dbc` brings `serde` and a
Pest grammar; someone reading plain measurement files should not pay for it.

```toml
falcon_mdf = { version = "0.3", features = ["dbc", "arxml"] }
```

Phase B1 needs no new dependency at all and should not be gated — frame
extraction is MF4 reading, and belongs in the default build.

**As shipped.** Two features, `dbc` and `arxml`, both off by default. The decoder
and the database types in `candb` are in the default build because they need
nothing external; only the two parsers are gated, and enabling one does not pull
in the other. Direct dependencies stay at seven for a default build, and rise to
eight with either feature or nine with both.

---

## 7. Sequencing: this goes behind streaming

Bus logs are the *largest* files this crate will ever be handed — hours of
traffic from several buses. `build_records` (`src/file.rs`) currently
materialises an entire channel group contiguously before anything can be read,
and the record cache bounds retention rather than peak, keeping an oversized
entry so a caller always gets a value back.

Shipping bus decoding onto that foundation means shipping a feature that
cannot open the files it exists for. Block-by-block decoding — already the
pre-1.0 item in `CHANGELOG.md` — comes first.

**Decided 2026-08-05.** B1 shipped ahead of streaming, as §6 allows: it adds no
dependency and reads frames through the existing record path, so it neither
worsens the memory profile nor commits the API to anything. B2 waits for
block-by-block decoding, which is the next piece of work.

**Done, and this section's premise needed two corrections along the way.**
`Mf4File::signal_chunks` (`src/stream.rs`) reads a channel a bounded window of
stream at a time. Verified against the eager path over all 2085 readable channels
of both corpora — 318 of them byte-valued and compared payload by payload, 76
spanning several chunks.

Two things this plan had not accounted for:

1. **Every bus log in the corpus is an *unsorted* data group** whose payload
   channel is variable-length. Chunking only sorted, fixed-length channels — the
   obvious first pass — turned out to decline the entire bus corpus, which is to
   say all the files streaming exists for. Both are now handled: unsorted records
   are demultiplexed per window, and a payload group's payloads are indexed per
   chunk with offsets continuing across chunks.
2. **Chunking per data block bounds nothing here.** Each CANedge log is a single
   large `DT` block, so a block-granular reader holds the whole stream — the
   exact failure the plan warned about, reintroduced one level down. Windows are
   therefore bounded by bytes, and uncompressed blocks are read a range at a time
   rather than whole.

Both were caught by mutation: dropping the payload-offset carry passed the suite
under block granularity and failed it under byte windows, which is what showed
the first implementation had never exercised the carry at all.

Still refused, and refused by name: a variable-length channel whose payloads live
in its own signal-data block rather than a companion group. No corpus file has
one, so it is unimplemented rather than untested. The unsorted block-boundary
rule is likewise unexercised — no unsorted file in the corpus spans two blocks —
and mirrors `index_records` by construction; both are noted at the code.

---

## 8. Open questions

- ~~**Is `plain_bus_event`'s reduced layout documented well enough to implement
  from the standard alone, or does it need a reference file?**~~ Settled by the
  corpus: there is no reduced layout to implement. See Phase B1 above.
- **Do we need J1939 transport reassembly for the first release?** Heavy-duty
  logs are a large share of the audience and multi-frame parameter groups are
  common there. Currently a non-goal; revisit after B2. **Now tracked as T5 in
  §10** — still open, but the audience argument has only got stronger now that
  identifier matching (T3) turns out to block J1939 DBCs anyway.
- ~~**Should decoded bus signals appear in `channels()` alongside measurement
  channels, or in a separate namespace?**~~ **Settled: a separate namespace.**
  `Mf4File::decode_bus` returns `BusSignals`, and nothing decoded appears in
  `channels()`. Decoded signals are derived, their existence depends on a database
  the file does not contain, and folding them in would make `channel_count()`
  depend on an argument. Shipped as T2 below.

  Implementing it added a third reason the survey had not anticipated: a series
  cannot be identified by signal name, because two messages may spell one, nor by
  identifier, because a multi-bus logger carries the same identifier on separate
  buses. `channels()` has no room for that key.

---

## 9. Blockers

B1, B2 and B3 are all shipped and verified, so nothing blocks the plan itself.
What follows is what blocks *trusting* it, split by whether it is fixable with
work or waiting on a file.

### ~~One thing is actually broken: CI does not build any of this~~ — fixed

`.github/workflows/ci.yml` ran `cargo test --all-targets` and
`cargo clippy --all-targets` with **no features enabled**, so `src/dbc.rs`,
`src/arxml.rs`, `tests/dbc_decoding.rs` and `tests/arxml_database.rs` were not
compiled on any push. Fixed by T1: the test job now has a feature axis
(default and `--all-features`) across all three platforms, and the lint job runs
clippy both ways.

**The fix caught a defect on its first run**, which is the argument for having
done it first. `Mf4File::decode_bus` is in the default build, but its doc example
called `CanDatabase::from_dbc_path`, which exists only behind `dbc`. No job had
ever compiled a default-feature doctest against a bus API, so it failed the
moment one did.

**And the MSRV job could not be extended as planned.** T1 called for
`--features dbc,arxml` on rustc 1.80. `dbc` builds there and now does so on every
push, which is the whole point of the `can-dbc` 7.x pin. `arxml` cannot:
`autosar-data` 0.22 is itself `edition = "2024"`, which requires rustc 1.85, and
no lockfile pin changes that. So the crate's declared 1.80 covers the default
build and `dbc` only.

*(Correction, made when the decision below was taken: 1.85 is the edition
requirement, not the real floor. `autosar-data-specification` uses let-chains,
stable in 1.88, so the true number is 1.88 — see the settlement below. The
finding above was right that `arxml` broke the declared MSRV and wrong about by
how much, because it reasoned from the manifest instead of building.)*

That is worth stating plainly, because it undercuts a decision recorded in §4:
`can-dbc` was held at 7.x specifically to avoid raising the MSRV to 1.83, and
then `arxml` raised it past 1.83 anyway. The restraint is real for anyone not
enabling `arxml` and void for anyone who does. It is now documented at the
dependency, in the README and in `CHANGELOG.md` rather than left to be discovered
at a consumer's build. Revisiting means either an `autosar-data` old enough to
predate edition 2024 — an older ARXML model — or accepting the higher number as
the crate's MSRV and taking `can-dbc` 10.x while we are there.

**Settled: 1.88 is the MSRV, and `can-dbc` is at 10.x.** The second option was
taken. The declared number now covers every feature — CI builds `--all-features`
on 1.88 — rather than covering the default build and contradicting one feature.

**And the number is 1.88, not the 1.85 this section asserted twice.** Taking the
decision meant building on the toolchain rather than reading manifests, and the
manifest answer was wrong: edition 2024 is what `autosar-data` *declares*, but
`autosar-data-specification` *uses* let-chains, which stabilised in 1.88. Neither
crate declares a `rust-version` at all, so cargo has nothing to report and the
floor is reachable only by compiling — 1.87 fails with `E0658`, 1.88 succeeds.
This is the same lesson as the transcription error recorded in §10: the claim
that survived was the one nothing had executed.
The upgrade cost eleven mechanical lines in `src/dbc.rs`, where 10.x replaced
`derive-getters` accessors with public fields, and returned two things:

- `ValDescription::id` is an `i64` rather than an `f64`, so the filter that
  dropped non-integral `VAL_` keys is gone. It was guarding against a shape the
  grammar admitted and the format does not; the newer parser fixes that at the
  source.
- `SG_MUL_VAL_` extended multiplexing is parsed and exposed as
  `Dbc::extended_multiplex`. This crate still does not map it — but it is now a
  mapping question rather than a parser limitation, and it has an API consequence
  recorded in §11.

`[package.metadata.docs.rs] all-features = true` is added, so a published build
documents both gated modules.

### The rest is missing test data, not missing work

Each of these is currently handled by refusing or by documenting, never by
guessing, which is why none of them blocks a release.

| Missing | What it leaves unverified |
|---|---|
| An ARXML with a resolvable multiplex selector | Multiplexed PDU dynamic parts, left out by design |
| A multi-block **unsorted** MF4 | `stream.rs`'s block-boundary carry rule, which mirrors `index_records` by construction |
| A public J1939 DBC (the SAE one is paid) | J1939 decoding end to end; OBD2 stood in for it |
| asammdf in the local environment | The oracle this plan originally specified for B2; SAE J1979 stood in, and is arguably better |
| A big-endian MF4 file, and any 4.0 / 4.2 file | Pre-existing gaps, already stated in the README |

The pattern from §5 is worth restating: every defect this work found was found by
an *outside* answer — the ISO 15765 identifiers, the J1979 formulas, the engine's
own clock, cantools' assertions. None was found by a fixture. Acquiring the files
above is therefore worth more than any amount of additional self-consistent
testing.

---

## 10. What is left before this is a tool

Ordered by who is affected, not by effort. T1 is config; T2 is the one that
decides whether the feature gets used at all.

**T1–T4 are done.** T5 and T6 remain, both post-freeze and both additive.

### T1 — Make CI build the features · **done**

Feature axis on the test job (default and `--all-features`, all three platforms),
clippy both ways on the lint job, `--all-features` on the MSRV job, and
`[package.metadata.docs.rs] all-features = true`. It found a broken doc example
on its first run, and it established that `arxml` cannot hold the 1.80 MSRV at
all — both recorded in §9, and the second is what moved the MSRV to 1.88.

### T2 — Decoded signals as time series · **done**

`Mf4File::decode_bus(&database)` returns `BusSignals`: one `BusSignal` per
signal, with every reading and its timestamp. Open question #3 is settled in §8 —
a separate namespace.

**A series is keyed by bus, message and signal together**, which is the part the
plan did not anticipate. The obvious key is the signal name, and it is wrong twice
over: two messages may define one name, and a multi-bus logger carries the same
identifier on separate buses, where merging would interleave readings from
unrelated networks with nothing downstream able to tell.

Verified by building the same result the other way. `decode_bus` is an
accumulation loop the caller would otherwise write, so `tests/bus_signals.rs`
writes that loop and compares: **74 489 readings across three corpora agree
reading for reading**, over both matching modes. The properties that make it a
series rather than a bag of numbers — order, parallel vectors, multiplexed
signals carrying only the frames that selected them — are pinned separately.

The bus-collision case is the one the corpus cannot reach: no file carries an
identifier on two buses. The accumulator is therefore separated from file reading
so it can be driven by frames built in the test, and dropping the bus from the key
fails that test. Same for dropping the message index.

### T3 — J1939 identifier matching · **done**

`CanDatabase::with_matching(IdMatching::J1939Pgn)`. Exact matches win; the
parameter group is consulted only when no message carries the identifier itself,
so enabling it cannot change a message that already matched.

**The corpus turned out to demonstrate the defect exactly.** The truck log
transmits EEC1 on `0x0CF00400` *and* `0x0CF00421` — two ECUs — while a J1939 DBC
keys it as `0x0CF004FE`, the null address, which appears nowhere in the file.
`exact_matching_decodes_a_real_j1939_log_to_nothing` asserts that failure on the
real log before the rest show PGN matching fixing it, and finding the group from
both ECUs is the case a "pin the address you saw" workaround cannot handle.

The parameter groups are checked against the numbers published in SAE J1939-21
and J1939-71, not against this code. The PDU1/PDU2 distinction is the part worth
having tested: below PDU format 240 the low byte of the group is a *destination*
address and is no more part of the group than the source is, and dropping that
distinction fails the suite.

Decoding is checked the way B2 was — against outside answers. Engine speed
computed from the raw bytes per J1939-71 matches over all 21 542 EEC1 frames and
ranges 913–1762 rpm; total engine hours reads 8596.25 h and only ever increases.

### T4 — DBC value tables decoded to text · **done**

`DecodedSignal::text` and `BusSignal::text_at`. Looked up against the raw value
*after* sign extension and *before* scaling, which is what a `VAL_` entry names —
reading it unsigned turns a gear of `-1` into `255` and loses the label, and
that mutation fails the suite. A value the table does not cover is left
unlabelled rather than mislabelled.

Both changed public types (`DecodedSignal` gained `text`, `SignalDef` gained
`value_table`), which is why this went before the freeze.

ARXML contributes no tables: a `TEXTTABLE` compu method is its equivalent, and
this build reads only the linear part of a compu method. Left empty rather than
guessed, and noted at the code.

### A note on how these were tested

One error is worth recording because nothing in the suite caught it. A DBC spells
identifiers in decimal, and a hand-converted `0x98FEE5FE` came out as
`0x98FEEFFE` — a different real parameter group, which this truck also broadcasts.
Every structural test still passed: both decode paths agreed with each other,
the series were ordered and parallel, the counts were stable. Only asking what the
number *meant* caught it, when total engine hours came back as 78 million.

The fixture now builds its identifiers from constants rather than decimal
literals, and `the_truck_series_are_the_ones_the_database_names` pins the physical
range. This is the same lesson as §9's closing paragraph, arriving from the
inside: self-consistency is not correctness, and two agreeing implementations of
the wrong thing agree.

### T5 — J1939 transport reassembly · post-freeze

Multi-frame parameter groups (TP.CM / TP.DT) are common in heavy-duty logs and
are currently undecodable: each frame decodes on its own and the reassembled
message never appears. A non-goal in §1 and still the right call for a first
release, but it is the largest remaining functional gap after T2.

### T6 — LIN and FlexRay frames · post-freeze

The truck logs in the corpus already contain a `LIN_Frame` group that
`can_frame_groups` correctly declines. §1 deferred these until CAN was complete
and correct, which it now is, so the reason to wait has expired — what remains is
that they have their own database semantics.

### Smaller, decided deliberately

- **`CHUNK_BUDGET` is a fixed 4 MiB** (`src/stream.rs`). Fine as a default, but a
  tool processing multi-gigabyte logs on a constrained machine may want it set.
  Left fixed on purpose; revisit if anyone asks, and note `OpenOptions::max_alloc`
  in `plan_ready_production.md` §7 wants the same treatment.
- ~~**No example binary for bus decoding.**~~ Added: `examples/decode_bus.rs`
  takes an MF4 and a DBC, prints frame count and then every decoded signal with
  its reading count, first value and range, and takes `--j1939` for the matching
  mode. Gated on the `dbc` feature via `required-features`.

---

## 11. Immediate next actions

T1 through T4 are done, along with the `decode_bus.rs` example. What is left:

1. **Freeze the API.** This is now the only thing standing between here and 1.0
   on the bus side. Everything that was going to change a public type before the
   freeze has changed it: `DecodedSignal` gained `text`, `SignalDef` gained
   `value_table`, and `BusSignal`/`BusSignals`/`IdMatching` are new. T5 and T6 are
   additive and can land after.

   One thing to settle while reviewing: `BusSignals::find` returns a `Vec` because
   a name does not identify a series. That is honest but awkward at a call site,
   and it is the kind of decision worth looking at once more before it is fixed
   for good.

2. ~~**Decide the `arxml` MSRV question** (§9).~~ **Decided:** 1.88 is the MSRV
   and `can-dbc` is at 10.x. Recorded in §9, along with a correction: the floor
   is 1.88 rather than the 1.85 previously documented.

   It leaves one thing behind for the freeze, which is why it was done first.
   `can-dbc` 10.x parses `SG_MUL_VAL_`, and extended multiplexing does not fit
   the type this crate has: a selector there is a *range* (`min-max`, several per
   signal) and names the multiplexor it belongs to, which is what makes nested
   multiplexing expressible. `Multiplexing::Selected(u64)` holds one value and no
   multiplexor name. So the choice is to widen the variant now or to accept that
   extended multiplexing is permanently out of reach for 1.x. Supporting it is
   not being proposed here — only that the shape be chosen deliberately rather
   than inherited from a parser the crate no longer uses.

3. **T5 and T6**, post-freeze, in that order.

Acquiring files stays orthogonal and worth doing whenever the chance appears: a
J1939 DBC, a multi-block unsorted log, an ARXML with a real multiplex selector.
Each converts a documented limitation into either a verified feature or a found
defect, and on this plan's record it is usually the latter — T3's work is the
newest evidence, where a real truck log turned a documented gap into a
demonstrated failure and then a verified fix.

`plan_ready_production.md` §7 carries the master ordering; its entry for the API
freeze now points here.
