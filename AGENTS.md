# AGENTS.md

Project instructions for every coding agent working in this repository
(ZCode, Claude Code, OpenCode, Qwen, …). One page; read it before changing
anything.

## What this is

`falcon_mdf` — a Rust library for reading and writing ASAM MDF v3/v4
(`.mf4` / `.mdf`) measurement files, with a GUI (`gui/`), Python bindings
(`python/`), and a fuzz workspace (`fuzz/`). Performance is tracked against
asammdf, the Python reference implementation.

Start with the [Architecture section of the README](README.md#architecture)
for the module map (io → blocks → parser → model → `file.rs`) and
[README#performance](README.md#performance) for what the published numbers
mean and where they were measured.

## Repository layout

| Path | Contents |
|---|---|
| `src/` | Core crate: `file.rs` = `Mf4File` reader API, `write.rs` = writer, `export/` = CSV/Parquet/MAT/HDF5/ASC, `bus.rs`–`dbc.rs`–`arxml.rs`–`ldf.rs` = bus decoding, `mdf3/` = MDF3 reader |
| `tests/` | Integration tests, one file per concern, named accordingly |
| `examples/` | Runnable examples; `bench.rs` is the timing binary the benchmark harness drives |
| `benches/` | Criterion benchmarks (`cargo bench`) |
| `benchmarks/` | **Tracked** benchmark results; `COMPARISON.md` is the curated summary |
| `scripts/` | **Tracked** harness scripts (corpus fetch, asammdf comparison) |
| `.agents/skills/perf-benchmark/` | **Tracked** full benchmark methodology |
| `test_data/` | Gitignored vendor corpus; fetch via `scripts/fetch_reference_files.sh` |

## Verify before you call it done

A change is not finished until these pass locally — they mirror CI
(`.github/workflows/ci.yml`):

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo test --doc --all-features
cargo test -p falcon_mdf_gui
```

- `dbc` and `arxml` are off by default, so a default-only build proves
  nothing about the gated modules — always check both feature axes.
- Reference-corpus tests (golden values, cross-checks) skip silently when
  `test_data/` is empty. Fetch it first; a skipped test is not a passed test.
- The asammdf conformance tests need `.venv` with asammdf installed
  (`.venv/bin/python -c "import asammdf"`).
- The MSRV is declared in `Cargo.toml` (`rust-version`); CI enforces it.
- Do not push, publish, or tag without the owner's approval.

## Benchmarks (when touching performance)

Full methodology: [`.agents/skills/perf-benchmark/SKILL.md`](.agents/skills/perf-benchmark/SKILL.md).
Non-negotiables:

- Quote against asammdf's `select()`, never `get()` alone, and always state
  the file size and whether the file is DZ-compressed.
- After any benchmark run, refresh `benchmarks/COMPARISON.md` by hand, then
  run `.venv/bin/python .agents/skills/perf-benchmark/scripts/check_comparison.py`
  — the run is not finished until it prints `PASS`.
- Never quote a number that is not in one of the four generated files under
  `benchmarks/`.

## Working style

- Surgical changes: every changed line traces to the task. Don't reformat or
  refactor unrelated code; match the existing style.
- Comments in this codebase explain *why* — constraints, format quirks, CI
  history — never *what*. Keep it that way.
- Prefer the simplest approach that solves the problem; no speculative
  abstractions.
- Settle factual disagreements with a test, not an argument.

## Agent hygiene

- `.agent_reports/status.md` is a living verification snapshot. It must carry
  an "as of `<commit>`" header and be refreshed or deleted when stale — a
  wrong fact in it is worse than no file. `features.md` beside it dates from
  an older round; re-verify its line numbers before relying on them.
- Gitignored local-only material by design: `.claude/`, `.opencode/`, `.qwen/`,
  `.agent_reports/`, `notes.txt`, `plan_*.md`, `AUDIT.md`, and
  `.agents/skills/orca-cli/` (synced via `skills-lock.json`).
- Writers work in their own worktree (the workspace already excludes
  `.claude/worktrees`); research and review agents stay read-only in the
  shared checkout.
