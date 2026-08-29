# Building and packaging `falcon`

The GUI is a binary crate (`falcon`) in the workspace's `gui/` member; the
library it consumes stays at the repo root and carries no GUI dependency.
Everything here was written against eframe 0.35 / egui 0.35 / egui_plot 0.36.

**The viewer is pre-1.0 and not stable**, and anything published has to say
so. It is stated in three places on purpose, because people arrive by
different routes: `falcon --help` (pinned by a test in `gui/src/cli.rs`, so it
cannot be dropped silently), the **Status** section at the top of
`RUNNING.md`, which ships inside every archive, and the release notes. What is
unstable is set out in `RUNNING.md` — the interface, the export formats, the
session store, and how evenly the vendor formats are covered.

## Release build

From the repository root:

```sh
cargo build --release -p falcon_mdf_gui
```

The binary lands at `target/release/falcon` and runs standalone on all three
platforms — it opens a file given as its argument, or an empty window
otherwise, and accepts dropped files. `--help` and `--version` answer and exit
without opening a window, so they work over SSH and in a packaging smoke test;
`gui/src/cli.rs` is where the arguments are read, and it is unit-tested.

- **macOS**: builds with the default toolchain; no SDK steps beyond what
  `cargo` already requires. The binary is not signed or notarized — Gatekeeper
  refuses a *downloaded* copy outright, and says the app is damaged rather
  than that it is unsigned. `FIRST-RUN-MACOS.md` is the answer shipped beside
  it in the archive; keep the two in step if the signing situation changes.
- **Linux**: needs the usual egui/wgpu prerequisites (`libxcb` or a Wayland
  toolchain, `libxkbcommon`); nothing MF4-specific.
- **Windows**: builds with the default MSVC toolchain; no extra steps.

## Bundling

`gui/Cargo.toml` carries `[package.metadata.bundle]` for
[`cargo bundle`](https://github.com/burtonageo/cargo-bundle):

```sh
cargo install cargo-bundle   # once
cargo bundle --release -p falcon_mdf_gui
```

- macOS: `target/release/bundle/osx/Falcon.app`, with `assets/icon.png`
  converted to an `.icns`.
- Linux: a `.deb` in `target/release/bundle/deb`, same PNG as the icon.
- Windows: `cargo bundle` produces a directory with the exe and an `.ico`;
  an installer (WiX, NSIS) is a separate step this project does not script.

The icon is generated, not drawn by hand: `assets/icon.png` (256×256, for
bundles) and `assets/icon.rgba` (64×64 raw RGBA, embedded by `main.rs` for
the runtime window icon so no PNG decoder is needed) come from the same
spike motif. Regenerate both together if the motif changes.

## What a release ships

`.github/workflows/release.yml` builds the four archives a `v*` tag publishes
(Linux x86_64, macOS arm64 and x86_64, Windows x86_64). Three things about it
are deliberate:

- **The `gate` job runs first, and every build waits on it.** It is the GUI's
  CI job — fmt, build, test, clippy with `-D warnings` — repeated at release
  time. A tag used to publish whatever compiled on the release runner, tests
  red or not.
- **Each archive is smoke-tested before it is packaged**, by running the
  binary it contains with `--version` and `--help`. Both answer without a
  display, so this works on a headless runner; it is the cheapest proof that
  what was packaged actually starts. The x86_64 macOS build is cross-compiled
  on an arm64 runner and is skipped, since Rosetta may not be there to run it.
- **Every archive is published with a `.sha256` beside it.** Nothing here is
  signed on any platform, so that checksum is the only integrity check a
  downloader has.

The archives carry the binary, both licences, the root `README.md`,
`RUNNING.md` (the viewer's own guide — the README is about the library), and
on macOS `FIRST-RUN.md`. `gui/Cargo.toml`'s version tracks the library's,
because the tag a release is cut from is the library version and
`falcon --version` has to name the release the binary came from.

## What the app persists

`eframe`'s `persistence` feature is enabled, so window position and size are
restored between runs from eframe's own storage, and the recent-files list is
saved through the same storage in `app.rs`. No other state is persisted by
design: a viewer that silently remembered plot state across files would show
stale data under a new file's name.

## How errors reach the user

There are no modal error dialogs; every failure is text that stays on screen
where the thing that failed would have been — an unopenable file names the
full `Mf4Error` in the main panel, an undecodable channel prints its reason
where its line would plot, and a failed export or attachment save leaves a
status line in the panel that ran it. A dialog that closes and takes the
message with it is exactly the failure mode this UI is built against.

Two failures happen before there is a screen to put text on, and those go to
stderr with a non-zero exit: arguments that make no sense (exit 2, so a script
wrapping the viewer can tell a usage error from a failed read) and a window
that cannot be created at all — a machine with no display, a broken driver, a
missing Wayland socket.

## Known limitations

- **Plotting materializes the whole channel.** Selecting a channel decodes
  all of its samples into memory (`times`, `values`, validity), and the
  decimation that keeps drawing cheap happens afterwards, not instead. A
  channel with hundreds of millions of samples can outgrow physical RAM
  before it is drawn. The library's windowed reader
  (`Mf4File::signal_chunks`) is the intended path for streaming decimation;
  the plot panel does not use it yet.
- Exports and attachment saves run on worker threads, but their results are
  still produced whole — a CSV export of a multi-gigabyte decode writes the
  entire file before reporting back.
