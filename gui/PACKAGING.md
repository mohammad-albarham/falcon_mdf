# Building and packaging `falcon`

The GUI is a binary crate (`falcon`) in the workspace's `gui/` member; the
library it consumes stays at the repo root and carries no GUI dependency.
Everything here was written against eframe 0.35 / egui 0.35 / egui_plot 0.36.

## Release build

From the repository root:

```sh
cargo build --release -p falcon_mdf_gui
```

The binary lands at `target/release/falcon` and runs standalone on all three
platforms — it opens a file given as its first argument, or an empty window
otherwise, and accepts dropped files.

- **macOS**: builds with the default toolchain; no SDK steps beyond what
  `cargo` already requires. The binary is not signed or notarized — Gatekeeper
  will ask about it on first launch until it is.
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
