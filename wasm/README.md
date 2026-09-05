# falcon-mdf-wasm

WebAssembly bindings for the `falcon_mdf` ASAM MDF v4 (MF4) measurement data file reader via `wasm-bindgen`.

A live demo of these bindings — a multi-channel MF4 viewer that parses and
decodes in a Web Worker and decimates zoom levels in Rust — runs at
[mohammad-albarham.github.io/falcon_mdf](https://mohammad-albarham.github.io/falcon_mdf/);
its page lives in [`demo/`](demo/) and is deployed by `.github/workflows/pages.yml`.

## Worker regression tests

From the repository root, run `node --test wasm/tests/worker.test.mjs`.
These tests drive the real message handler with a mock WASM boundary, covering
file-specific caches, replacement/freeing, text-window alignment and XY
resampling. Run the Rust binding tests and a rebuilt browser demo as well;
the mock cannot validate Rust decoding or wasm-bindgen interoperability.

The viewer keeps raw and channel-kind caches per file object. Replacing a
comparison file releases that file; successfully opening a new primary file
releases all previous files. Failed opens preserve the previous instances.
Channels with equal names across files remain separate; duplicate names
within one file still need a future location-based API.

## Building

Build the WebAssembly module using `wasm-pack` or `cargo` (run from this
directory):

```bash
# Build with wasm-pack for web targets
wasm-pack build --target web

# Or build the wasm32-unknown-unknown target directly with cargo
cargo build --target wasm32-unknown-unknown --release
```

## JavaScript API

```javascript
import init, { WasmMf4File } from "./pkg/falcon_mdf_wasm.js";

async function run() {
  await init();

  const response = await fetch("measurement.mf4");
  const bytes = new Uint8Array(await response.arrayBuffer());

  // Open the MF4 file from in-memory byte buffer
  const file = new WasmMf4File(bytes);

  // File metadata as a JSON object string
  console.log(JSON.parse(file.info()));
  // { version: "4.10", start_time: "2023-01-01T00:00:00.000Z", channel_group_count: 1, channel_count: 10 }

  // Channel count and channel names
  console.log(`Total channels: ${file.channel_count()}`);
  const channelNames = JSON.parse(file.channel_names());
  console.log("Channels:", channelNames);

  // Every channel's metadata in one call (same names/order as channel_names)
  console.log(JSON.parse(file.channels()));
  // [ { name: "VehicleSpeed", unit: "km/h", group: "Engine", description: "" }, ... ]

  // Extract signal samples and timestamps for a channel (JSON)
  const signalJson = file.signal(channelNames[0]);
  const signal = JSON.parse(signalJson);
  console.log(signal);
  // { name: "VehicleSpeed", unit: "km/h", timestamps: [0.0, 0.01, ...], values: [0.0, 1.2, ...] }
}
```

Non-finite floating point numbers (`NaN`, `+Infinity`, `-Infinity`) are returned as `null` in accordance with the JSON standard.

### Typed arrays, windowed decimation, and CSV

For plotting, the typed-array endpoints move the data without a JSON detour —
and `signal_window` decimates on the Rust side, so a zoomed view ships at most
`max_points` points (first/min/max/last per pixel column; a single-sample
spike always survives) instead of the whole channel. The demo in `demo/` runs
the whole API inside a [Web Worker](demo/worker.js) so the main thread only
ever draws:

```javascript
// Full channel as Float64Arrays — NaN stays NaN (draw it as a gap)
const arrays = file.signal_arrays("VehicleSpeed");
// { timestamps: Float64Array, values: Float64Array, name, unit }

// A zoom window, decimated to a point budget. Non-finite bounds are clamped
// to the channel's extent, so (-Infinity, Infinity) is the full view.
const window = file.signal_window("VehicleSpeed", 10.0, 20.0, 2000);
// same shape, at most ~2000 points covering [10, 20] s

// The same window as CSV, formatted in Rust ("timestamp,<name>" header,
// non-finite values as empty fields)
const csv = file.signal_csv("VehicleSpeed", 10.0, 20.0);

// Statistics over the same window (same inclusive bounds and NaN handling
// as signal_window), as JSON — one call per cursor region or visible view
const stats = JSON.parse(file.signal_stats("VehicleSpeed", 10.0, 20.0));
// { count: 900, invalid: 3, min: 0, max: 210.5, mean: 88.2,
//   first: 12.5, last: 190.25, t0: 10, t1: 20 }
```

Errors from every endpoint cross into JavaScript as thrown `Error`s (for
example `ChannelNotFound` for an unknown name, or the parser's own message for
a file that is not MF4); nothing in the binding panics, because a wasm panic
would kill the module for every caller.

### File structure

`structure()` returns the file's internal outline — the tree a viewer's
structure panel draws, from the identification block down to a single
channel — as one JSON document. Metadata only: nothing here decodes
samples. MDF 3 files come in the same shape minus the sections the format
does not carry (block walk, history, attachments, events, hierarchy).

```javascript
const s = JSON.parse(file.structure());
// {
//   format: 4, version: "4.10", block_count: 66,
//   id_block: "MDF ", hd_block: "##HD",
//   history: [{ time: "2011-08-24T13:53:19.000Z", tool: "Vector Informatik GmbH CANape 10.0.0.30836" }],
//   attachments: [{ name: "ReadMe.txt", embedded: true, size: 1234 }],
//   events: [{ name: "", type: "Trigger", position: 0.0 }],
//   hierarchy: [{ name: "Engine", channels: ["Speed"], unresolved: 0, children: [] }],
//   data_groups: [{
//     index: 0, sorted: true, comment: "",
//     channel_groups: [{
//       index: 0, name: "100ms", samples: 102, bus: false, vlsd: false, comment: "",
//       reductions: [{ cycles: 10, interval: 0.1, sync: "Time" }],
//       channels: [{ index: 0, name: "t", unit: "s", master: true,
//                    array: false, kind: "f64", unreadable: null }],
//     }],
//   }],
// }
```

`kind` is the same string `channels()` carries, so a viewer can mark
unplotable channels from this document alone; `unreadable` names the reason a
channel's samples cannot be read at all. Non-finite `position`/`interval`
values are `null`.
