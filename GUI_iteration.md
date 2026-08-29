# GUI publish-readiness iterations

A pass over `gui/` asking one question: if someone who has never seen this
repository downloads the viewer, does it hold up? Each iteration below is a
defect found, what was changed, and how the change was checked. Written
against `ba1e278` plus the uncommitted working tree it sat on.

The audit is recorded as honestly as the fixes: the last section lists what is
still not publish-perfect and was deliberately left alone.

---

## Baseline

Before changing anything, the suite was run against the real corpus. This
matters more than it sounds: the corpus (`test_data/`) is gitignored, and the
GUI's corpus tests **skip silently** when it is absent. A first run in a fresh
worktree reported passes that were nothing of the kind — several test binaries
had quietly matched zero files. Linking the corpus in first turned 4 reporting
binaries into 25.

| Check | Result |
| --- | --- |
| `cargo test -p falcon_mdf_gui` (corpus present) | 249 passed, 0 failed |
| `cargo run -p falcon_mdf_gui --example verify_corpus -- test_data/reference` | 67 passed, 0 failed, 0 warnings |
| `cargo clippy -p falcon_mdf_gui --all-targets -- -D warnings` | **failed** — see iteration 1 |

The code itself came out of the audit well. Every panel that polls a worker
thread already handles `TryRecvError::Disconnected` rather than hanging on it;
`session::prune_to_file` validates restored channel locations against the file
before they are used; `loader.rs` deliberately uses `open_buffered` over mmap
so a file truncated underneath the viewer cannot SIGBUS; the release profile
does not set `panic = "abort"`, so a worker panic unwinds into a message
instead of killing the process. The `.unwrap()`/`.expect()` sites are almost
all the "just built this, now read it back" shape, guarded one line above.

Two things were genuinely wrong, and three were missing rather than wrong.

---

## Iteration 1 — the GUI's CI job was red

**Found.** `gui/src/lib.rs` carried `#![allow(clippy::chunks_exact_to_as_chunks)]`.
No such lint exists on the pinned stable toolchain (1.97.1), so clippy emits
`unknown lint`. CI runs `cargo clippy -p falcon_mdf_gui --all-targets -- -D warnings`,
which promotes it to an error:

```
error: unknown lint: `clippy::chunks_exact_to_as_chunks`
  --> gui/src/lib.rs:11:10
error: could not compile `falcon_mdf_gui` (lib) due to 1 previous error
```

The GUI CI job could not have been passing. Shipping starts from green CI, so
this came first.

**Changed.** Removed the attribute. It was not merely misspelled but dead:
`grep -rn chunks_exact gui/src` matches nothing but the `allow` itself.

**Checked.** `cargo clippy -p falcon_mdf_gui --all-targets -- -D warnings`
now exits 0.

---

## Iteration 2 — a stale channel location panicked a worker thread

**Found.** `signal_loader::decode_channel` reached its channel by indexing:

```rust
let channel = &file.data_groups()[loc.data_group_index]
    .channel_groups[loc.channel_group_index]
    .channels[loc.channel_index];
```

A `ChannelLoc` outlives the file it was made against — a session restored
against a file that was rewritten shorter, a second file swapped under a plot.
`computed::eval_expr` also mints a deliberate sentinel location with
`data_group_index: usize::MAX` for constant expressions. Any of these reaching
this function is an out-of-bounds index, and the function runs on a worker
thread, so the panic surfaces to the user only as *"the worker thread ended
without a result"* — a message that names neither the channel nor the reason.

This is a latent crash, not an observed one: nothing routes a bad location here
today. It is also `pub` API of the GUI library, and "nothing routes it here
today" is what a crash in a released build is made of.

**Changed.** Look the channel up instead of indexing, and return the existing
`SignalLoadResult::Err` with a message naming all three indices.

**Checked.** New `gui/tests/stale_channel_location.rs`, 2 tests: every
out-of-range axis (data group, channel group, channel) and the computed-channel
sentinel. Both would panic against the old code — the test asserts on the error
message, and a panic in a test is a failed test.

