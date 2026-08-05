# falcon_mdf — GUI viewer plan

A desktop application for opening MF4 files, browsing their channels and
plotting them, built on this crate. The crate stays a library; the GUI is a
second workspace member that depends on it.

---

## 1. What this is for

`falcon_mdf` reads MF4 files correctly and fast, but the only way to see
anything is to write Rust or run an example that dumps CSV. Every audience for
a measurement-file reader — an engineer checking whether a log captured what
they expected, someone comparing two runs, anyone triaging a file another tool
refuses — wants to look at the data before scripting against it.

The reference product in this space is asammdf's Qt GUI. That is the bar for
*features*, not for implementation.

**Non-goals.** No editing or writing (the library cannot write; Phase 5 of the
main plan). No DBC/ARXML bus decoding. No MDF 3.x. Nothing that widens the
library's own scope — the GUI is a consumer of the public API, and where the
API is inadequate the fix belongs in the library, deliberately.

A useful side effect: the GUI is the first serious external consumer of the
public API, arriving exactly when Phase 6 wants to freeze it. Whatever the GUI
finds awkward is a finding about the API, and should be recorded as one rather
than worked around in the app.

---

## 2. Framework: egui via eframe, with egui_plot

**Recommended, and the reason is the data path, not the widgets.**

This application is ~80% one interactive plot over large numeric arrays. The
decisive question is what happens between `Arc<Vec<u8>>` and pixels.

- **egui keeps that in-process.** The decimator can call `Signal::value_at` or
  walk `values_f64()` directly and hand `[f64; 2]` points to the plot. No
  copying, no serialization.
- **Tauri would put a JS bridge in the middle.** The UI would be nicer and
  plotting libraries there (uPlot) are excellent, but every pan and zoom
  serializes sample arrays across the boundary. For millions of samples that is
  the dominant cost, and it buys polish this audience does not need.
- **iced and Slint** are credible frameworks with a weaker plotting story;
  `egui_plot` is the mature option and this app is mostly plot.
- **Xilem** is the interesting long-term bet from the Druid team and is not
  ready.

egui is also the consensus pick for developer tools and visualization
dashboards specifically, which is what this is.

**The honest trade-off:** egui is immediate-mode. It redraws every frame, and
its look is "tool-like" rather than native-polished — no platform-native menus
or widgets without extra work. For a measurement viewer that is acceptable and
roughly what users of this category expect. If native polish ever becomes a
requirement, that is a rewrite, not a tweak. Worth knowing now.

`egui_plot` will not carry raw sample counts on its own: the known guidance is
that plotting hundreds of thousands of points allocates and tessellates on the
CPU each frame, and that data should be downsampled before it reaches the plot.
That is §4, and it is the core engineering of this project.

---

## 3. Repository layout

Keep the library at the repo root and add the GUI as a workspace member. A root
package *and* a workspace coexist fine, so this avoids moving `src/`, breaking
`test_data/` paths, the benches, the fuzz target or CI.

```toml
# Cargo.toml (root, additive)
[workspace]
members = ["gui"]
```

```
falcon_mdf/
  Cargo.toml        # the library, unchanged deps
  src/              # unchanged
  gui/
    Cargo.toml      # publish = false
    src/
```

**Constraint, and it is the important one: no GUI dependency may enter the
library crate.** Not behind a feature flag either — a feature that pulls
`eframe` into `falcon_mdf` puts a windowing stack in the dependency tree of
every library user who enables it by accident. The dependency points one way.

Binary name `falcon`; crate name `falcon_mdf_gui`, `publish = false`.

---

## 4. The one hard problem: drawing a lot of samples

Everything else here is assembly. This is the part that decides whether the app
is usable.

**Never hand raw samples to the plot.** Decimate to roughly twice the plot's
pixel width, then plot that.

**Decimate by min/max per pixel column, not by sampling every Nth point.**
This matters more here than in most domains. A measurement channel's single-
sample spike is frequently the whole reason someone opened the file; stride
sampling deletes it silently, and the plot looks plausible while lying. For
each pixel column take the min and the max of the samples falling in it and
emit both, in time order. A spike survives as a vertical excursion. This is the
standard aggregation for this problem.

