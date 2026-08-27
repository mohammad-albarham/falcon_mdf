# TASK: `iter_to_dataframe()` — streaming DataFrames

Work ONLY inside this worktree: `/Users/pain/Desktop/hoppy_projects/fmdf-wt/gpsmap`
(git branch `feat/iter-dataframe`). Never touch a file outside it.

## Why
`Mf4File.to_dataframe()` now works on the Python binding (see
`python/src/lib.rs`), but the streaming counterpart was left undone. The reason
given was accurate:

> the public binding surface exposes `signal_chunks` only per-channel, and a
> multi-channel streaming DataFrame iterator would need aligned chunking across
> channels

That is the actual work: aligned multi-channel chunking. Without it, a file
larger than memory cannot reach pandas at all, which is the whole point of the
streaming API asammdf offers as `iter_to_dataframe`.

## Deliverable
1. In the **Rust library**, an aligned multi-channel streaming iterator: given N
   channels from one channel group and a chunk size, yield successive windows
   where every channel's slice covers the **same sample index range**. Build it
   on the existing `signal_chunks` in `src/stream.rs` — read that module first;
   it already does the hard part per channel, including unsorted groups and
   VLSD. Do not write a second decoder.
   - Channels in different channel groups have different timebases and cannot be
     aligned by index. Refuse that **by name** rather than aligning them wrongly.
2. `Mf4File.iter_to_dataframe(chunk_size, channels=None, backend="pandas")` on
   the Python class, yielding one DataFrame per window, reusing the same Arrow
   IPC → pyarrow path `to_dataframe()` already uses.
3. Document it in `python/README.md` beside `to_dataframe()`.

## Tests
- **Rust:** assert the aligned iterator's concatenated output is exactly equal to
  the non-streaming `signals()` result for the same channels — same values, same
  order, same length. That is the property that makes streaming trustworthy.
- **Rust:** a chunk size that does not divide the sample count evenly (the
  uneven-tail case is where this class of bug lives in this repo's history).
- **Rust:** cross-group channels are refused by name.
- **Python:** extend `python/tests/test_smoke.py` — concatenating the yielded
  frames equals `to_dataframe()` for the same channels. Generate the fixture at
  runtime; **never commit a `.mf4`** — this repo does not commit measurement
  files. `pytest.skip` cleanly when pandas or pyarrow is absent.

## Constraints
- Do NOT modify `gui/`. Library changes confined to `src/stream.rs` (plus its
  re-export) and the binding.
- Peak memory must not scale with the file — that is the point. Do not decode
  everything and then slice it.
- `cargo build` at the worktree root must still pass.

## Verify (run these, report the real output)
- `cargo test --all-features`
- `cargo clippy --all-features --all-targets -- -D warnings`
- `cd python && .venv/bin/maturin develop && .venv/bin/python -m pytest tests/ -q`

Then `git commit` on branch `feat/iter-dataframe`.

## Report
When you are finished, print exactly this block as the last thing you output:

=== HERDR REPORT ===
STATUS: success | partial | failed
SUMMARY: <one line: what you actually did>
CHANGED: <files you modified, or "none">
VERIFIED: <command you ran and its result, or "not verified">
BLOCKER: <what stopped you, or "none">
=== END REPORT ===
