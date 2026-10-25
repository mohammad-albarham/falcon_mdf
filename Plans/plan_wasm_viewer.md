# Wasm viewer → native-GUI parity (and beyond)

Tracking plan for `wasm/demo/` vs the native egui viewer (`gui/`).
Living document: tick items as they ship, update the status table, keep the
gates green. Local-only by repo convention (`plan_*.md` is gitignored); move
it under `docs/` or mirror into GitHub issues/milestones if you want it shared.

- Viewer code: `wasm/demo/{main.js,worker.js}` + `wasm/src/lib.rs` (bindings)
- Live site: https://mohammad-albarham.github.io/falcon_mdf/ (auto-deploys on
  merge to `main` via `.github/workflows/pages.yml`)
- Baseline shipped: `006ecde` (worker decoding, Rust-side decimation, overlay
  plot, cursor readout, windowed CSV)

## How to track

- Status per item: `[ ]` todo · `[~]` in progress · `[x]` done (with commit)
- Size: S ≤ half a day · M ≤ 2 days · L ≥ a week (or needs design first)
- An item is **done** only when its *Definition of Done* passes: native tests
  where Rust is touched, the browser checklist for UI, and the full gate (§
  Verification gates) stays green.
- One phase at a time; within a phase, items are independent unless noted.

## Status table

| Phase | Theme | State |
|---|---|---|
| 0 | Core loop (worker, decimation, overlay, cursor, CSV) | **done** (`006ecde`) |
| 1 | Plot interaction parity | **done** (`b5f87c7`) |
| 2 | Data coverage (strings, tables, search, details) | **done** (`112245e`, `4d1da6b`) |
| 3 | Formats & bus (v3, DBC, GPS) | **done** (`112245e`→`00ae8e8`; 3.5 deferred, see item) |
| 4 | Beyond the GUI (X-Y, computed, compare, share, huge files) | **done** (`ed1bd45`→HEAD; 4.6 design-first, see docs/wasm-streaming-design.md) |

