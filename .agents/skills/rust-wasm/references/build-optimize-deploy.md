# Build, optimize, deploy

Companion to SKILL.md. Covers the wasm-pack build matrix, binary size and
runtime tuning, and the GitHub Pages pipeline this repo deploys with.

## Build targets

`wasm-pack build --target <t>` decides what the glue JS looks like:

| Target | Shape | When |
|---|---|---|
| `web` | ES module; page awaits `init()` manually | static pages, no bundler — this repo's choice |
| `bundler` | consumed by webpack/vite/parcel, which handle instantiation | SPA built with a bundler |
| `nodejs` | CommonJS module for Node | CLI/SSR tooling |
| `no-modules` | globals + global `wasm_bindgen()` init | legacy plain `<script>` pages |

Switching targets regenerates the glue; always rebuild `pkg/` rather than
editing it, and never let a page load glue from one build and a `.wasm` from
another — the import names will not line up.

## Build modes

- `--dev` — fast, debuginfo kept, no wasm-opt. Use while iterating; use with
  the panic hook to get real backtraces.
- `--release` — optimized, `wasm-opt` pass applied. What CI deploys.
- `--profiling` — optimized *and* debuginfo, for profiling a release-like
  build.
- `--profile <name>` — a custom `[profile.<name>]` from Cargo.toml.

## Size and speed knobs

Profiles only take effect when defined in the **workspace root** manifest
(`wasm/` is a workspace member, so its own `[profile]` sections are ignored
with a warning):

```toml
[profile.release]
opt-level = "z"     # optimize for size; use 3 when the hot path is speed-bound
lto = true          # fat LTO — consistently large wins for wasm binaries
codegen-units = 1   # more optimization headroom, slower builds
strip = true        # symbols are not useful in the shipped .wasm
```

wasm-pack runs `wasm-opt` on release builds by default; the per-crate knob
(inside the binding crate's Cargo.toml) controls the level or disables it:

```toml
[package.metadata.wasm-pack.profile.release]
wasm-opt = ["-Oz"]  # or false to skip (offline builds, debugging)
```

Measuring before and after a change:

```bash
ls -l wasm/pkg/*_bg.wasm                          # raw size
twiggy top -n 20 wasm/pkg/*_bg.wasm               # biggest symbols (cargo install twiggy)
```

Notes:

- The first release build downloads the `wasm-opt` binary; offline builds
  need `wasm-opt = false` or `--no-opt`.
- `panic = "abort"` is already wasm's default panic strategy; setting it
  globally changes native release builds too, so only add it if the workspace
  accepts that.
- Dependency weight shows up in the `.wasm`: check with `cargo tree` before
  pulling in anything general-purpose (`regex`, `chrono`, `rand`) into a
  wasm-facing crate.

## The GitHub Pages pipeline

Annotated from `.github/workflows/pages.yml` — copy this shape for any
static wasm site:

```yaml
on:
  push:
    branches: [main]
    paths: ["wasm/**", ".github/workflows/pages.yml"]  # core-only changes don't rebuild the site
permissions:
  contents: read
  pages: write
  id-token: write
concurrency:
  group: pages
  cancel-in-progress: true                              # last push wins, no queue pileup

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-unknown-unknown               # install the target on CI
      - uses: Swatinem/rust-cache@v2
      - name: Install wasm-pack
        run: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
        # prebuilt binary; compiling wasm-pack from source costs minutes for nothing
      - run: wasm-pack build wasm --target web --release
      - run: |
          cp -r wasm/pkg wasm/demo/pkg                  # glue + .wasm beside the page
          touch wasm/demo/.nojekyll                     # Pages would otherwise run Jekyll,
                                                        # which eats files it considers special
      - uses: actions/upload-pages-artifact@v3
        with:
          path: wasm/demo
  deploy:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - id: deployment
        uses: actions/deploy-pages@v4
```

Deployment rules that come from the Pages environment:

- The site lives at `https://<owner>.github.io/<repo>/` — a **subpath**.
  Every asset URL in the page must be relative, or it 404s in production
  while working on localhost.
- `.nojekyll` is not optional for wasm sites.
- `pages.yml` and the wasm crate are the same release unit: a change to the
  binding without a matching demo change still republishes, so drive the
  demo locally (SKILL.md loop) before pushing.
