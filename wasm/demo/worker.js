// The parsing/decoding worker. It — and only it — owns the WasmMf4File, so
// the main thread never runs wasm and never freezes: it draws whatever this
// worker posts back.
//
// Protocol (plain objects over postMessage):
//   main -> worker                                   notes
//   {type:"open", bytes}                             bytes is a transferred ArrayBuffer
//   {type:"meta"}
//   {type:"series", id, name, t0, t1, maxPoints}     decimated window (id: staleness tag)
//   {type:"sample", names, t}                        nearest raw sample per channel
//   {type:"csv", name, t0, t1}
//   {type:"drop", names}                             free raw caches for removed channels
//
//   worker -> main
//   {type:"open", channelCount}
//   {type:"meta", info, channels}                    JSON strings, main parses
//   {type:"series", id, name, unit, tMin, tMax,
//    timestamps, values}                             Float64Arrays, buffers transferred
//   {type:"sample", t, values}                       values: {name: number|null}
//   {type:"csv", name, csv}
//   {type:"error", message}
import init, { WasmMf4File } from "./pkg/falcon_mdf_wasm.js";

let file = null;
let inited = false;

// Raw (undecimated) per-channel arrays, kept for cursor lookups: a nearest-
// timestamp probe is a binary search over these, no wasm call at all. The
// Rust side caches decodes too, so this map costs memory, not re-decodes.
const raw = new Map();

async function ensureInit() {
  if (!inited) {
    await init();
    inited = true;
  }
}

function post(msg, transfer) {
  self.postMessage(msg, transfer ?? []);
}

// The Rust call may hand back a view into wasm linear memory rather than an
// owning array; copy into a fresh typed array so the buffer can be posted to
// the main thread (wasm memory is not transferable) and stays valid whatever
// the module does next.
function copyF64(view) {
  const out = new Float64Array(view.length);
  out.set(view);
  return out;
}

function rawSeries(name) {
  if (!file) throw new Error("no file is open");
  if (!raw.has(name)) {
    raw.set(name, file.signal_arrays(name)); // throws on an unknown name
  }
  return raw.get(name);
}

// Index of the sample nearest to `t` in an ascending Float64Array.
function nearestIndex(times, t) {
  let lo = 0;
  let hi = times.length; // lower bound: first index with times[i] >= t
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (times[mid] < t) lo = mid + 1;
    else hi = mid;
  }
  if (lo <= 0) return times.length ? 0 : -1;
  if (lo >= times.length) return times.length - 1;
  return t - times[lo - 1] <= times[lo] - t ? lo - 1 : lo;
}

self.onmessage = async (ev) => {
  const msg = ev.data;
  try {
    switch (msg.type) {
      case "open": {
        await ensureInit();
        file = new WasmMf4File(new Uint8Array(msg.bytes));
        raw.clear();
        post({ type: "open", channelCount: file.channel_count() });
        break;
      }
      case "meta": {
        post({ type: "meta", info: file.info(), channels: file.channels() });
        break;
      }
      case "series": {
        // Decode the full series once (extents + cursor cache), then let
        // Rust decimate the window to the point budget.
        const r = rawSeries(msg.name);
        const w = file.signal_window(msg.name, msg.t0, msg.t1, msg.maxPoints);
        const ts = copyF64(w.timestamps);
        const vs = copyF64(w.values);
        post(
          {
            type: "series",
            id: msg.id,
            name: msg.name,
            unit: w.unit,
            tMin: r.timestamps[0],
            tMax: r.timestamps[r.timestamps.length - 1],
            timestamps: ts,
            values: vs,
          },
          [ts.buffer, vs.buffer]
        );
        break;
      }
      case "sample": {
        const values = {};
        for (const name of msg.names) {
          const r = rawSeries(name);
          const i = nearestIndex(r.timestamps, msg.t);
          if (i < 0) {
            values[name] = null;
          } else {
            const v = r.values[i];
            values[name] = Number.isFinite(v) ? v : null; // NaN/invalid reads as "no value"
          }
        }
        post({ type: "sample", t: msg.t, values });
        break;
      }
      case "csv": {
        post({ type: "csv", name: msg.name, csv: file.signal_csv(msg.name, msg.t0, msg.t1) });
        break;
      }
      case "drop": {
        for (const name of msg.names) raw.delete(name);
        break;
      }
      default:
        throw new Error(`unknown message type: ${msg.type}`);
    }
  } catch (e) {
    // Everything the worker does funnels through here — a thrown wasm Error
    // must reach the page as a message, never as a silent hang.
    post({ type: "error", message: e?.message ?? String(e) });
  }
};
