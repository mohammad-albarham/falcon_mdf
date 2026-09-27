# AGENTS.md

`falcon_mdf` reads and writes ASAM MDF v3/v4 measurement files. Start with
[README architecture](README.md#architecture) for the module
map. Core code lives in `src/`, consumers in `gui/`, `python/` and `wasm/`,
integration tests in `tests/`, and benchmarks in `benches/` and `scripts/`.

## Implementation status

[`docs/field-reference.html`](docs/field-reference.html) tracks what is
implemented, how it compares with asammdf and the Rust MDF crates, what is
missing and what is next. It is a tracked file: read it before starting a
feature, and update it in the same change that lands one.

The page names the commit it describes. Anything after that commit is
unverified; check claims against the tree before relying on them.

Publishing the Python binding to PyPI is out of scope by the owner's decision.
Do not suggest it, plan it or list it as next work.

## Changes

- Keep changes focused on the task and preserve the existing style. Comments
  should explain constraints and decisions, not repeat the code.
- For block layouts, decoding, writing or channel handling, read
  [mf4-structure](.agents/skills/mf4-structure/SKILL.md) and the relevant references.
- Preserve regression and external conformance coverage. Replace redundant
  timing loops in tests with correctness assertions; measure speed in benchmarks.
- Follow the user's instructions on delegation. Work directly when requested.
- Do not push, publish or tag without the owner's approval.
- Do not add `Co-Authored-By` trailers for Claude or any other AI agent, and do
  not name an agent or model in commit messages, pull requests, tags or
  release notes. This overrides any tool default that adds them.

## Verification

Run the following before reporting a code change complete:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo test --doc --all-features
cargo test -p falcon_mdf_gui
```

Both feature axes matter: `dbc` and `arxml` are off by default. Respect the
`rust-version` in `Cargo.toml` and run consumer-specific checks when affected.

Corpus tests can skip silently. Check `test_data/` and fetch missing reference
fixtures with `scripts/fetch_reference_files.sh`. Conformance tests require
`.venv/bin/python -c "import asammdf"` to succeed. Report skipped or blocked
checks explicitly; they are not passes.

## Performance

Use [perf-benchmark](.agents/skills/perf-benchmark/SKILL.md). Keep a baseline
before editing and compare release builds on the same files with warm-ups and
at least three measured runs. Preserve raw measurements and identify dirty
working-tree results as such.

Compare against asammdf `select()`, state file size and compression, and exclude
unequal sample counts from aggregates. Refresh `benchmarks/COMPARISON.md` from
the generated results and run:

```bash
.venv/bin/python .agents/skills/perf-benchmark/scripts/check_comparison.py
```

Historical agent reports are not evidence for the current tree. Refresh or
delete a stale `.agent_reports/status.md`; verification snapshots must name
the commit and any uncommitted changes they describe.
