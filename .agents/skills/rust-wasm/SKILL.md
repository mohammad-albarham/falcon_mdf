---
name: rust-wasm
description: >-
  Build, extend, test, optimize, and deploy Rust code compiled to WebAssembly
  with wasm-bindgen and wasm-pack — bindings crates, browser demos, and the
  JavaScript that drives them. Use whenever a task mentions wasm, WebAssembly,
  wasm32, wasm-bindgen, wasm-pack, pkg/, a browser demo, or JavaScript calling
  into Rust — even when the word "wasm" never appears, e.g. "expose this
  parser to the browser", "make the demo open files faster", or "deploy the
  viewer to Pages".
---

# Rust → WebAssembly (wasm-bindgen + wasm-pack)

Reference for building browser-facing WebAssembly apps from Rust: a thin
`#[wasm_bindgen]` binding crate, a static HTML/JS page that loads it, and a CI
pipeline that publishes both. The patterns are grounded in this repository's
`falcon-mdf-wasm` crate but apply to any Rust→wasm app built the same way.

## Start from the living example

Before writing anything, read the working end-to-end implementation in this
repo — it answers most "how should I structure X" questions by example:

| Concern | File |
|---|---|
| Binding crate: `#[wasm_bindgen]` API, JSON escaping, NaN → null | `wasm/src/lib.rs` |
| Crate manifest: `cdylib` + `rlib`, workspace wiring | `wasm/Cargo.toml` |
| Demo page JS: `init()` await, yielding before long calls, canvas | `wasm/demo/main.js` |
| Build + GitHub Pages deploy pipeline | `.github/workflows/pages.yml` |
| Documented public JS API | `wasm/README.md` |
| Native tests for the binding's JSON output | `wasm/tests/json.rs` |

`wasm/pkg/` and `wasm/demo/pkg/` are generated output — never hand-edit or
commit them; rebuild instead.

## Toolchain

Check before starting; install what is missing:

```bash
rustup target list --installed | grep wasm32-unknown-unknown \
  || rustup target add wasm32-unknown-unknown
wasm-pack --version \
  || curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
```

- `wasm-pack` bundles its own compatible `wasm-bindgen` CLI. Do **not**
  `cargo install wasm-bindgen-cli` to fix a version complaint — upgrade
  wasm-pack instead. (The CLI version must match the `wasm-bindgen` *crate*
  version in Cargo.lock; wasm-pack fetches a matching CLI itself.)
- `rust-toolchain.toml` pins the channel; `rustup target add` installs the
  wasm target for that toolchain.

## The build → serve → verify loop

```bash
# from the repo root
wasm-pack build wasm --target web --release
rm -rf wasm/demo/pkg && cp -r wasm/pkg wasm/demo/pkg
python3 -m http.server 8000 -d wasm/demo
# open http://localhost:8000, load the sample, watch the console
```

Why each step is what it is:

- `--target web` emits an ES module usable with a plain
  `<script type="module">` — no bundler required.
- The demo imports `./pkg/falcon_mdf_wasm.js` **relative** to the page, so the
  glue and the `.wasm` must sit inside `demo/pkg/`. That is the same layout
  the Pages workflow assembles (`cp -r wasm/pkg wasm/demo/pkg`), so local
  testing exercises what will deploy.
- Serving over HTTP is mandatory, not a nicety: browsers block ES module
  imports and the `.wasm` fetch on `file://`. Any static server works.
- The first `--release` build downloads a `wasm-opt` binary; it needs network
  access (skip with `--no-opt` when offline or debugging).

## Binding rules (Rust side)

The binding crate is a thin adapter over the core crate (`src/`). Business
logic belongs in the core; the binding owns only the JS boundary.

**Expose a class when there is state.** JS gets a class whose instances own
Rust memory (each has a `.free()` for deterministic release):

```rust
#[wasm_bindgen]
pub struct WasmMf4File {
    inner: Mf4File,
}

#[wasm_bindgen]
impl WasmMf4File {
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: Vec<u8>) -> Result<WasmMf4File, JsValue> {
        let inner = Mf4File::from_bytes(bytes).map_err(js_err)?;
        Ok(WasmMf4File { inner })
    }
}
```

**Errors.** Return `Result<T, JsValue>` and convert with
`JsError::new(&msg).into()`. A Rust *panic* is not an error you can catch on
the JS side — it traps the instance (`RuntimeError: unreachable` in the
console). See `references/testing-debugging.md` for the panic hook.