---

## Iteration 3 — `falcon --version` tried to open a file called `--version`

**Found.** The launcher took `std::env::args().nth(1)` as a path, whatever it
was. The viewer ships as a standalone binary and so is run from a shell as
often as it is double-clicked, and a shell expects `--help` to work. It did
this instead:

```
$ falcon --version
[a window opens]  Failed to open file
                  --version
                  No such file or directory (os error 2)
```

That reads as broken, not as unsupported — a bad first thirty seconds for
someone evaluating the tool.

**Changed.** New `gui/src/cli.rs`, parsing into `Launch::{Window, Help, Version, Usage}`.
It lives in the library rather than in `main.rs` for the reason the rest of the
viewer's logic does: so it can be tested without spawning a process or opening
a window. `main.rs` now returns `ExitCode` and prints help/version to stdout,
usage errors to stderr.

Decisions worth naming:

- **`--help` and `--version` exit before any window is created**, so they work
  over SSH and on a headless CI runner. Iteration 5 leans on this.
- **A usage error exits 2, not 1** — a script wrapping the viewer can tell a
  bad argument from a file that would not open.
- **`--` is honoured, and a bare `-` is a file name.** A measurement can be
  called anything the filesystem allows.
- **Two files is an error naming both.** That case is nearly always a glob that
  matched more than the person expected, and seeing the pair says so.

**Checked.** 11 unit tests in the module, plus the built binary end to end:

```
$ ./falcon --version   → falcon 0.5.0                  exit 0
$ ./falcon --help      → usage text                     exit 0
$ ./falcon --plot      → unrecognised option '--plot'   exit 2
$ ./falcon a.mf4 b.mf4 → names both files               exit 2
```

No window opened in any of them.

---

## Iteration 4 — the binary did not know which release it was

**Found.** Iteration 3 made this visible: `falcon --version` printed
`falcon 0.1.0`, because `gui/Cargo.toml` had never moved off its initial
version. Releases are tagged `v0.3.0`, `v0.4.0` — the *library's* version, now
0.5.0. A binary inside a `v0.5.0` archive announcing 0.1.0 is worse than
announcing nothing: a bug report quoting it points at the wrong code.

**Changed.** `gui` version tracks the library's (0.5.0), with a comment at the
field saying why, so the next release bump does not silently drop it again.

**Checked.** `./falcon --version` → `falcon 0.5.0`, matching the tag a release
would be cut from.

---

## Iteration 5 — a tag published whatever compiled

**Found.** Three gaps in `.github/workflows/release.yml`, all of them things a
downloader feels rather than a maintainer:

1. **No gate.** A `v*` tag went straight to building and uploading. Tests red,
   clippy red, does not matter — if it compiled on the release runner, people
   downloaded it. Iteration 1 is exactly the state that would have shipped.
2. **No checksums.** The binaries are unsigned on all three platforms, so a
   checksum is the *only* integrity check a downloader has, and there was none.
3. **The archives carried the wrong document.** They shipped the root
   `README.md`, which is about the Rust library — the API, the benchmarks —
   and not `gui/RUNNING.md`, which is about the thing in the archive.

**Changed.**

- A `gate` job (fmt, build, test, clippy `-D warnings`) that every build
  `needs:`.
- A smoke test that runs the just-built binary with `--version` and `--help`
  before packaging it. Cheap, needs no display, and is the only proof in the
  pipeline that what is being packaged actually starts. Skipped for the
  x86_64 macOS build, which is cross-compiled on an arm64 runner where Rosetta
  may not exist.
- A `.sha256` generated per archive and uploaded beside it (`sha256sum` on
  Linux and Windows, `shasum` on macOS, which has no `sha256sum`).
- `gui/RUNNING.md` added to every archive, and `FIRST-RUN.md` to the macOS
  ones.

**Checked.** The YAML parses and the dependency edge is real
(`build-release-gui.needs == gate`). The packaging and checksum steps were run
locally against the built binary — archive contents correct,
`shasum -a 256 -c` reports `OK`.