**Use the file's own reduction blocks when it has them.** `reduced_signal()`
already exposes SR blocks — the file's precomputed min/mean/max at coarser
intervals (Phase 4.7 of the main plan). When a file carries them, the
zoomed-out view is free and exact; compute decimation only when it does not, or
when zoomed in past the reduction's interval. This is a real advantage over
naive viewers and the library already supports it.

**Cache per (channel, viewport width).** Recompute on zoom/pan, not per frame.

**Decode off the UI thread.** `Arc<Mf4File>` shared with a worker; the UI shows
a spinner and stays responsive. `Signal` is `Send` and owns its data, so a
decoded signal can be handed back and held.

**Batch reads by channel group** — see §6.1. Not optional.

---

## 5. Feature phases

Each phase ends with something runnable.

**Status: P1, P3, G1, G2, G3 and G4 are done.**

The record and payload caches now hold up to four entries, bounded by
`Limits::max_alloc` in bytes rather than by entry count — a group's records can
be hundreds of megabytes, so counting entries would have been a 4x memory
multiplier nobody asked for. Alternating reads between two channel groups went
from **191.5 µs to 78.6 µs** per iteration. The read-fast path survived: recency
is an atomic timestamp bumped through `&self`, so a hit never escalates to the
write lock. `Mf4File` and `Signal` are both `Send + Sync`, now pinned by a
compile-time assertion in `tests/api_surface.rs`.

`falcon` opens all 65 corpus files (57 reference + 8 sample), browses and
searches channels, and shows file metadata. `gui/examples/verify_corpus.rs`
keeps that a standing check rather than a one-time claim.

**What the GUI found in the API — the point of building it before the freeze.**
Three things, one of which is a defect:

1. **B36 — found, fixed, closed.** `channel_names()` returned a different order
   on every process run, because `ChannelsDB` is a `HashMap` and Rust seeds its
   hasher randomly — while `build_channels_db: false` returned a *sorted* list.
   One accessor, two contracts, selected by an option documented as a
   memory/speed trade-off. Exactly B18's shape, and found the same way B18 was:
   by consuming the API rather than reading it. Now sorted at the source in
   `ChannelsDB`, so both configurations agree by construction; the order is a
   documented guarantee, and the GUI dropped the defensive sort it had been
   doing to work around it.
2. **No substring search primitive.** `find_channels` is exact-match only, so a
   searchable channel list — a near-universal need for any UI — has to pull the
   whole name list, filter client-side, then re-resolve each match. Worth a
   `channels_matching(predicate)` before the freeze.
3. **`Channel` does not carry its sample count.** It lives on the parent
   `ChannelGroup`, which is correct (all channels in a group share it) but means
   code holding a bare `&Channel` from `find_channels` must index back into
   `data_groups()[dg][cg]` to display it.

### G1 — Shell and browsing
- Open via file dialog, drag-and-drop, and CLI argument. Recent-files list.
- Channel list with substring search, backed by `find_channels`/`channel_names`.
  Group by data group / channel group; show unit and sample count.
- Metadata panel: version, start time, comment, file size, `statistics()`.
- Verify: open every one of the 57 reference files without a crash or hang.

### G2 — Plotting one channel — **done**
- Select a channel, plot it against its master (`master_channel`).
- Decimation per §4, min/max columns.
- Cursor with a value readout at the hovered time; zoom and pan.
- Verify: a channel with a known one-sample spike still shows the spike when
  fully zoomed out. This is the test that decimation is honest — build a
  synthetic file for it rather than hunting one in the corpus.

`gui/src/decimate.rs` holds `decimate_min_max(times, values, x_range,
n_columns)`, deliberately a free function so a test can call it without an
`egui::Context`. Cached per `(channel, x_range, pixel width)`, so egui's
per-frame `show()` only rescans when the view actually moved. The fixture is
`gui/tests/spike_survives_decimation.rs`: a hand-built MF4 file, 9973 samples
(prime, so no stride can land on the spike by luck), one sample forced to 999.0
at index 5000, decoded through the real path and decimated to 200 columns.

Two things are worth recording about how it was verified, because the phase
turned on both.

