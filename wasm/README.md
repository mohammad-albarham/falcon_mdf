# falcon-mdf-wasm

WebAssembly bindings for the `falcon_mdf` ASAM MDF v4 (MF4) measurement data file reader via `wasm-bindgen`.

A live demo of these bindings — a multi-channel MF4 viewer that parses and
decodes in a Web Worker and decimates zoom levels in Rust — runs at
[mohammad-albarham.github.io/falcon_mdf](https://mohammad-albarham.github.io/falcon_mdf/);
its page lives in [`demo/`](demo/) and is deployed by `.github/workflows/pages.yml`.

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
```

Errors from every endpoint cross into JavaScript as thrown `Error`s (for
example `ChannelNotFound` for an unknown name, or the parser's own message for
a file that is not MF4); nothing in the binding panics, because a wasm panic
would kill the module for every caller.
