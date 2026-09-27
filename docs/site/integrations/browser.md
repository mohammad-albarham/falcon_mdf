# Browser and WebAssembly

The [live browser viewer](https://mohammad-albarham.github.io/falcon_mdf/) opens an `.mf4` file, lists channels, and plots selected signals. Parsing and plotting happen in your browser through WebAssembly; the measurement file is not uploaded to a server.

The [WebAssembly package](https://github.com/mohammad-albarham/falcon_mdf/tree/main/wasm) exposes `WasmMf4File` to JavaScript. Its API includes metadata, channel lists, typed arrays, windowed signals, CSV, statistics, and file structure.

```javascript
import init, { WasmMf4File } from "./pkg/falcon_mdf_wasm.js";

await init();
const bytes = new Uint8Array(await file.arrayBuffer());
const measurement = new WasmMf4File(bytes);
const names = JSON.parse(measurement.channel_names());
const first = measurement.signal_arrays(names[0]);
console.log(first.timestamps, first.values);
```

Here, `file` is a browser [`File`](https://developer.mozilla.org/en-US/docs/Web/API/File) selected by the user. `signal_arrays` avoids serializing sample arrays to JSON. `signal_window` returns a decimated time window suitable for plotting, and the viewer runs decoding in a Web Worker so the page stays responsive.

See the [WASM README](https://github.com/mohammad-albarham/falcon_mdf/blob/main/wasm/README.md) for build instructions and the full JavaScript examples.