**The teeth check.** Replacing the decimator with naive stride sampling made
the fixture test fail with all 200 output points flattened to the baseline —
the spike gone entirely. Without that check a green test would have proved
nothing, since a decimator that returns *anything* plausible passes a test that
only looks at the plot.

**What the test could not catch.** The first implementation rebuilt each column
boundary as `x0 + (col_index + 1) * col_width`. Once `col_width` falls below
the ulp of `x0`, that addition rounds back to `x0`, the inner loop's
`times[i] < col_end` is false forever, `i` never advances, and the outer loop
spins — an unkillable UI freeze, the same class as B13. Reachable without a
malformed file: many samples sharing one timestamp (ordinary in bus logs, and
the thing that defeats the sample-*count* early return) plus a narrow zoom on
an epoch-seconds master. Fixed by advancing `i` past the sample that defined
the column *before* testing any boundary, so progress is structural rather than
an argument about floating point. Pinned by a watchdog test that hangs on the
old code.

The residual: in that degenerate case the output is no longer bounded by
`2 × n_columns` — every sample becomes its own column, so 1000 identical
timestamps yield 1000 points for a 200-column request. Bounded by the visible
sample count, never unbounded, and only reachable where timestamps are
unresolvable at f64 precision across the visible span — at which point there is
nothing meaningful left to aggregate *by time*. Accepted rather than fixed.

**SR blocks were considered and deliberately not built.** Only 12 of 65 corpus
files carry reduction blocks, all toy dSPACE fixtures capped at 5001 samples —
one has 5000 reduced records at a 0.001 interval, essentially 1:1 — while the
corpus's actual largest channel (145,535 samples, `CAN_DataFrame.ID`) has none.
Level selection and an extra cache dimension for zero files that would benefit.
Revisit when a real file justifies it.

### G3 — Multiple channels, and honest failures — **done**
- Several channels on one plot; stacked plots; per-channel colour and
  visibility. Second Y axis for differing units.
- **Unreadable channels surface their reason.** `Mf4Error::Unsupported` now
  names the case (`ArrayGroupTemplate`, `ArrayDynamicSize`, sync channels) —
  show that text in the channel list rather than hiding the channel or showing
  an empty plot. This is the UI payoff for Phase 4.14's work and the thing that
  distinguishes this from a viewer that silently shows nothing.
- Invalidation bits: samples the file marks invalid must be visibly gapped, not
  drawn as if measured. `Signal::validity()` already provides this.
- Verify: a file with an invalid range plots a gap, not a line.

Overlay and stacked views, per-channel colour from a fixed palette, and a
visibility checkbox per plotted channel. The spec's "second Y axis for
differing units" became stacked subplots: egui_plot 0.36 has no per-series
second axis (axes exist only as widgets), so instead of silently plotting
volts against RPM on one scale, each channel gets its own, X-linked so zoom
and pan stay in sync; the overlay legend names each line's unit. That is the
honest reading of the requirement.

Verified twice, as G2 was. `gui/tests/invalid_range_plots_a_gap.rs` builds a
file whose samples 400..600 carry the invalidation bit with 1e9 garbage in the
record, decodes it through the real path and asserts two segments with the
garbage in neither — plus the teeth check that ignoring validity yields one
segment containing the garbage. Then the running app, driven on screen: the
gap is visible in both views, hovering the gap reads "(sample marked
invalid)", unticking a channel removes its line without disturbing the others,
and a channel the library cannot decode shows a ⚠ in the list and its reason
inline where its line would be.

**What the GUI found in the API — the second such finding.** The channel list
must say why a channel cannot be shown *before* the user asks to plot it, but
a synchronisation channel's refusal lived only in `Signal::values()`: parsing
handed back a `Channel` that looked readable and failed on the first read.
`Channel::unreadable()` now reports `UnreadableReason::SyncChannel` at parse
time, with the decode-time refusal kept as the backstop for hand-built
signals, and `tests/synthetic_blocks.rs` pins it, teeth first. The list's ⚠
and the plot's inline failure line both render that one reason.

### G4 — The rest of the file — **done**
- Attachments (list, save embedded data out), events on the time axis as
  markers, file history, channel hierarchy tree, source info per channel.
- Export selected channels to CSV, reusing the `export_to_csv` example's logic.
- Verify: exported CSV matches the existing example's output for the same
  channels.

