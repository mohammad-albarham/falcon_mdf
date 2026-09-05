# MF4 reader and viewer review

Reviewed 2026-09-05 against base commit
`5cf939c478c318251d85492c7b192fd0aa7ce1f8`. Changes described here are the
working-tree enhancement pass based on that commit.

The [ASAM MDF wiki](https://www.asam.net/standards/detail/mdf/wiki/) supplies
the format overview; [asammdf](https://github.com/danielhrisca/asammdf) supplies
an independent implementation and local conformance oracle. The local Python
environment uses asammdf 8.7.2. The full versioned ASAM specification was not
available for this review, so this is not a certification of MDF compliance.

## Findings addressed

| Area | Reproduced problem | Result and regression |
| --- | --- | --- |
| Core block parsing | A short link table with a huge declared count caused a capacity panic; offset addition could overflow; saturated header arithmetic accepted an impossible layout | Validate actual bytes before allocating and use checked header arithmetic. `tests/link_bounds.rs` failed four cases before the fix |
| GUI scalar loading | Arrays flattened into more values than timestamps, violating plot assumptions | Report an explicit array limitation and direct the user to the typed Table view. `gui/tests/master_axis.rs` |
| GUI master axis | Invalid, non-finite or decreasing master values reached binary-search consumers | Report the master and sample index before using the axis. Do not fabricate a clock or reorder data. Valid scalar channels still decode |
| Browser comparison | Raw and kind caches used bare names across different files | Scope caches to the file object. Same-name channels retain independent cursor/table/XY values and types |
| Browser text windows | Windowed labels were sent beside the full timestamp array | Return timestamps from the same window as the labels; preserve full extents separately |
| Browser file lifetime | Replacing files did not call their WASM `free()` methods | Free replaced files only after successful construction; a new primary clears its comparisons |
| Browser XY | Pair count was capped by the shorter series even though Y is sampled at X timestamps; transfer lists named unused buffers | Use all X samples when Y is nonempty and transfer the arrays actually included in the reply |
| Reference material | README contradicted implemented writer/LD capabilities; agents lacked a focused MDF guide | Correct the specific support statements and add `.agents/skills/mf4-structure`, linked from AGENTS.md |

The browser behaviors are covered by `wasm/tests/worker.test.mjs`, which
executes the shipped handler and replaces only the generated WASM boundary.
Its original six cases all failed before the fixes. CI now runs this suite.

## Prioritized next work

1. **Explicit channel locations throughout the browser API.** The core and GUI
   retain locations, while `decode_v4` in `wasm/src/lib.rs` resolves a name
   with `find_channel`. Per-file cache isolation fixes comparisons but cannot
   disambiguate repeated names within one file. Acceptance: independently
   select, inspect, plot and export every occurrence, including session reload.
2. **Windowed browser reads and a bounded decode cache.** `rawSeries` obtains
   a full `signal_arrays` result before plotting a window; Rust also caches
   decoded samples. Opening is off the UI thread, but the memory cost still
   scales with selected full signals. Acceptance: bounded memory on large
   compressed and uncompressed files, peak-preserving windows, and cursor
   lookup against original samples. Measure with the performance skill before
   claiming a speedup or changing published numbers.
3. **Honest rewriting diagnostics.** `Mf4Writer::from_file` skips some
   unreadable/unrepresentable channels and can substitute sample indices after
   master-decode failure. Add a report of every omission/substitution, with a
   strict mode for callers requiring preservation. Recheck raw values,
   conversions, units, validity and metadata with an independent reader.
4. **GUI array element selection and text plots.** The typed Table already
   exposes non-scalar data; scalar plotting needs an explicit element or text
   representation. Acceptance: preserve sample count, shape and element
   validity, including empty/dynamic array samples.
5. **Shared master-axis validity policy.** This pass protects GUI scalar
   consumers. Core time-series and browser operations need a consistent policy
   for invalid/nonmonotonic masters that preserves raw-table access. Cover
   valid values paired with invalid masters, gaps, duplicate timestamps and
   non-time synchronization domains.
6. **Broader external conformance.** Existing 4.20 LD/DV conformance and
   synthetic invalidation cases do not establish every vendor/version/layout.
   Expand the matrix with independent split-invalidation, arrays, VLSD and
   compressed unsorted fixtures.

## Verification scope

All required AGENTS.md checks passed locally: formatting, default and
all-feature Clippy, 782 core tests, 35 documentation tests (five ignored
examples), and 267 GUI tests. GUI Clippy also passed. The vendor corpus was
present and the conformance tests used the local asammdf environment.

The native WASM suite passed 90 tests; its all-target Clippy check passed
after narrow cleanup of existing warnings, preserving unordered floating-point
comparison behavior. All six worker regressions passed, and the release
`wasm-pack` build succeeded. A real browser smoke test exercised the bundled
sample and asammdf-generated comparison files, including cursor/table values,
text-window alignment and XY pairing, with an empty browser console.

This pass makes no new throughput or memory measurements and does not change
published benchmark claims. It fixes demonstrated correctness and ownership
problems; it is not a claim that Falcon now implements every MDF feature or
outperforms every alternative.
