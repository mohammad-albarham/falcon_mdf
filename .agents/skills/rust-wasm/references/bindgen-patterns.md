# wasm-bindgen patterns

Deep-dive companion to SKILL.md. Read the section you need; every pattern is
cross-checked against `wasm-bindgen` 0.2.x behavior.

## The type map

What crosses the boundary, and how:

| Rust | JavaScript | Notes |
|---|---|---|
| `u8`–`u32`, `i8`–`i32` | `number` | copied |
| `u64` / `i64` | `BigInt` | copied — prefer `f64`/`u32` APIs to avoid BigInt friction |
| `f32`, `f64` | `number` | copied; non-finite passes through as `NaN`/`Infinity` |
| `bool` | `boolean` | |
| `char` | `string` (length 1) | |
| `String`, `&str` | `string` | copied with a UTF-16 conversion |
| `Vec<u8>`, `Box<[u8]>` | `Uint8Array` | copied |
| `Vec<T>` (other Copy `T`) | `Array` | copied |
| `Option<T>` | `T`, `null`, or `undefined` | |
| `Result<T, JsValue>` | `T` or a thrown `Error` | |
| `#[wasm_bindgen] struct` | class instance | owns Rust memory; `.free()` releases it |
| `JsValue` | any | escape hatch when nothing else fits |

Every crossing copies unless you use the zero-copy patterns below. Copying is
cheap relative to parsing — don't optimize it away until measured.

## Error handling

The repo's dual-target pattern (`wasm/src/lib.rs`): on wasm throw a real JS
`Error`; on native test builds return null so `map_err` still compiles:

```rust
fn js_err(err: impl std::fmt::Display) -> JsValue {
    #[cfg(all(target_arch = "wasm32", not(target_os = "emscripten")))]
    {
        JsError::new(&err.to_string()).into()
    }
    #[cfg(not(all(target_arch = "wasm32", not(target_os = "emscripten"))))]
    {
        let _ = err;
        JsValue::NULL
    }
}
```

`JsError` (in `wasm_bindgen::prelude`) is the easy route — it implements
`Into<JsValue>` and produces a proper JS `Error` with `message` and stack.
Plain `JsValue::from_str("msg")` throws a bare string, which breaks
`e.message ?? e` handling on the JS side.

Panics never become catchable JS errors. They abort the instance; see
`testing-debugging.md` for the panic hook and the trap table.

## JSON-string API vs serde-wasm-bindgen

The JSON-string pattern (SKILL.md) keeps the dependency tree at zero and
makes the wire format explicit and documented. Its helpers live in
`wasm/src/lib.rs` (`escape_json_str_into`, `write_f64`) — always reuse them;
hand-rolled string interpolation is where injection bugs come from.

Switch to `serde-wasm-bindgen` when the schema is deeply nested or changing
often and hand-writing the encoder costs more than the two dependencies:

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
serde-wasm-bindgen = "0.6"
```

```rust
#[wasm_bindgen]
pub fn info(&self) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&self.inner.info()).map_err(Into::into)
}
```

JS receives an ordinary object — no `JSON.parse`. Do **not** use
`serde_json::to_string` + `JSON.parse` as a middle ground; it is the string
pattern with extra dependencies.

## Async methods (Promises)

With `wasm-bindgen-futures` in dependencies, an `async fn` export returns a
real JS `Promise`:

```rust
#[wasm_bindgen]
pub async fn load_from_url(url: String) -> Result<WasmMf4File, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsError::new("no window"))?;
    let resp = JsFuture::from(
        window.fetch_with_str(&url),
    ).await.map_err(js_err)?;
    // resp is a Response; check status, then array_buffer() ...
}
```

Requirements and cautions:

- Add `wasm-bindgen-futures = "0.4"` and the `web-sys` features you touch
  (`["Window", "Request", "Response", "fetch"]` for the above).
- `spawn_local` runs fire-and-forget futures on the main thread.
- The browser cannot interrupt Rust code — an `async` method that runs a
  long synchronous parse still freezes the tab between awaits. Split work
  with awaits, or move it to a Worker.

## Closures and event listeners

JS callbacks into Rust go through `Closure`, which pins the Rust callbable
and hands JS a stable function reference:

```rust
use wasm_bindgen::closure::Closure;

let cb = Closure::wrap(Box::new(move || {
    // runs on every tick
}) as Box<dyn FnMut()>);

window.set_interval_with_callback(cb.as_ref().unchecked_ref(), 1000)?;
```

The lifetime rule that bites: **the `Closure` must outlive every JS
reference to it.** Dropping it while a listener/interval still points at it
breaks the callback. Either store the `Closure` in a long-lived struct member
(cleared when the listener is removed) or deliberately leak with
`cb.forget()`. Forgetting is acceptable for one global hook; prefer explicit
ownership everywhere else.

## Zero-copy buffers

Default to the copying patterns (`Vec<u8>` in, `String` out). Go zero-copy
only when a profile shows the copy matters — inputs in the hundreds of MB.

**Input without a copy** — hand JS a pointer+length API and build the view
on the JS side:

```rust
#[wasm_bindgen]
pub fn parse_ptr(ptr: *const u8, len: usize) -> Result<WasmThing, JsValue> {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    ...
}
```

```js
const view = new Uint8Array(wasmMemory.buffer, ptr, len);
```

Dangers, both real: growing wasm memory **detaches** `memory.buffer`
(a view taken earlier goes dead), and the pointer is only valid while the
producer object lives. Copy the view JS-side (`view.slice()`) if wasm will
allocate during the parse.

**Output views** — `js_sys::Uint8Array::view(&[u8])` and
`js_sys::Float64Array::view(&[f64])` build a JS typed array over borrowed
Rust data without copying, but the JS object's validity is tied to the Rust
borrow: hand it out only within the call, never store it.

## Web Workers

A long synchronous parse is the one honest reason to reach for a Worker: the
UI stays alive while the worker's instance chews. With `--target web`, the
worker imports the same ES module glue and awaits its own `init()` — the
module is instantiated once per realm, so the worker holds an independent
instance (re-parse the file there; instances are not transferable).

Parallelism (`rayon` on wasm via `wasm-bindgen-rayon`) additionally requires
`+atomics`, `+bulk-memory`, shared memory, and cross-origin isolation
(COOP/COEP headers). GitHub Pages cannot set those headers — on Pages the
app stays single-threaded. Do not add threaded dependencies to the binding
crate "for later".