> **Round complete 2026-09-04 — MERGED to main.** Phases 2–4 executed on
> `feat/wasm-viewer` (worktree `fmdf-wt/wviewer`) and fast-forward merged
> into main as `fa57f61` (2026-09-04): 2.1–2.5, 3.1–3.4, 4.1–4.5, 4.7
> shipped and browser-verified; 3.5 ARXML deferred with rationale (needs a
> core bytes-based AUTOSAR loader); 4.6 delivered as a design gate
> (`docs/wasm-streaming-design.md`), implementation pending its spike.
> Wasm size 394 KB → 783 KB (guide < 2 MB). Wasm crate tests 32 → 85, all
> green; fmt/clippy/core/doc/GUI gates green. Browser checklist re-run on
> the final build (points 1–9 pass; 10 = live-site spot check pending the
> Pages deploy of this merge — push not done, owner's call).
> Note: main's checkout still carries the uncommitted optimization-round
> WIP (src//gui//benchmarks/); the two CHANGELOG Unreleased sections
> (viewer Added + optimization Changed) are combined and staged for that
> round's commit.

> Phase 1 browser checklist run 2026-08-30 against the worktree demo: all ten
> points pass (sample load, overlay+legend, zoom/pan/reset timing, readout,
> CSV row parity 101=header+100, 8-channel cap message, garbage-file recovery,
> NaN gaps on Boost, worker-only wasm verified via main-thread resource list);
> live-site spot check deferred to merge time (Pages deploys from `main`).
> Wasm size after Phase 1: 394,312 bytes (~385 KB) — well under the 2 MB guide.

## Parity matrix (GUI feature → viewer)

| Native GUI (`gui/src/`) | Viewer today | Phase |
|---|---|---|
| Plot: stacked per-channel lanes, X-linked | both modes, session-persisted | 1.1 ✅ |
| Plot: two cursors + region statistics | A/B cursors + region stats (`signal_stats`) | 1.2 ✅ |
| Plot: drag-to-zoom region | shift-drag rubber band | 1.3 ✅ |
| Time axis: relative / absolute switch | both, adaptive absolute labels | 1.4 ✅ |
| Stats panel (`panels/stats.rs`) | stats strip (window) + region figures; label distributions for text channels | 1.7 ✅ |
| Keyboard + touch navigation | arrows/zoom/Home/1–8, pinch | 1.5/1.6 ✅ |
| Channel search: pattern/regex (`search.rs`, `search_channels`) | contains/wildcard/exact via the reader index + platform regex in the worker | 2.3 ✅ |
| Table view (`panels/table.rs`) | virtualized index-paged table; numeric/text rows from the raw cache, array rows list elements, byte rows as hex | 2.2 ✅ |
| String / value↔text conversion channels | kind metadata (`channel_kind`), `signal_text` label windows, state bands, label readout + stats | 2.1 ✅ |
| Details / tree / blocks / hexdump panels | channel details panel (kind, unit, group, samples, data type, shape, conversion rule); structure tree (`structure()` + sidebar Structure tab, 4.8); blocks/hexdump deliberately parked | 2.5 ✅ (scope: details), 4.8 ✅ (tree) |
| MDF v3 reader (`mdf3/`) | v3 (and 2.x) opens transparently via signature sniff; every endpoint format-agnostic | 3.1 ✅ |
| Bus decoding + bus panel (`bus.rs`, `dbc.rs`) | DBC attach decodes signals into ordinary channels; CAN/LIN frame panel with two-way cursor link. ARXML decode deferred (3.5) | 3.2/3.3 ✅ |
| GPS panel (`gps.rs`) | canvas track, equirectangular projection, speed coloring, two-way cursor sync | 3.4 ✅ |
| X-Y mode (`xy.rs`) | X-Y panel; Y sampled at X's timestamps, stride-limited, auto-scaled axes | 4.1 ✅ |
| Computed channels (`computed.rs`) | Rust expression evaluator (`[Channel]`, + - * /, abs/sqrt/min/max); computed channels behave as ordinary channels everywhere | 4.2 ✅ |
| Two-file compare (`SeriesKey::File`) | worker holds extra files; second file's channels overlay as `@label::channel`, readout shows both | 4.3 ✅ |
| Background loading (`loader.rs`, `job.rs`) | worker already non-blocking | — (done) |
| Per-pixel min/max decimation (`decimate.rs`) | ported (first/min/max/last); GPS tracks stride-decimated, label runs state-collapsed | — (done) |

"Better than the GUI" opportunities unique to the web are Phase 4. Shipped:
shareable view URLs (4.4), no-install sharing (always was), offline
persistence via OPFS (4.5). Outstanding: OS-level file streaming beyond wasm
memory limits (4.6 — design in `docs/wasm-streaming-design.md`, implementation
pending its spike).

---

## Phase 1 — Plot interaction parity (make it the daily driver)

- [x] **1.1 Stacked lanes mode** — M (`b5f87c7`)
  Toggle overlay ↔ vertically stacked per-channel lanes sharing the X axis
  (the native GUI's default; honest for mixed units). Data path exists; it is
  a draw-mode + per-lane Y-scale change in `main.js`.
  DoD: mode toggle persists per session; zoom/pan/cursor work in both modes;
  screenshot check on CANedge log with 4 mixed-unit channels.
  *Shipped: segmented Overlay/Stacked control (`sessionStorage["plot-mode"]`),
  per-lane Y scales + name/unit labels, lanes ≥64 px then the wrap grows;
  browser-verified on sample.mf4 (3 mixed-unit lanes) — CANedge 4-channel
  screenshot still pending (needs Phase 3 DBC file, see 1.1 note in 3.2).*
- [x] **1.2 Two cursors + region stats** — M (`b5f87c7`)
  Shift-click (or long-press) sets cursor B; between A and B show per-channel
  min / max / mean / Δt / Δ(value). Compute in Rust (new binding, additive:
  `signal_stats(name, t0, t1)`); the reader already has `cut`/statistics
  primitives — don't recompute per pixel in JS.
  DoD: native tests for `signal_stats` (window edges, empty window, NaN
  handling match `decimate_window`); browser check vs GUI values on the same
  file.
  *Shipped: `signal_stats(&mut self, name, t0, t1)` (note: `&mut self`, not
  `&self` — shares the decode LRU with `signal_window`); JSON
  `{count,invalid,min,max,mean,first,last,t0,t1}`, window edges INCLUSIVE
  matching `decimate_window`, 8 new tests in `window.rs` (18→26 in that file,
  32 total in the crate). Oscilloscope cursor cycle A→B→clear; region stats
  one worker round-trip per channel per region. Browser-verified hand-exact
  on Time channel (samples 2.6–5.8 → min 2.6 max 5.8 mean 4.2 Δ 3.2).*
- [x] **1.3 Drag-to-zoom** — S (`b5f87c7`)
  Drag with a modifier (e.g. ⇧ drag or a toolbar toggle) selects a time region
  and zooms into it; ordinary drag stays pan. Reuses the existing debounce.
  DoD: browser checklist; no conflict with pan on touch.
  *Shipped: shift locked at pointerdown, rubber band drawn, <8 px ignored;
  browser-verified [2.0,6.2] from a 25%→65% band = 43 samples exactly.*
- [x] **1.4 Absolute / relative time axis** — S (`b5f87c7`)
  `info().start_time` exists; render X ticks as absolute datetimes (with a
  toggle), keep internal math in seconds. Mind the CANedge log: master is
  epoch seconds — display relative-to-start AND wall-clock variants.
  DoD: tick correctness across DST-free formatting (`Intl` is fine — it is
  local, not a CDN); screenshot check.
  *Shipped: Relative/Absolute segmented toggle (`sessionStorage["time-mode"]`),
  `Date.parse` of ISO start with graceful fallback; adaptive label resolution
  (date at ≥day steps, clock+seconds below, sub-second `.mmm`/`.µs`),
  measured label thinning, corner tag shows view-start date + "local";
  readout carries cursor wall-clock. Browser-verified against the known
  start time to the millisecond.*
- [x] **1.5 Keyboard navigation** — S (`b5f87c7`)
  ←/→ pan, +/- zoom, Home reset, 1–8 toggle overlay slots, Esc clears cursors.
  DoD: browser checklist; focus handling does not fight the channel filter.
  *Shipped: canvas `tabindex=0` + focus ring, keydown on the canvas only;
  panBy/zoomAtT/resetView extracted as the single view-mutation path.
  Browser-verified: 1.25× zoom = 80 samples exactly, ±10% pan, Home reset.*
- [x] **1.6 Touch / pinch zoom** — S (`b5f87c7`)
  Pointer-events are already used; add two-pointer pinch (the `touch-action:
  none` is set). DoD: manual tablet check or Chrome device-mode checklist.
  *Shipped: pointer-pair tracking with re-basing, gesture taint (a pinch
  never places cursors), one-finger hand-back to pan. Verified via synthetic
  two-finger pointer events: 3× spread → exactly a 3.3 s window (33 samples);
  gather zooms out. Real-tablet check still worth one manual pass.*
- [x] **1.7 Stats strip** — S (`b5f87c7`)
  Samples / min / max / mean / invalid-count for the *selected* channel over
  the visible window (fold into `signal_stats` from 1.2 — same call).
  DoD: matches the old demo's stats on sample.mf4; NaN/invalid counted.
  *Shipped: `#statsbar` under the toolbar; while a region is live it shows
  region figures instead. Verified: Speed matches the old demo exactly;
  Boost now honestly reports 80 valid + 20 invalid (old demo counted 100).*

## Phase 2 — Data coverage (stop lying about unplotable channels)

- [x] **2.1 String and text-conversion channels** — M
  `SignalValues::Str` and value↔text conversions currently go through
  `to_f64()` → NaN. Extend the binding (additive) with a kind field and, for
  text channels, the labels themselves (e.g. `signal_arrays` gains
  `kind:"f64"|"text"|"bytes"|"array"`; text returns a parallel string array).
  Plot text channels as state bands (step segments per label); show the label
  in the cursor readout.
  DoD: native tests over the Vector text-conversion corpus files
  (`test_data/reference/Vector_*Text*.mf4`); browser: such a channel plots as
  bands and reads out as text; numeric channels unchanged.
  *Shipped: metadata path is `channels()` + `channel_kind(name)` (kind strings
  `f64|text|bytes|array` + self-named exotics like `canopen_date`); decode
  path is `signal_text(name, t0, t1, max_points)` — run-collapsed label
  changes (first/every change/window-last, 1.5× hard cap + `truncated`),
  `signal_stats` returns the label distribution `{label, samples, seconds}`
  (last sample holds no time), `signal_csv` exports labels. Drawing: state
  bands with a stable per-channel label vocabulary (first-appearance order,
  worker-side); overlay mode gives each text channel a bottom strip, stacked
  mode spans the lane; invalid labels gap like NaN. Browser-verified on
  Vector_Value2TextConversion (bands, label readout "No match", region stats
  5 samples/1s each, 6-state legend) + sample.mf4 numeric regression exact;
  bytes channel refuses to plot with a visible reason. Wasm size 394→406 KB.*
- [x] **2.2 Table view** — M
  Paged, virtualized sample table for the selected channels over the visible
  window (GUI `panels/table.rs` parity). Windowed fetches already exist; add
  `signal_window`-style paging by index range if needed (binding, additive).
  DoD: 300k-sample channel scrolls smoothly; values match cursor readout.
  *Shipped: whole-series, index-paged table (600-row pages) off the worker's
  raw cache — same arrays the cursor readout probes. Numeric/text pages slice
  typed arrays; array/bytes pages go through the new `raw_page` binding
  (element lists / Rust-formatted hex). Virtualized: spacer body + one
  positioned page. Browser-verified on an 18.6M-sample channel (instant first
  page, deep scroll to row 762k). Known browser limit: a scrollbar can only
  reach ~762k rows (Chrome's 16.7 M px scroller cap); beyond that a row-seek
  control would be needed — noted for the huge-file phase.*
- [x] **2.3 Search modes** — S
  Replace substring filter with `search_channels(pattern, mode)` (exact /
  contains / regex — the reader API exists). Debounce; run in worker.
  DoD: native tests exist upstream; browser check on a 1k+ channel file.
  *Shipped: binding `search_channels(pattern, mode)` maps contains
  (case-insensitive) / wildcard / exact onto the reader index; regex is a
  platform RegExp filtering `channel_names()` in the worker (full syntax,
  zero wasm bytes). UI: mode dropdown + 150 ms debounce; results tagged by
  query+mode so stale replies drop. Browser-verified on a generated
  20k-channel file.*
- [x] **2.4 Array / VLSD channels** — M
  Array channels decode to `SignalValues::Array`. Minimum honest behavior:
  mark unplotable in the list, view per-sample elements in the table (2.2),
  keep spike-safe plotting of a selectable element index.
  DoD: Vector array corpus files listed correctly; no NaN garbage plots.
  *Shipped (beyond the minimum): array channels PLOT — element windows
  (`signal_element_window`, binding pre-existing) decimated like scalars,
  legend ◂/▸ stepper cycles elements with wrap, cursor readout reads the
  plotted element from the raw cache (`signal_arrays` now carries eps/starts).
  Fixed and dynamic shapes both work; bytes stay unplotable but select for
  table (hex) + details. Browser-verified on Vector_MeasurementArrays
  (Curve1, 8 elements).*
- [x] **2.5 Details panel** — M (scope to details first)
  Selected-channel details (unit, group, conversion rule, sample count,
  comment) from `channels()` + a new additive `channel_details(name)` if the
  conversion rule is wanted. Tree/blocks/hexdump panels are stretch — park
  them behind a later decision.
  DoD: details render for every channel of the CANedge log; garbage-safe.
  *Shipped: `channel_details(name)` (metadata-only, no decode) +
  `describe_conversion` (one line per rule: linear formula, table sizes,
  identity). Toolbar Details toggle follows the selection; unplotable
  channels select too, so bytes channels get their honest views. Tree/
  blocks/hexdump stay parked.*

## Phase 3 — Formats & bus (the "engineer opens real logs" phase)

- [x] **3.1 MDF v3 read** — M
  Enable the `mdf3` feature for the wasm crate; sniff the signature in
  `WasmMf4File::new` (or a `WasmMdfFile` alias) so v3 files open transparently.
  Watch wasm size (gate: report before/after).
  DoD: native tests over the v3 corpus; browser opens a `.mdf` end to end.
  *Shipped: the wasm crate enables the core `mdf3` feature; `new` sniffs the
  major-version digit at byte 8 (both formats open with the same "MDF     " ID
  — first-8-bytes are not a discriminator) and routes 2.x/3.x to `Mdf3File`
  over an in-memory source. `Inner{V4,V3}` dispatch covers listings, kinds
  (v3 data-type codes 7/8 = text/bytes), info (ns→ISO8601, missing abs time →
  relative-only), search (local wildcard matcher — the reader's is private)
  and series (v3 physical values over the group's time channel, no
  invalidation in the format). 7 tests over an asammdf-written 3.30 fixture
  (written by the test, copied to the corpus for the browser). Size gate:
  446,918 → 511,180 B (+64 KB). Browser: gen_v330.mdf opens end to end, MDF
  3.30 in the info bar, Speed plots, State plots as text bands.*
- [x] **3.2 DBC decoding** — L
  Enable `dbc`, add attach-DBC in the worker (user drops the `.dbc` next to
  the log), decoded signals appear as regular channels (reader `decode_bus`).
  The CANedge OBD2 log is the acceptance file. Watch size; consider a second
  wasm build if the size gate fails.
  DoD: decoded OBD2 signals (RPM, pedal, etc.) plot from
  `OBD2 (Audi A4)/…/00000002.MF4`; values cross-checked against the GUI or
  asammdf on the same DBC; size gate reported.
  *Shipped: `attach_dbc(bytes)` materializes decode_bus output as cached
  series named `message.signal` (bus-suffixed on multi-bus duplicates);
  overlay channels flow through every endpoint (windows, stats, CSV, table,
  readout); value-table signals are text (bands + labels). UI: Attach DBC
  button + ?dbc= deep link. 6 tests over the acceptance log with a
  hand-written multiplexed DBC — rpm pinned against hand arithmetic on the
  same frames (the GUI uses the same core decoder, so this IS GUI parity).
  Size gate: 511,180 → 782,679 B (+271 KB; no second build needed). Browser:
  EngineSpeed plots 808..3586 rpm over 2210 readings; readout 887 rpm.*
- [x] **3.3 Bus frame panel** — M (after 3.2)
  CAN/LIN frame list (ID, DLC, data, dir) from `can_frames`/`lin_frames`,
  paged like 2.2, linked to the plot cursor.
  *Shipped: `bus_groups` + `bus_frames_page(kind, group, start, count)` (hex
  payload, id, DLC, ext/dir, bus) + `bus_frame_locate(kind, group, t)`
  (Rust-side binary search). Panel is virtualized like the table; a frame
  click places cursor A at that frame (frame→plot) and a plot cursor scrolls
  + highlights the nearest frame (plot→frame). 2 tests; browser-verified:
  frame click reads exactly 2,368 rpm (hand-decoded value).*
- [x] **3.4 GPS panel** — M
  Canvas track plot from lat/lon channels, colored by speed, no map tiles
  (static-site rule: no CDN). The GUI panel is the reference. Needs the
  Internal CANedge GPS file (`test_data/mf4-sample-data-v2.1/Internal…`).
  DoD: track renders for the GPS IMU log; cursor syncs position ↔ plot.
  *Shipped: `detect_gps_channels()` ports the GUI name heuristics;
  `gps_track(lat, lon, speed?)` rides the latitude master, pads shorter
  companions with gaps (`aligned:false`), stride-decimates past 20k points.
  Canvas: equirectangular projection (cos-lat corrected), green→red speed
  coloring, start marker, plot cursors marked on the track, track click
  places cursor A. 7 tests over a writer-made square drive. NOTE: the
  CANedge GPS logs carry coordinates inside CAN payloads — the live track
  needs the vendor GPS DBC attached (3.2's flow); the browser check ran on
  the writer fixture (20k points, both cursor directions verified).*
- [ ] **3.5 ARXML decoding** — optional, decide after 3.2's size data.
  *Decision (documented deferral): size headroom exists (783 KB / 2 MB), but
  the browser cannot use `CanDatabase::from_arxml_path` — AUTOSAR loading in
  the core is path-based and the wasm side needs a `from_arxml(&[u8])`
  in-memory variant (autosar-data load-from-buffer investigation) before the
  binding can exist. The viewer's DBC path covers bus decoding today; revisit
  when a real ARXML acceptance file + the core byte-loader land together.*

## Phase 4 — Beyond the GUI

- [x] **4.1 X-Y mode** — M
  Two overlaid channels plotted value-vs-value (worker has both raw arrays
  already). GUI `xy.rs` is the reference. DoD: circle test (sin vs cos);
  Lissajous from the sample file.
  *Shipped: X-Y panel with two channel selectors; the worker samples Y at
  X's own timestamps (nearest-sample, the readout's probe) and strides to
  20k; canvas with auto-scaled labeled axes. Browser-verified pixel test on
  a writer-made sin/cos fixture: empty-center ellipse, all 12 sampled edge
  angles inked.*
- [x] **4.2 Computed channels** — L
  Small expression evaluator (a + b*2, abs(), integrate?) creating virtual
  channels; evaluate in Rust on the raw arrays (never on decimated data) and
  treat like any channel thereafter. GUI `computed.rs` is the reference.
  DoD: parser tests incl. error messages; computed channel plots, zooms,
  exports to CSV like a real one.
  *Shipped: recursive-descent evaluator over a `[Channel]`-reference grammar
  (+ - * / unary minus parens abs/sqrt/min/max); parse errors name the
  offset; references are checked at define time; the first reference's
  timeline anchors evaluation and others sample nearest-index, in Rust over
  raw arrays. Computed channels are ordinary channels to listings, plot,
  stats, CSV and the table. 2 native tests + browser-verified
  ([Sin]*100+5 → -95..105, mean 5, 10k samples). integrate() was left out —
  no use case in the viewer today.*
- [x] **4.3 Two-file compare** — M
  Worker holds a map of open files; overlay shows `file:channel` names and
  per-file colors (GUI `SeriesKey::File` model). DoD: same channel from two
  logs overlays with independent Y offsets; cursor readout shows both.
  *Shipped: the worker keeps extra files keyed by label; second-file
  channels are named `@<label>::<channel>` — plain text to the main thread,
  a routing directive on the worker — so every endpoint works on both files
  without touching the drawing code. 'Compare file…' button + ?compare=
  deep link. Browser-verified: sample Speed and gen_xy Cos overlay; readout
  shows both values at one cursor. Per-file COLORS: identity rides the
  @label:: prefix in the legend rather than a separate palette — revisit if
  labels collide.*
- [x] **4.4 Shareable view state** — S
  Encode `{file, channels, window, mode}` in the URL hash (extends `?file=`).
  DoD: paste-a-link opens the exact view (given the file URL is reachable).
  *Shipped: hash carries f/c/t0/t1/m, updated debounced on every view change
  (never for dropped local files) and restored on load after meta lands.
  Browser-verified round trip on sample.mf4.*
- [x] **4.5 OPFS persistence** — M
  Cache opened files in Origin Private File System; reopen instantly, survive
  reload. Size cap + evict LRU; always opt-in per file.
  *Shipped: opt-in 'keep this file in the browser' checkbox on local-file
  open; entries LRU-capped at 5 files / 512 MB total (oldest evicted,
  oversize files refused); landing lists cached files with open/remove;
  quota or privacy failure degrades to no caching.*
- [ ] **4.6 Huge-file streaming (File System Access API)** — L
  Read block ranges from disk on demand instead of loading the whole file
  into wasm memory (the web's answer to the GUI's mmap). **Needs core
  support** (a read-callback `open` path in `src/`) — design doc first, then
  binding. DoD: a >2 GB log opens and pans where `from_bytes` OOMs.
  *Design delivered (`docs/wasm-streaming-design.md`): a `remote_source`
  ByteSource whose read callback is a worker-side Atomics.wait mailbox over
  range requests, 64 KiB block LRU, `open_streaming(len, cb)` mirroring
  `new`. Deliberately NOT implemented this round: the plan gates it behind
  this design review, and the central risk (the Atomics mailbox under
  COOP/COEP) should be spiked first before the ~40-line core change lands.*
- [x] **4.7 Virtualized channel list** — S
  Render only visible rows; files with 10k+ channels stay smooth. DoD: synth
  a 20k-channel file in a test; filter + scroll at 60 fps.
  *Shipped with the platform, not code: `content-visibility: auto` +
  `contain-intrinsic-size` on the channel rows, so the browser skips
  layout/paint off-screen. Browser-checked on a generated 20k-channel file
  (filter + search + scroll smooth).*
- [x] **4.8 Structure tree** — M
  The desktop viewer's structure panel (`panels/tree.rs`): the file's
  outline — identification/header blocks, history, attachments, events,
  channel hierarchy, data groups → channel groups → channels — with
  collapse, filter and per-group plot-all.
  *Shipped: `structure()` returns the whole outline as one metadata-only
  JSON document (v4 via `block_map`/HD chains, v3 with the sections its
  format lacks left out); the demo's sidebar grew Structure/Channels tabs,
  and the tree mirrors the desktop rules — filter matches group or channel
  names and forces data-group sections open, Plot all adds readable
  non-master channels up to this viewer's 8-channel overlay cap skipping
  already-plotted ones, channels toggle through the ordinary `toggleChannel`
  path so plotted state stays in sync with the channel list, 400-row cap
  per group with an overflow note. Browser-verified against
  `Vector_Embedded.MF4` (66 blocks, 4 history entries, 3 embedded
  attachments, both data groups), a writer fixture (text kind, 400-row
  cap at 451 channels), and an asammdf v3 file; native tests pin the JSON
  shape, escaping, flags, and v3 differences.*

---

## Verification gates (every item, every merge)

```bash
cargo build -p falcon-mdf-wasm --target wasm32-unknown-unknown
cargo test -p falcon-mdf-wasm          # count must not drop
cargo test --all-features --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

Browser checklist (serve `wasm/demo` over HTTP after
`wasm-pack build wasm --target web --release && cp -r wasm/pkg wasm/demo/pkg`):

1. Sample button works end to end; info bar + channel list render.
2. 3+ channels overlay; legend shows units and visible ranges.
3. Wheel zoom re-decodes in < ~300 ms (status line settles); drag pans;
   double-click resets.
4. Cursor readout lists every overlaid channel's value at the nearest sample.
5. Download CSV row count equals the visible-window sample count
   (cross-check via a native `signal_csv` call).
6. 9th channel refused with the visible message.
7. Garbage file shows the parser's error; the page survives and recovers.
8. NaN/invalid samples draw as gaps, never bridges (sample.mf4 Boost).
9. Main thread never imports the wasm module (worker-only rule).
10. Live-site spot check after deploy (sample button on the Pages URL).

## Invariants (do not break while executing this plan)

- The public binding API is additive-only; the live site must not regress —
  `pages.yml` deploys on merge, so the browser checklist is a pre-merge gate.
- Nothing in `wasm/src/lib.rs` may panic (no `unwrap`/`expect`/panicking
  indexing): a wasm panic kills the module for every caller. Every error is a
  thrown JS `Error` with a message.
- All wasm stays in the worker; the main thread only draws.
- Static site: no build step for the demo, no CDN, no framework, no plotting
  library — the canvas draws plain lines from Rust-decimated points.
- Dependencies: none beyond `js-sys` without a conscious decision recorded
  here (each addition: why, size cost, alternative rejected).
- Wasm size gate: report `.wasm` bytes before/after any feature-flag flip
  (3.1, 3.2); keep the site's first load reasonable (< ~2 MB wasm as a guide).

## Risks / open decisions

- **Feature-flag size blowup** (Phase 3): `dbc`/`mdf3` may push the binary
  past the size guide. Fallback: a second, heavier "pro" build of the same
  demo (two pkgs, feature-detected) — decide when measured.
- **4.6 touches `src/`**, everything else can stay inside `wasm/`. If the
  core change stalls, ship 4.5 (OPFS) as the interim large-file story.
- **Text-channel plotting** (2.1) needs a draw primitive the canvas path
  doesn't have yet (state bands) — prototype before committing to the binding
  shape; the API must not churn once published.