One bug was caught in the new workflow before it could ship: the smoke-test
step first wrote `[ "$RUNNER_OS" = "Windows" ] && BIN="$BIN.exe"`, which
evaluates to false — and therefore exit 1 — on Linux and macOS. GitHub runs
`bash -e`, so that one-liner would have failed three of the four builds. It is
an `if` block now.

---

## Iteration 6 — the docs assumed you had the repository

**Found.** With `RUNNING.md` now shipped inside the archives, its first
instruction was `cargo run --release -p falcon_mdf_gui` — addressed to someone
holding a source tree, read by someone holding a binary. And nothing anywhere
told a macOS user what to do about the refusal they are guaranteed to hit:
Gatekeeper rejects an unsigned download and reports it as *damaged*, which
sends people to the issue tracker to report a corrupt build.

**Changed.**

- `gui/RUNNING.md` opens with **If you downloaded a release** — what is in the
  archive, the four commands, what each platform complains about and why
  (unsigned), and how to verify the `.sha256`. The build instructions follow,
  unchanged.
- New `gui/FIRST-RUN-MACOS.md`, shipped into the macOS archives as
  `FIRST-RUN.md`: why the refusal happens, the `xattr -d com.apple.quarantine`
  fix, the right-click-Open alternative, which archive matches which Mac, and
  the fact that this is a bare binary rather than a `.app`.
- `gui/PACKAGING.md` gains a **What a release ships** section covering the
  gate, the smoke test and the checksums, and its "How errors reach the user"
  section now also covers the two failures that happen before there is a
  screen to put text on — bad arguments, and a window that cannot be created.

**Checked.** Documentation only; no behaviour depends on it. The file paths
the workflow copies were confirmed to exist by running the packaging steps
locally.

---

## Final verification

Run in the worktree with the corpus linked in, on stable 1.97.1:

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | clean |
| `cargo clippy -p falcon_mdf_gui --all-targets -- -D warnings` | clean (was failing) |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test -p falcon_mdf_gui` | **262 passed, 0 failed** (was 249) |
| `verify_corpus -- test_data/reference` | 67 passed, 0 failed, 3060 blocks, 0 warnings |
| `falcon --version` / `--help` / bad args | correct output, exit 0 / 0 / 2 |
| release packaging + checksum, run locally | archive contents correct, `shasum -c` OK |

Net: 13 new tests, one CI job unblocked, one latent panic removed.

---

## Known gaps, deliberately not closed

These are real and a publisher should know about them. None is a small change,
and each is a decision rather than an oversight.

- **Nothing is signed.** No Apple Developer ID, no notarization, no Windows
  Authenticode. Every first run is a scary dialog. Iteration 6 documents the
  way past it, which is the best a repository can do; the fix is a paid
  certificate and a signing step in the release workflow.
- **Plotting materializes the whole channel.** Selecting a channel decodes all
  of its samples into memory before decimation runs, so a channel with
  hundreds of millions of samples can outgrow RAM before it draws. The library
  already has the streaming path (`Mf4File::signal_chunks`); the plot panel
  does not use it. Already recorded in `PACKAGING.md`.
- **Non-numeric channels plot as an empty chart with no explanation.**
  Strings, complex numbers, byte arrays and CANopen dates decode fine but have
  no number to plot, so they look identical to a broken channel. 41 channels
  across the 67-file corpus. `RUNNING.md` documents it; the honest fix is a
  line in the plot area saying "this channel has no numeric value", not a doc.
- **No macOS `.app` in the release.** `cargo bundle` is configured and works,
  but the workflow ships the bare binary, so double-clicking it in Finder
  opens a Terminal alongside the window.
- **The corpus tests skip silently.** A fresh clone runs the GUI suite, sees
  green, and has exercised none of the corpus paths — the trap this pass
  opened with. A CI-only guard that fails when `test_data/` is missing would
  close it; `benchmarks/` already has that shape of check.