**Structured data as JSON strings** (this repo's pattern): return a `String`
the JS side `JSON.parse`s. Zero extra dependencies, one documented shape —
but it imposes two rules the binding must enforce:

- Strings must be JSON-escaped (reuse `escape_json_str_into` in
  `wasm/src/lib.rs`; it handles RFC 8259 escapes and control characters).
- Non-finite floats (`NaN`, `±inf`) must be emitted as `null` because JSON
  has no representation for them (see `write_f64`). The JS side must treat
  `null` as "no sample" in every loop over the arrays.

For deep or rapidly evolving schemas, `serde` + `serde-wasm-bindgen` is the
standard alternative (comparison in `references/bindgen-patterns.md`).

**Numbers.** `f64`, `f32`, `u32`/`i32` and smaller map to plain JS numbers.
`u64`/`i64` surface as `BigInt` — prefer `f64`/`u32` APIs unless the range is
genuinely needed.

**Binary.** A `Vec<u8>` parameter accepts a JS `Uint8Array`; the data is
copied across the boundary, which is the right default for one-shot file
loads. Zero-copy patterns (raw pointer + length, `Uint8Array::view` for
output) live in `references/bindgen-patterns.md` — reach for them only when a
profile shows the copy matters.

**Design the API per-call, not chatty.** One call returning a whole series
beats per-sample getters: every boundary crossing copies and validates.

## JS side rules

The shape from `demo/main.js`:

```js
import init, { WasmMf4File } from "./pkg/falcon_mdf_wasm.js";

let ready = false;
let file = null;

async function boot() {
  await init();        // must resolve before ANY export is touched
  ready = true;
}

// file input or drag-drop:
const bytes = new Uint8Array(await f.arrayBuffer());   // or fetch().arrayBuffer()
try {
  file = new WasmMf4File(bytes);
} catch (e) {
  showError(e.message ?? e);
}
const info = JSON.parse(file.info());
```

- **Every wasm call is synchronous** and freezes the tab for its duration.
  Before a potentially long call, yield so status text actually paints:
  `await new Promise(r => setTimeout(r, 30));`
- All asset URLs must be **relative** (`./pkg/...`, `sample.mf4`). The site is
  served from a subpath on GitHub Pages; an absolute `/pkg/...` URL 404s there
  even though it works on localhost.
- Guard on `ready`: calls into a not-yet-`init()`ed module fail confusingly
  ("exports is undefined"), so gate user actions on the initialized flag.

## Pitfalls that actually bite

| Symptom | Cause → fix |
|---|---|
| Blank page or `Failed to fetch dynamically imported module` on `file://` | ES modules + wasm fetch need HTTP → serve with a static server |
| `RuntimeError: unreachable` | Rust panic trapped → enable `console_error_panic_hook`, reproduce with `--dev` |
| Exports are `undefined` / "not a function" | called before `await init()`, or glue and `.wasm` from different builds → rebuild pkg, never mix old glue with new wasm |
| `wasm-pack build` fails complaining about wasm-bindgen version | bundled CLI older than the crate in Cargo.lock → upgrade wasm-pack |
| `getrandom`/`rand` fails to compile for wasm32 | enable the js feature: `getrandom = { version = "0.2", features = ["js"] }` (renamed `wasm_js` in getrandom 0.3) |
| `std::time::Instant` misbehaves | works on wasm32-unknown-unknown since Rust 1.67, but the `web-time` crate maps to `performance.now()` on all platforms — prefer it |
| Adding `rayon`/`std::thread` breaks the build or hangs | plain wasm32 has no threads (needs atomics + cross-origin isolation); keep the binding crate single-threaded |
| Tab freezes during parse | expected — synchronous wasm; yield before the call, or move to a Web Worker (`references/bindgen-patterns.md`) |
| Deployed site 404s on assets | absolute paths under the Pages subpath, or missing `.nojekyll` |

## Extending the app — checklist

1. Core logic changes go in the core crate (`src/`), not the binding.
2. Add or extend the `#[wasm_bindgen]` method; keep it thin.
3. `cargo test -p falcon-mdf-wasm` — the `rlib` target runs `wasm/tests/`
   natively, no browser needed. Assert on the JSON output shape.
4. `cargo clippy -p falcon-mdf-wasm --all-targets -- -D warnings`
   (repo CI gate; the AGENTS.md feature-axis rule applies to the core crate).
5. Rebuild and copy pkg into the demo (commands above), drive the page, check
   the console is clean.
6. Update `wasm/README.md` whenever the public JS API changed.
7. Pushing triggers `.github/workflows/pages.yml` on `wasm/**` changes — but
   AGENTS.md forbids pushing without the owner's approval.

## Going deeper

Read on demand:

- `references/bindgen-patterns.md` — full Rust↔JS type map, async/Promise
  methods, closures and event listeners, zero-copy buffers, Web Workers,
  serde-wasm-bindgen.
- `references/build-optimize-deploy.md` — dev vs release builds, size/speed
  tuning (LTO, opt-level, wasm-opt), all wasm-pack targets, the GitHub Pages
  pipeline annotated.
- `references/testing-debugging.md` — native-first test strategy,
  wasm-bindgen-test in a headless browser, panic hook, devtools trap decoder.

Official documentation: [Rust → WebAssembly book](https://rustwasm.github.io/docs/book/),
[wasm-bindgen guide](https://rustwasm.github.io/docs/wasm-bindgen/),
[wasm-pack book](https://rustwasm.github.io/docs/wasm-pack/), API references
on docs.rs (`wasm-bindgen`, `js-sys`, `web-sys`), and
[MDN WebAssembly](https://developer.mozilla.org/en-US/docs/WebAssembly) for
the JavaScript-side memory/instantiation model.
