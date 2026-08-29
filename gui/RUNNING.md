# Running falcon

How to build the viewer, open a measurement in it, and tell a real failure
apart from a known limitation. Build and distribution notes — bundles, icons,
per-platform packaging — live in [PACKAGING.md](PACKAGING.md).

This file ships inside the release archives as well as living in the
repository, so it starts with the case where there is nothing to build.

## If you downloaded a release

The archive holds one binary — `falcon`, or `falcon.exe` — plus the licences,
this file, and on macOS a `FIRST-RUN.md`. Unpack it anywhere; nothing installs
and nothing is written outside the archive until the viewer itself saves its
recent-files list.

```sh
./falcon --version        # names the release this binary came from
./falcon --help           # the arguments it takes
./falcon measurement.mf4  # open a file straight away
./falcon                  # or an empty window, and open from the top bar
```

The binaries are **unsigned on every platform**, which each one complains
about in its own way:

- **macOS** refuses outright, and says the app is damaged rather than that it
  is unsigned. `FIRST-RUN.md` in the archive is the one-line fix.
- **Windows** shows a SmartScreen warning: *More info* → *Run anyway*.
- **Linux** does not care, but the binary may need `chmod +x falcon` after
  some unpackers.

Because nothing is signed, verify the download against the `.sha256` file
published beside it:

```sh
sha256sum -c falcon-gui-linux-x86_64.tar.gz.sha256   # shasum -a 256 -c on macOS
```

The rest of this file is the same whether the viewer was downloaded or built.

## Build and run

The GUI is the `falcon_mdf_gui` package, a workspace member built as a library
(`gui/src/lib.rs`) with a thin binary (`falcon`). Exposing the viewer's logic
makes it testable without an open window, covered by 9 integration test files
under `gui/tests/`. From the repository root:

```bash
# Release. Worth it: decimation and plotting are noticeably smoother, and a
# debug build stutters on the larger bus logs.
cargo run --release -p falcon_mdf_gui

# Debug. Compiles faster, sluggish on anything large.
cargo run -p falcon_mdf_gui
```

`cargo build --release -p falcon_mdf_gui` leaves a standalone binary at
`target/release/falcon` that runs without cargo.

## Opening a file

Four ways, all equivalent once the file is open:

1. **On the command line**, which opens it immediately at startup:

   ```bash
   cargo run --release -p falcon_mdf_gui -- test_data/reference/ASAP2_Demo_V171.mf4

   # or, against the standalone binary
   target/release/falcon test_data/reference/ASAP2_Demo_V171.mf4
   ```

2. **"Open File…"** in the top bar. The dialog filters on both `mf4` and `MF4`,
   so the upper-case vendor files are not hidden.
3. **Drag and drop** onto the window. No extension filter at all — it takes
   whatever path you drop.
4. **Recent Files** in the top bar, which persists across runs.

The window title becomes `falcon — <filename>`, so several open viewers stay
tellable apart.

## What you are looking at

The window is two panes. **Everything the file holds is on the left; what you
select there is shown on the right.** The strip along the bottom names the open
file and the current selection, so "where am I" never needs hunting for in a
scrolled tree.

### Left: the file

Three tabs, each a different way into the same file:

| Tab | What it lists |
| --- | --- |
| **Structure** | The file as the format means it: the identification and header blocks, then file history, attachments, events and the channel hierarchy, then the data groups, their channel groups, and their channels. A filter box at the top hides groups with no match. Toolbar buttons provide **Expand all** and **Collapse all**, and a small **Plot all** button on each channel group header plots its channels (capped at 16). A checkbox beside a channel plots it individually. A group marked 🚌 holds logged bus traffic, ≡ variable-length data; a channel marked ▦ is an array and ⚠ one this build cannot decode (hover for the reason). |
| **Blocks** | Every block in the file, from byte 0 to the last one, in the order they sit on disk — address, type, size, and a line describing its fields. The chips above the list are the file's composition and filter it by type; the gaps between blocks are shown too, marked as alignment padding or, when larger, as bytes no block covers. |
| **Channels** | The flat, searchable channel list, for when you know the name and not where it lives. The search supports **Substring**, **Wildcard** (`*` and `?`) and **Regex** (literals, `.`, postfix `*` `+` `?`, `[abc]`, `[^abc]`, `^`, `$`; malformed patterns report their error). It matches channel names, units, comments and group acquisition names. Result rows show the group each match came from, and filter toggles narrow by arrays only, unreadable only or master channels only. **Plot all matching** adds up to 32 matches to the plot. Switching back to Structure scrolls to the picked channel. |

