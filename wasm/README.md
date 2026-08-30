# falcon-mdf-wasm

WebAssembly bindings for the `falcon_mdf` ASAM MDF v4 (MF4) measurement data file reader via `wasm-bindgen`.

A live demo of these bindings — open an `.mf4` file and browse its channels
in the browser — runs at
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

  // Extract signal samples and timestamps for a channel
  const signalJson = file.signal(channelNames[0]);
  const signal = JSON.parse(signalJson);
  console.log(signal);
  // { name: "VehicleSpeed", unit: "km/h", timestamps: [0.0, 0.01, ...], values: [0.0, 1.2, ...] }
}
```

Non-finite floating point numbers (`NaN`, `+Infinity`, `-Infinity`) are returned as `null` in accordance with the JSON standard.
