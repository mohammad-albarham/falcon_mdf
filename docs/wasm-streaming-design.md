# Wasm huge-file streaming — design (plan 4.6)

Status: **design first, implementation pending** — this document is the
"design doc first, then binding" gate the plan puts on 4.6, because it is
the one wasm feature that must touch `src/` (the core crate).

## Problem

The viewer opens files with `WasmMf4File::new(bytes)` — the whole recording
must fit in wasm linear memory (plus its decoded series). A 2 GB log cannot
be shipped to the browser in one `fetch().arrayBuffer()` anyway (browsers
cap `ArrayBuffer` well below disk sizes users have), and even when it can,
duplicating it into wasm memory doubles the footprint. The native GUI gets
this for free from mmap; the web equivalent is **on-demand block reads**.

## Core-side shape: a read-callback source

The core already abstracts its data source: `src/io/mod.rs` defines

```rust
pub trait ByteSource: Send + Sync {
    fn len(&self) -> u64;
    fn read_bytes(&self, offset: u64, len: usize) -> Result<Vec<u8>>;
    // …
}
```

and every reader (MDF4 and MDF3) walks blocks through it. The streaming
design therefore needs **no new reader entry points** — it needs one new
constructor-shaped helper that lets a host supply reads asynchronously:

```rust
// src/io/mod.rs (new)
/// A `ByteSource` whose bytes the host fetches on demand. `read` is called
/// with (offset, len) and must resolve to exactly those bytes.
pub fn remote_source(
    len: u64,
    read: impl Fn(u64, usize) -> Result<Vec<u8>> + Send + Sync + 'static,
) -> Arc<dyn ByteSource>
```

Two hard constraints shape this:

1. **wasm-bindgen closures are not `Send`.** The host callback must live
   behind a channel: the `ByteSource::read_bytes` implementation (running
   synchronously inside wasm, on the worker thread) posts a read request to
   JS, and JS resolves it before wasm continues. With `wasm-bindgen`'s
   `closure` + `JsValue` promise bridging this means the read is a **blocking
   spin over a `SharedArrayBuffer`+`Atomics.wait` mailbox** (same-origin
   COOP/COEP headers required) — this is the design's central bet, and the
   reason the feature is gated behind a design round: without SAB support
   the fallback is chunked `fetch()` per block, which multiplies latency by
   the block count of a random-access walk.
2. **Block granularity.** MDF block reads are linked-list walks of
   HL/DH/DG/CG/CN/DT blocks plus the DT data itself. A fixed 64 KiB read
   granularity (rounding every `read_bytes` up to block-aligned 64 KiB
   windows, cached in a small LRU of windows) keeps request count within
   ~2× the block count for typical files while amortising network cost.

## Viewer-side shape

- `WasmMf4File::open_streaming(len, read_callback)` mirrors `new(bytes)`:
  same API surface after construction, zero bytes held up front.
- The worker fetches a `Response` once, keeps it, and services reads with
  `response.body.getReader()` reads against a range-request URL
  (`Range: bytes=a-b`), so the server needs nothing but byte-range support
  (every static file server has it).
- Landing UX: a URL field on the drop zone ("open from URL, streamed"), so
  the file is never downloaded twice and never fully resident.
- The Rust-side LRU of decoded series (`SERIES_CACHE_CAP`) becomes the
  memory governor; its cap may need to become a byte-budget parameter —
  noted as the one likely follow-up to the `SERIES_CACHE_CAP` constant.

## Acceptance (from the plan)

A >2 GB log opens and pans where `from_bytes` OOMs. Concretely:

1. unit: `remote_source` serves a `Vec<u8>`-backed fixture through the
   callback and every existing reader test passes unchanged against it;
2. wasm: the binding compiles and the demo degrades gracefully (feature
   detected, button hidden without SAB/COOP-COEP);
3. browser: a >2 GB file served by `python3 -m http.server` (range-capable)
   opens, pans and decodes windows.

## Decision requested

Implement after 4.6's design review: the core change is ~40 lines plus
tests, the binding ~120, the demo UX ~80. The risk sits entirely in the
`Atomics.wait` mailbox inside the worker; a spike should prove that path
first before the core lands.