### Right: what it is

Six tabs. **Details** follows the selection; the rest are about a channel or a
group and say so when the selection is neither.

| Tab | What it shows |
| --- | --- |
| **Details** | The file (version, start time, statistics, block composition, header properties), or a data group, channel group, channel, attachment, event or history entry. A channel gives its layout, conversion, array shape, validity and source, and links to the `##CN` block that defines it. A **block** gives its header fields, its links as buttons that follow them, the blocks that point at it, and its bytes as a hex dump. |
| **Plot** | Every plotted channel against its master, overlay or stacked, with min-max decimation, cursor readouts, event markers, and CSV/MF4 export. A toolbar toggle switches the x axis between relative seconds and absolute UTC wall-clock time (`YYYY-MM-DD HH:MM:SS.mmm`), and per-signal color pickers and line width controls (1.0–4.0) style each trace. Invalid samples are drawn as gaps. **Cursors** turns on measurement cursors A and B — click places A, shift-click places B — reading out time and value deltas, plus a region statistics table (count, excluded count, min, max, mean, delta) over the window between them. **Clear cursors** removes them, and **Fit view** resets bounds to the full time range. |
| **Numeric** | The instantaneous value of every plotted channel at a chosen time: a time box with **Start** and **End** buttons jumping to the bounds of the plotted range, one row per channel with its value and the timestamp of the sample used. Values are taken from the sample at or before that time, never interpolated; invalid samples are skipped and counted. |
| **Samples** | The selected group as a table: one row per sample, one column per channel, values in their own types — integers as integers, payloads as hex, text as text — with invalid samples struck through as `—`. Clicking a column header sorts by it (ascending, descending, then back to file order), a filter box keeps only the rows whose cells contain what you type, and **Export table…** writes exactly what is shown as CSV. Invalid samples sort last in both directions, because they are not measurements. |
| **Bus** | The frames of a bus-logged group: timestamp, identifier in hex and decimal, bus channel, length and payload, filterable by identifier or name. Which reader applies is decided from the group itself — `CAN_DataFrame` for CAN, `LIN_Frame` for LIN — and a group composing neither says so. For CAN, **Load DBC…** decodes a selected frame's payload into named signals, and a **Signals** mode decodes the whole group into time series and charts the ones you tick — a signal whose value table gives it text is listed rather than drawn as a flat line. For LIN those controls are hidden, because this build carries no LIN database. **Export frames…** writes the frames currently listed — after the filters, in the order shown — as CSV: `index`, `time_ms`, `id`, `id_hex`, `extended`, `bus_channel`, `length`, `data_hex`, and for CAN a `message` column carrying the name the loaded database gives that identifier, empty when it has none. LIN has no `message` column at all, since there is no LIN database to fill it. |
| **Statistics** | Count, valid count, range, mean, spread, median, timing and sample rate for the selected channel, plus a distribution. The 5th, 25th, 75th and 95th percentiles are reported beside the median, computed by linear interpolation between neighbouring ranks — the definition numpy and asammdf use, so the numbers agree with the tool people check against. Invalid samples are excluded and counted separately. |

Both the block list and the sample table are virtualized: a group with millions
of samples, or a file with a hundred thousand blocks, only ever builds the rows
on screen. Sorting and filtering the table produce an index list rather than
reordering the samples, so neither costs a re-decode.

### Keyboard

| Keys | What |
| --- | --- |
| `Cmd`/`Ctrl` + `O` | Open a file |
| `Cmd`/`Ctrl` + `F` | Jump to the channel search and focus it |
| `Cmd`/`Ctrl` + `1` `2` `3` | Structure, Blocks, Channels |
| `Cmd`/`Ctrl` + `Shift` + `1`…`6` | Details, Plot, Numeric, Samples, Bus, Statistics |
| `Alt` + `←` / `→` | Back and forward through selections |
| `?` | The shortcut list |