History, attachments, events and the hierarchy live as collapsing sections in
the file panel, each naming its count so an empty one still says "none of
these here"; embedded attachments get a save action, and the channel detail
grows the source rows. Time-synchronised events become markers on the plot's
X axis (angle, distance and index events do not belong on a time axis and
stay in the list), capped so a trigger-happy file cannot flood the legend.
The hierarchy draws as far as the accessor reaches and marks nodes whose
children this build cannot descend into, rather than silently drawing one
level; recursing `parse_hierarchy` remains the Phase 6 decision it was.

The export moved into the library as `falcon_mdf::write_csv`, with the example
rewritten to call it: reuse by construction, and `tests/export_csv.rs` pins
the format byte for byte against output captured *before* the move. Verified
on screen the same way: the app's Export action wrote a CSV that `diff` cannot
distinguish from the pre-refactor example's.

**What the GUI found in the API — the third such finding.** A hierarchy
element is a (data group, channel group, channel) triple of *block offsets*,
and no public path mapped those offsets back to a `Channel` — the tree would
have had to print numbers as names. `Mf4File::channel_at` is the resolver,
returning `None` for a triple no block carries; `tests/synthetic_blocks.rs`
pins both halves.

### G5 — Packaging
- App icon, window state persistence, error dialogs that do not lose the
  message.
- Bundle for macOS/Windows/Linux; document the build.

---

## 6. Known issues to resolve, with evidence

### 6.1 The record cache is a single slot — **fixed in P1**
`Mf4File::records_for` (`src/file.rs:1618`) keeps `record_cache:
RwLock<Option<CachedRecords>>` — **one** entry, keyed by one `(dg, cg)` pair.
`payload_cache` is the same shape. Reading channel A from group 1, then B from
group 2, then A again rebuilds the entire record buffer twice. A group's
records "can be hundreds of megabytes" by the field's own comment.

G2 never notices this. G3 plots channels from several groups at once and hits
it every frame.

Two options, and the choice belongs to the library rather than the app:
1. **GUI batches by group** — decode everything needed from group 1, then move
   on. Cheap, no library change, but it constrains the app's structure forever
   and any future consumer rediscovers the cliff.
2. **Make it a small LRU** (2-4 entries, bounded by `Limits::max_alloc`). A
   contained library change that fixes the problem for every consumer.

Recommend (2), sized deliberately, with (1) as the interim so G2 is not
blocked. Measure first: the profiling harness in `benches/` and the phase-3
work already establish how.

### 6.2 `channel_hierarchy()` cannot reach child nodes
`parse_hierarchy` walks the `ch_next` sibling chain and only *flags* nesting
via `has_children`; it never descends into `ch_first` (recorded in main plan
4.14.4). G4 wants a tree. Either the accessor learns to recurse, or the tree
view can only ever draw one level. Library-side, and worth deciding during
Phase 6 rather than discovering it in G4.

### 6.3 Confirm the backends are `Sync` — **confirmed in P3**
`Mf4File` uses `RwLock`, so sharing it as `Arc<Mf4File>` across threads should
hold, but the I/O backends have not been checked for it. Verify before
building G1's loader on that assumption — and note B14: `Mmap` carries an
undocumented obligation if the file is truncated externally, which a GUI makes
*more* likely, not less, since files sit open while a logger may still be
writing. The buffered backend may be the right default for the GUI even though
mmap is the library's.

---

## 7. Sequencing against the main plan

Phase 6 (API freeze) and the GUI should be done **together, in that order of
authority**: the GUI is the best available test of whether the public API is
right, and freezing before hearing from a real consumer wastes the opportunity.

Suggested order: G1 → API review informed by G1/G2 → 6.1 fix → finish the
freeze → G3-G5. Write support (Phase 5) is unrelated and should not block this.

---

## 8. Definition of done

- Opens all 57 reference files and both sample corpora without crash or hang.
- Plots a 30k-sample channel interactively at a steady frame rate; a
  million-sample channel remains usable through the decimation path.
- A one-sample spike is visible when fully zoomed out.
- Invalid samples are gapped; unreadable channels state why.
- The library crate's dependency tree is unchanged by the GUI's existence.
