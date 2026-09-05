# Testing and debugging wasm

Companion to SKILL.md. Strategy first: test the logic natively, test the
boundary in a browser, debug with the panic hook and the trap decoder.

## Native-first testing (this repo's strategy)

The crate keeps `rlib` in `crate-type`, so the binding crate compiles for
the host target and plain `cargo test` works — no browser, no wasm-pack:

```bash
cargo test -p falcon-mdf-wasm
```

`wasm/tests/json.rs` is the model: it builds a file in memory, calls the
`#[wasm_bindgen]` API, and asserts the **exact JSON output** with a small
parser. Anything that does not need real JS semantics (parsing logic, JSON
escaping, NaN → null emission) belongs here — it runs in milliseconds and in
CI without a browser. The dual-target `js_err` helper (see
`bindgen-patterns.md`) is what makes the same code run natively.

## Browser tests: wasm-bindgen-test

Only for behavior that exists solely in a browser: `fetch`, DOM, real event
loop, `console` output.

```toml
[dev-dependencies]
wasm-bindgen-test = "0.3"
```

```rust
// wasm/tests/browser.rs — run with wasm-pack test, NOT cargo test
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn opens_a_file() {
    let f = WasmMf4File::new(test_bytes());
    assert_eq!(f.channel_count(), 3);
}
```

```bash
wasm-pack test --headless --chrome    # or --firefox / --safari; drivers auto-managed
```

wasm-pack launches a real browser headlessly, serves the test bundle, and
reports each `#[wasm_bindgen_test]` on stdout. Keep these files out of the
default `cargo test` path (a dedicated file name like `browser.rs` without
`#[test]` items does not run natively — `#[wasm_bindgen_test]` is not `#[test]`).

## Panics: console_error_panic_hook

Without help, a Rust panic on wasm is a bare `RuntimeError: unreachable` —
no message, no location. `console_error_panic_hook` prints the real panic
message and a backtrace to the devtools console:

```toml
# [dependencies] — small, no_std-friendly; fine in release too
console_error_panic_hook = "0.1"
```

```rust
#[wasm_bindgen(constructor)]
pub fn new(bytes: Vec<u8>) -> Result<Self, JsValue> {
    #[cfg(debug_assertions)]
    console_error_panic_hook::set_once();
    ...
}
```

`cfg(debug_assertions)` keeps the hook in `--dev` builds (where you debug)
and out of the shipped release binary. Installing it once anywhere early
(`set_once`) is enough — it is global.

## Logging from Rust

```toml
[dependencies]
web-sys = { version = "0.3", features = ["console"] }
```

```rust
web_sys::console::log_1(&format!("channels={}", n).into());
```

`web-sys` is feature-gated per browser API — request only what you call;
every enabled feature grows the binary.

## Trap decoder

What the devtools console is actually telling you:

| Console says | Meaning → fix |
|---|---|
| `RuntimeError: unreachable` | Rust panic trapped. With the hook installed you get the message + backtrace; without it, reproduce under `wasm-pack build --dev` |
| `memory access out of bounds` | wasm stack overflow or heap exhaustion — usually a huge allocation (multi-GB file) or runaway recursion |
| `import ... not found` / `... is not a function` | glue and `.wasm` from different builds, or code ran before `await init()` resolved. Rebuild `pkg/` wholesale |
| `TypeError: Failed to fetch ..._bg.wasm` | page served from `file://`, wrong relative path, or `pkg/` split across directories |
| `recursive use of an object detected` | a `#[wasm_bindgen]` method that borrows the object was re-entered (JS callback during the call touched the same object). Restructure so the callback does not re-enter, or clone the needed data out first |
| instance used after `.free()` | freeing does not null the JS reference; using it is undefined behavior and may abort. Treat `free()` as consuming: drop the JS reference in the same code path that frees |

## Debug builds and inspection

- Iterate with `wasm-pack build --dev`: debuginfo kept, `wasm-opt` skipped,
  panic hook active. The binary is several times larger — fine on localhost.
- Chrome's devtools WebAssembly inspector shows imports/exports and, with
  debuginfo present in the `.wasm`, maps frames back to Rust source.
- `--profiling` gives release optimizations *with* debuginfo for perf work
  where a dev build's slowness would distort the numbers.
- To watch a long parse interactively: log stage boundaries from Rust
  (`console::log_1`) rather than stepping — wasm debuggers stop on
  instructions, not lines.