Shortcuts do not fire while a text box has focus, so typing `b` into the
search box searches for `b` rather than jumping to the block list.

### What it remembers

Reopening a file restores the channels that were plotted and the two tabs that
were open, for the last 20 files. A channel the file no longer has — the path
was rewritten with a shorter recording, say — is dropped rather than restored
into a group that has shrunk. The store is a text file under eframe's storage
alongside the recent-files list; a line it cannot read is skipped rather than
taken as a reason to forget every other file.

## Getting measurement files

There are none in the repository and there never will be: they are other
vendors' files, this project does not redistribute them, and they are large.
The fetch script pulls them into `test_data/`, which is gitignored:

```bash
bash scripts/fetch_reference_files.sh   # 67 files, ~10 MB
```

Only the values those files decode to — the JSON under `tests/data/` — is
checked in. See `scripts/fetch_reference_files.sh` for what is fetched and from
where.

Good files to open first, once fetched:

| File | Why |
| --- | --- |
| `test_data/reference/ASAP2_Demo_V171.mf4` | 45 channels over 7 groups, 50,974 samples — a broad first look |
| `test_data/reference/Vector_MeasurementArrays.mf4` | array channels, several shapes |
| `test_data/reference/multiple.MF4` | an *unfinalized* 4.11 bus log, with both CAN and LIN groups |
| `test_data/mf4-sample-data-v2.1/J1939 (truck)/LOG/958D2219/00002501/00002081.MF4` | a real CAN log; the one to use for plotting and decimation |

The bus-logging corpus under `mf4-sample-data-v2.1/` has no public source and is
not fetched by the script.

No published set contains a file with **video**, because MDF 4 stores it as a
synchronisation channel plus an *external* attachment — the `.avi` sits beside
the `.mf4`, so a real example is a multi-file vehicle recording. To see how one
presents, generate it:

```bash
.venv/bin/python scripts/make_video_fixture.py   # needs asammdf
```

That writes two files, which is the point — a video recording is a *pair*:

```
test_data/generated/video_sync.mf4   the measurement, naming the stream
test_data/generated/drive.avi        the stream itself, 10 frames at 25 fps
```

**To watch the video, open `drive.avi` in any player.** It is a real file (with
ffmpeg installed; without it the script writes a header stub that will not
play). The viewer cannot show it: falcon is a plotter, and the video lives
outside the MF4 entirely.

Opened in the viewer, `video_sync.mf4` shows what the measurement side of a
video looks like. The `VideoFrames` channel carries the ⚠ marker, and hovering
gives the reason — its samples are frame indices into that stream, not
measurements, so there is nothing to plot. There is one sample per frame, which
is how a timestamp maps to a picture. Under Attachments, `drive.avi` shows as
external with no **Save…** button, because those bytes are not in the MF4 —
delete the `.avi` and the MF4 still opens, still naming a stream that is now
gone.

## Known limitation: channels that plot as nothing

The viewer is a plotter. Every channel goes through `values_f64()`, so anything
without a numeric value — **strings, complex numbers, byte arrays, CANopen
dates and times** — decodes to all-`NaN` and draws an empty chart.

It does so **without any warning**. The `⚠` marker beside a channel means
*undecodable*, and these channels decode perfectly well; they simply have no
number to plot. So an empty chart currently looks the same as a broken one.

Across the 67-file reference set this affects **41 channels**.
`test_data/reference/all_datatypes_test.mf4` shows it in one file: `int8_data`
plots, while `string_data` and `complex64_data` look broken and are not. Use
the library's `Signal::values()` to see those samples in their own type.

## Checking the viewer against a whole corpus

To confirm the GUI's file-handling path opens everything — no crash, no hang,
and any failure showing its real error text:

```bash
cargo run -p falcon_mdf_gui --example verify_corpus -- test_data/reference
```

It prints a `PASS`/`FAIL` line per file with version, group and channel counts,
and reports the block map per file (block count, coverage percentage, warning count),
then a summary and totals. Over `test_data/reference` it reports 67 passed,
0 failed, 3,060 blocks and 0 warnings.

**Run it from the repository root.** The default paths are relative to the
working directory, so from inside `gui/` it finds nothing and reports
`0 passed, 0 failed, 0 total` rather than saying the corpus is missing.
