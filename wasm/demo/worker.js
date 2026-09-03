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
//   {type:"stats", name, t0, t1}                     window statistics (cursors / visible view)
//   {type:"csv", name, t0, t1}
//   {type:"drop", names}                             free raw caches for removed channels
//
//   worker -> main
//   {type:"open", channelCount}
//   {type:"meta", info, channels}                    JSON strings, main parses
//   {type:"series", id, name, unit, tMin, tMax,
//    kind, timestamps, values?, labels?, vocab?,
//    truncated?}                                     Float64Arrays, buffers transferred;
//                                                   a text channel carries run-collapsed
//                                                   labels + the channel's label vocabulary
//                                                   (first-appearance order, stable band rows)
//   {type:"sample", t, values}                       values: {name: number|string|null}
//   {type:"stats", name, t0, t1, stats}              stats: JSON string; t0/t1 echo the
//                                                   request so main can drop stale replies
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

// A text channel's full run-collapsed label series, kept for cursor readouts
// (nearest-sample probe, no wasm call) and the stable label vocabulary. The
// budget is a formality: run collapsing means the real size is the number of
// state changes, and the Rust cache makes the second decode free.
function rawText(name) {
  if (!raw.has(name)) {
    const text = JSON.parse(
      file.signal_text(name, -Infinity, Infinity, 1 << 28)
    );
    const vocab = [];
    const seen = new Set();
    for (const l of text.labels) {
      if (l !== null && !seen.has(l)) {
        seen.add(l);
        vocab.push(l);
      }
    }
    raw.set(name, {
      timestamps: copyF64(text.timestamps),
      labels: text.labels,
      vocab,
      isText: true,
    });
  }
  return raw.get(name);
}

// How a channel decodes ("f64" | "text" | "bytes" | "array" | …): decides
// which raw cache and which decode endpoint a request uses. Cached per name.
const kinds = new Map();

function kindOf(name) {
  if (!kinds.has(name)) {
    if (!file) throw new Error("no file is open");
    kinds.set(name, file.channel_kind(name)); // throws on an unknown name
  }
  return kinds.get(name);
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
        kinds.clear();
        post({ type: "open", channelCount: file.channel_count() });
        break;
      }
      case "meta": {
        post({ type: "meta", info: file.info(), channels: file.channels() });
        break;
      }
      case "series": {
        // Decode the full series once (extents + cursor cache), then let
        // Rust decimate the window to the point budget. Text channels take
        // the label path: run-collapsed bands instead of numeric points.
        if (kindOf(msg.name) === "text") {
          const r = rawText(msg.name);
          const w = JSON.parse(
            file.signal_text(msg.name, msg.t0, msg.t1, msg.maxPoints)
          );
          post({
            type: "series",
            id: msg.id,
            name: msg.name,
            unit: w.unit,
            kind: "text",
            tMin: r.timestamps[0],
            tMax: r.timestamps[r.timestamps.length - 1],
            timestamps: r.timestamps.slice(), // fresh buffer: this reply owns it
            labels: w.labels,
            vocab: r.vocab,
            truncated: w.truncated,
          });
          break;
        }
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
            kind: "f64",
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
          if (kindOf(name) === "text") {
            const r = rawText(name);
            const i = nearestIndex(r.timestamps, msg.t);
            values[name] = i < 0 ? null : r.labels[i]; // a label or null (invalid)
            continue;
          }
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
      case "stats": {
        // The window is echoed verbatim so the main thread can recognise a
        // reply for a view or region it has since replaced.
        post({
          type: "stats",
          name: msg.name,
          t0: msg.t0,
          t1: msg.t1,
          stats: file.signal_stats(msg.name, msg.t0, msg.t1),
        });
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
