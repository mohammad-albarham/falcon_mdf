# TASK: Fix the aligned-streaming mismatch your own test found

Work ONLY inside this worktree: `/Users/pain/Desktop/hoppy_projects/fmdf-wt/gpsmap`
(git branch `feat/iter-dataframe`). Never touch a file outside it.

## What happened
Your aligned multi-channel streaming work was merged and then **reverted**,
because the test you wrote — `aligned_streaming_reproduces_eager_signals_across_corpus`
in `tests/stream_aligned.rs` — **fails** against the real sample corpus:

```
thread 'aligned_streaming_reproduces_eager_signals_across_corpus' panicked at
tests/stream_aligned.rs:138:29:
assertion `left == right` failed:
test_data/mf4-sample-data-v2.1/J1939 (truck)/LOG/958D2219/00002501/00002081.MF4
'CAN_DataFrame.DataBytes' bytes mismatch over 20791 window(s) of chunk_size 7
```

You reported it passing. It did not — **`test_data/` is gitignored, so it does
not exist in this worktree**, and the test skipped instead of running. Never
report a corpus test as passing from a checkout that has no corpus.

## Get the corpus first
```
bash scripts/fetch_reference_files.sh
```
Then confirm the file above exists before you start. If it still does not, say
so in BLOCKER and stop — do not guess at the fix.

## The bug
The failing file is a **J1939 truck log with an unsorted channel group**, and
the mismatching channel is a variable-length-ish frame payload
(`CAN_DataFrame.DataBytes`) read at `chunk_size 7` — an odd size that does not
divide the record count.

Aligned streaming must return **exactly** what eager `signals()` returns. It
does not, which means the window boundaries are wrong for at least the unsorted
case: an unsorted group's records are demultiplexed by record ID, so the *n*-th
record of the group is not the *n*-th record in the file. Aligning by a raw
record offset instead of by the channel's own demultiplexed sample index is the
likely cause — read how `signal_chunks` handles unsorted groups in
`src/stream.rs` and align on the same index it does.

The eager path is the reference. Where they disagree, **the streaming path is
wrong** — do not change the eager path or relax the test to make them agree.

## Constraints
- Keep the three properties the test suite already asserts: equality with eager
  `signals()`, correct uneven-tail behaviour, and named refusal for cross-group
  channels.
- Peak memory must still not scale with the file — do not "fix" this by
  decoding everything and slicing.
- Do NOT modify `gui/`. Do not weaken or delete the failing assertion.

## Verify — this is the whole point
Run the full suite **in this worktree with the corpus present** and paste the
real output:

- `bash scripts/fetch_reference_files.sh`
- `cargo test --all-features` — `aligned_streaming_reproduces_eager_signals_across_corpus`
  must actually RUN and PASS, not skip. Say explicitly in your report whether it
  ran or skipped.
- `cargo clippy --all-features --all-targets -- -D warnings`

Then `git commit` on branch `feat/iter-dataframe`.

## Report
When you are finished, print exactly this block as the last thing you output:

=== HERDR REPORT ===
STATUS: success | partial | failed
SUMMARY: <one line: what you actually did>
CHANGED: <files you modified, or "none">
VERIFIED: <command you ran and its result, and whether the corpus test RAN or SKIPPED>
BLOCKER: <what stopped you, or "none">
=== END REPORT ===
