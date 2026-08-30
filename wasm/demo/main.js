// Drives the viewer. This file never imports the wasm module: the worker
// (worker.js) owns the WasmMf4File, so parsing and decoding happen off the
// main thread and this file only draws and translates user gestures into
// worker messages.
//
// Interaction model: click channels in the sidebar to overlay them (up to
// MAX_CHANNELS), wheel to zoom around the pointer, drag to pan, double-click
// to reset to the full time range. Zoom/pan re-request each visible window
// decimated in Rust to the point budget, debounced; between requests the
// stale (already-decimated) data is redrawn shifted, so the plot always
// responds immediately.

const MAX_CHANNELS = 8;
const REDECODE_DEBOUNCE_MS = 100;
const COLORS = [
  "#4f8cff", "#ffb454", "#62d96b", "#ff6b81",
  "#c792ea", "#4dd0e1", "#e5c07b", "#7ee787",
];

const $ = (id) => document.getElementById(id);

const landing = $("landing");
const viewer = $("viewer");
const dropzone = $("dropzone");
const fileInput = $("file-input");
const sampleBtn = $("sample-btn");
const statusEl = $("status");
const errorEl = $("error");
const fileInfo = $("fileinfo");
const closeBtn = $("close-btn");
const filterInput = $("filter");
const channelList = $("channels");
const legendEl = $("legend");
const csvBtn = $("csv-btn");
const sharedYEl = $("shared-y");
const plot = $("plot");
const plotwrap = $("plotwrap");
const readoutEl = $("readout");
const plotmsgEl = $("plotmsg");

const worker = new Worker("worker.js", { type: "module" });

// ---------------------------------------------------------------------------
// State

let channels = []; // [{name, unit, group, description}] from meta
let shown = []; // [{name, unit, color, tMin, tMax, ts, vs, min, max}]
let selected = null; // the channel "Download CSV" targets (last clicked)
let view = null; // {t0, t1} — null until the first series arrives
let epoch = 0; // tags series requests; responses with an old id are stale
let pending = 0; // series requests in flight, for the status line
let cursor = null; // {t} while the pointer is over the plot
let sampleInFlight = false;
let sampleQueuedT = null;
let fileOpen = false;
let fileName = "";
let fileBytes = 0;

// ---------------------------------------------------------------------------
// Small helpers

function showError(msg) {
  errorEl.hidden = false;
  errorEl.textContent = msg;
}

function plotMsg(msg) {
  plotmsgEl.hidden = !msg;
  plotmsgEl.textContent = msg ?? "";
}

function setStatus(msg) {
  statusEl.textContent = msg;
}

function humanBytes(n) {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB"];
  let i = -1;
  do {
    n /= 1024;
    i++;
  } while (n >= 1024 && i < units.length - 1);
  return `${n.toFixed(n >= 100 ? 0 : 1)} ${units[i]}`;
}

function fmtNumber(v) {
  if (!Number.isFinite(v)) return "—";
  if (v === 0) return "0";
  const a = Math.abs(v);
  if (a >= 1e6 || a < 1e-3) return v.toExponential(3);
  if (Number.isInteger(v)) return v.toLocaleString();
  return String(parseFloat(v.toPrecision(4)));
}

const shownNames = () => new Set(shown.map((s) => s.name));

function globalExtent() {
  let lo = Infinity;
  let hi = -Infinity;
  for (const s of shown) {
    if (s.tMin !== null) lo = Math.min(lo, s.tMin);
    if (s.tMax !== null) hi = Math.max(hi, s.tMax);
  }
  return lo <= hi ? [lo, hi] : null;
}

// ---------------------------------------------------------------------------
// Worker protocol

worker.onerror = (e) => showError(`worker failed: ${e.message ?? e}`);

worker.onmessage = (ev) => {
  const msg = ev.data;
  switch (msg.type) {
    case "open":
      fileOpen = true;
      worker.postMessage({ type: "meta" });
      break;
    case "meta":
      onMeta(msg);
      break;
    case "series":
      onSeries(msg);
      break;
    case "sample":
      onSample(msg);
      break;
    case "csv":
      onCsv(msg);
      break;
    case "error":
      if (!fileOpen) {
        setStatus("");
        showError(msg.message);
      } else {
        plotMsg(msg.message);
        setStatus("");
        pending = 0;
      }
      break;
  }
};

function onMeta(msg) {
  let info;
  try {
    info = JSON.parse(msg.info);
    channels = JSON.parse(msg.channels);
  } catch (e) {
    showError(`Could not read the file's metadata: ${e.message ?? e}`);
    return;
  }

  fileInfo.replaceChildren();
  const nameEl = document.createElement("b");
  nameEl.textContent = fileName;
  fileInfo.append(nameEl, ` ${humanBytes(fileBytes)}`);
  for (const part of [
    `MDF ${info.version}`,
    `start ${info.start_time}`,
    `${info.channel_group_count} groups`,
    `${info.channel_count} channels`,
  ]) {
    const sep = document.createElement("span");
    sep.className = "sep";
    sep.textContent = "·";
    fileInfo.append(sep, part);
  }

  landing.hidden = true;
  viewer.hidden = false;
  setStatus("");
  plotMsg("");
  filterInput.value = "";
  renderChannelList();
  renderLegend();
  requestAnimationFrame(draw);
}

function onSeries(msg) {
  // Extents are eternal; the decimated points are only for this epoch.
  const entry = shown.find((s) => s.name === msg.name);
  if (!entry) return;
  entry.unit = msg.unit || entry.unit;
  entry.tMin = msg.tMin;
  entry.tMax = msg.tMax;
  pending = Math.max(0, pending - 1);
  setStatus(pending > 0 ? `Decoding ${pending} channel${pending > 1 ? "s" : ""}…` : "");

  // Bootstrap: the first series was requested as (-Inf, Inf), i.e. decimated
  // over exactly the extent we now adopt as the view.
  const bootstrapping = !view;
  if (bootstrapping && globalExtent()) {
    const [g0, g1] = globalExtent();
    view = g0 < g1 ? { t0: g0, t1: g1 } : { t0: g0 - 0.5, t1: g1 + 0.5 };
    // Other channels added while the bootstrap was in flight (or dropped
    // from it as stale) still need a window for the now-known view.
    requestAll();
  }

  if (msg.id !== epoch) return; // a newer view was requested meanwhile

  entry.ts = msg.timestamps;
  entry.vs = msg.values;
  let lo = Infinity;
  let hi = -Infinity;
  for (const v of entry.vs) {
    if (Number.isFinite(v)) {
      if (v < lo) lo = v;
      if (v > hi) hi = v;
    }
  }
  entry.min = lo === Infinity ? null : lo;
  entry.max = hi === -Infinity ? null : hi;

  renderLegend();
  requestAnimationFrame(draw);
}

function onSample(msg) {
  sampleInFlight = false;
  for (const entry of shown) {
    entry.at = msg.values[entry.name] ?? null;
  }
  renderReadout();
  if (sampleQueuedT !== null) {
    const t = sampleQueuedT;
    sampleQueuedT = null;
    sendSampleQuery(t);
  }
}

function onCsv(msg) {
  const rows = msg.csv.trimEnd().split("\n").length - 1; // minus header
  const blob = new Blob([msg.csv], { type: "text/csv" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `${msg.name.replace(/[^\w.-]+/g, "_")}_window.csv`;
  a.click();
  URL.revokeObjectURL(url);
  setStatus(`CSV saved: ${rows.toLocaleString()} rows of ${msg.name} in [${fmtNumber(view?.t0)}, ${fmtNumber(view?.t1)}] s`);
}

// ---------------------------------------------------------------------------
// Requests to the worker

let requestTimer = null;

function scheduleRequestAll() {
  clearTimeout(requestTimer);
  requestTimer = setTimeout(requestAll, REDECODE_DEBOUNCE_MS);
}

function maxPoints() {
  const m = margins();
  const w = Math.max(64, plotwrap.clientWidth - m.l - m.r);
  // Four points per pixel column (first/min/max/last): one column per pixel.
  return 4 * Math.round(w);
}

function requestAll() {
  if (!view || shown.length === 0) return;
  epoch += 1;
  pending = shown.length;
  for (const s of shown) {
    worker.postMessage({
      type: "series",
      id: epoch,
      name: s.name,
      t0: view.t0,
      t1: view.t1,
      maxPoints: maxPoints(),
    });
  }
  setStatus(`Decoding ${pending} channel${pending > 1 ? "s" : ""}…`);
}

function requestBootstrap(name) {
  // First channel: ask for the infinite window; Rust clamps it to the
  // channel's extent and the response carries that extent (tMin/tMax).
  epoch += 1;
  pending = 1;
  setStatus(`Decoding ${name}…`);
  worker.postMessage({
    type: "series",
    id: epoch,
    name,
    t0: -Infinity,
    t1: Infinity,
    maxPoints: maxPoints(),
  });
}

function sendSampleQuery(t) {
  if (shown.length === 0) return;
  if (sampleInFlight) {
    sampleQueuedT = t; // latest-wins: only the newest cursor position matters
    return;
  }
  sampleInFlight = true;
  worker.postMessage({ type: "sample", names: shown.map((s) => s.name), t });
}

// ---------------------------------------------------------------------------
// Channel overlay management

function toggleChannel(name) {
  const idx = shown.findIndex((s) => s.name === name);
  plotMsg("");
  if (idx >= 0) {
    const [removed] = shown.splice(idx, 1);
    worker.postMessage({ type: "drop", names: [removed.name] });
    if (selected === name) selected = shown.at(-1)?.name ?? null;
    if (shown.length === 0) {
      view = null;
      cursor = null;
      readoutEl.hidden = true;
    }
    renderChannelList();
    renderLegend();
    renderReadout();
    requestAnimationFrame(draw);
    return;
  }

  if (shown.length >= MAX_CHANNELS) {
    plotMsg(
      `Up to ${MAX_CHANNELS} channels can be overlaid — remove one from the legend first.`
    );
    return;
  }

  const meta = channels.find((c) => c.name === name);
  shown.push({
    name,
    unit: meta?.unit ?? "",
    color: COLORS[shown.length % COLORS.length],
    tMin: null,
    tMax: null,
    ts: null,
    vs: null,
    min: null,
    max: null,
    at: null,
  });
  selected = name;
  renderChannelList();
  renderLegend();
  if (view) requestAll();
  else requestBootstrap(name);
}

function renderChannelList() {
  const q = filterInput.value.trim().toLowerCase();
  const on = shownNames();
  const shownList = q ? channels.filter((c) => c.name.toLowerCase().includes(q)) : channels;
  channelList.replaceChildren();
  if (shownList.length === 0) {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = "No matching channels";
    channelList.append(li);
    return;
  }
  for (const c of shownList) {
    const li = document.createElement("li");
    li.textContent = c.name;
    const tip = [c.name, c.group && `group: ${c.group}`, c.unit && `unit: ${c.unit}`, c.description]
      .filter(Boolean)
      .join("\n");
    li.title = tip;
    if (on.has(c.name)) li.classList.add("selected");
    li.addEventListener("click", () => toggleChannel(c.name));
    channelList.append(li);
  }
}

function renderLegend() {
  legendEl.replaceChildren();
  if (shown.length === 0) {
    const span = document.createElement("span");
    span.className = "hint";
    span.textContent = "Click channels to overlay them (up to 8).";
    legendEl.append(span);
    csvBtn.disabled = true;
    csvBtn.textContent = "Download CSV";
    csvBtn.title = "";
    return;
  }
  csvBtn.disabled = false;
  csvBtn.textContent = selected ? `Download CSV · ${selected}` : "Download CSV";
  csvBtn.title = selected ? `visible window of ${selected}` : "";
  for (const s of shown) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "leg" + (s.name === selected ? " sel" : "");
    row.title = s.name;
    const dot = document.createElement("span");
    dot.className = "dot";
    dot.style.background = s.color;
    row.append(dot);
    const label = document.createElement("span");
    label.className = "leg-name";
    label.textContent = s.name;
    row.append(label);
    if (s.unit) {
      const unit = document.createElement("span");
      unit.className = "leg-unit";
      unit.textContent = `[${s.unit}]`;
      row.append(unit);
    }
    if (s.min !== null) {
      const range = document.createElement("span");
      range.className = "leg-range";
      range.textContent = `${fmtNumber(s.min)} … ${fmtNumber(s.max)}`;
      row.append(range);
    }
    const x = document.createElement("span");
    x.className = "rm";
    x.textContent = "✕";
    x.title = "remove";
    x.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleChannel(s.name);
    });
    row.append(x);
    row.addEventListener("click", () => {
      selected = s.name;
      renderLegend();
    });
    legendEl.append(row);
  }
}

function renderReadout() {
  if (!cursor || shown.length === 0) {
    readoutEl.hidden = true;
    return;
  }
  readoutEl.hidden = false;
  readoutEl.replaceChildren();
  const head = document.createElement("div");
  head.className = "ro-t";
  head.textContent = `t = ${fmtNumber(cursor.t)} s`;
  readoutEl.append(head);
  for (const s of shown) {
    const row = document.createElement("div");
    row.className = "ro-row";
    const dot = document.createElement("span");
    dot.className = "dot";
    dot.style.background = s.color;
    row.append(dot);
    const name = document.createElement("span");
    name.className = "ro-name";
    name.textContent = s.name;
    row.append(name);
    const val = document.createElement("b");
    val.textContent = s.at === null ? "—" : fmtNumber(s.at);
    row.append(val);
    if (s.unit) {
      const unit = document.createElement("span");
      unit.className = "ro-unit";
      unit.textContent = s.unit;
      row.append(unit);
    }
    readoutEl.append(row);
  }
}

// ---------------------------------------------------------------------------
// Plot

function margins() {
  return { l: 58, r: 14, t: 12, b: 26 };
}

function clampView() {
  const ext = globalExtent();
  if (!ext || !view) return;
  const [g0, g1] = ext;
  const span = g1 - g0;
  if (!(span > 0)) return;
  // Never zoom deeper than ~1e-7 of the recording nor out beyond 105% of it.
  const minSpan = span * 1e-7;
  let s = view.t1 - view.t0;
  if (s < minSpan) {
    const c = (view.t0 + view.t1) / 2;
    view = { t0: c - minSpan / 2, t1: c + minSpan / 2 };
    s = minSpan;
  }
  if (s > span * 1.05) {
    view = { t0: g0, t1: g1 };
    return;
  }
  // Keep the window overlapping the recording.
  const slack = s * 0.05;
  if (view.t0 < g0 - slack) view = { t0: g0 - slack, t1: g0 - slack + s };
  else if (view.t1 > g1 + slack) view = { t0: g1 + slack - s, t1: g1 + slack };
}

function niceStep(raw) {
  const mag = Math.pow(10, Math.floor(Math.log10(raw)));
  const norm = raw / mag;
  return (norm < 1.5 ? 1 : norm < 3 ? 2 : norm < 7 ? 5 : 10) * mag;
}

let canvasSize = [0, 0];

function draw() {
  const w = plotwrap.clientWidth;
  const h = plotwrap.clientHeight;
  if (w < 10 || h < 10) return;
  const dpr = window.devicePixelRatio || 1;
  const bw = Math.round(w * dpr);
  const bh = Math.round(h * dpr);
  if (canvasSize[0] !== bw || canvasSize[1] !== bh) {
    canvasSize = [bw, bh];
    plot.width = bw;
    plot.height = bh;
  }
  const ctx = plot.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  ctx.font = "11px system-ui, sans-serif";

  if (shown.length === 0 || !view) {
    ctx.fillStyle = "#8b93a7";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(
      shown.length === 0 ? "No channels overlaid" : "Decoding…",
      w / 2,
      h / 2
    );
    return;
  }

  const m = margins();
  const pw = w - m.l - m.r;
  const ph = h - m.t - m.b;
  const t0 = view.t0;
  const t1 = view.t1;
  const tSpan = t1 - t0 || 1;
  const X = (t) => m.l + ((t - t0) / tSpan) * pw;

  // Shared Y mode scales every channel against the union of the visible
  // data; the default scales each channel to the plot (the sidebar shows
  // each one's own range), which is what keeps a temperature trace readable
  // next to an RPM trace.
  const shared = sharedYEl.checked;
  let gy0 = Infinity;
  let gy1 = -Infinity;
  if (shared) {
    for (const s of shown) {
      if (s.min === null) continue;
      gy0 = Math.min(gy0, s.min);
      gy1 = Math.max(gy1, s.max);
    }
    if (!(gy0 < gy1)) {
      gy0 = 0;
      gy1 = 1;
    } else {
      const pad = (gy1 - gy0) * 0.05;
      gy0 -= pad;
      gy1 += pad;
    }
  }
  const Y = (entry, v) => {
    if (shared) return m.t + (1 - (v - gy0) / (gy1 - gy0)) * ph;
    let lo = entry.min;
    let hi = entry.max;
    if (!(lo < hi)) {
      const mid = lo === hi ? lo : 0;
      lo = mid - 0.5;
      hi = mid + 0.5;
    } else {
      const pad = (hi - lo) * 0.05;
      lo -= pad;
      hi += pad;
    }
    return m.t + (1 - (v - lo) / (hi - lo)) * ph;
  };

  // Grid: shared X (time) ticks, and Y ticks only in shared mode (in
  // per-channel mode each line has its own scale, so Y labels would lie).
  ctx.strokeStyle = "#242a38";
  ctx.fillStyle = "#8b93a7";
  ctx.lineWidth = 1;
  const step = niceStep(tSpan / 6);
  for (let t = Math.ceil(t0 / step) * step; t <= t1; t += step) {
    const x = X(t);
    ctx.beginPath();
    ctx.moveTo(x, m.t);
    ctx.lineTo(x, h - m.b);
    ctx.stroke();
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    ctx.fillText(fmtNumber(t), Math.min(Math.max(x, m.l + 18), w - m.r - 18), h - m.b + 6);
  }
  ctx.textAlign = "left";
  ctx.fillText("t [s]", w - m.r - 34, h - m.b + 6);
  if (shared) {
    const ystep = niceStep((gy1 - gy0) / 5);
    for (let v = Math.ceil(gy0 / ystep) * ystep; v <= gy1; v += ystep) {
      const y = Y(null, v);
      ctx.beginPath();
      ctx.moveTo(m.l, y);
      ctx.lineTo(w - m.r, y);
      ctx.stroke();
      ctx.textAlign = "right";
      ctx.textBaseline = "middle";
      ctx.fillText(fmtNumber(v), m.l - 6, y);
    }
  } else {
    for (let g = 0; g <= 5; g++) {
      const y = m.t + (ph * g) / 5;
      ctx.beginPath();
      ctx.moveTo(m.l, y);
      ctx.lineTo(w - m.r, y);
      ctx.stroke();
    }
  }

  // Lines. NaN (or ±inf) values arrive as gaps from the Rust decimation and
  // break the path here, so an invalid stretch reads as a hole, not a bridge.
  ctx.lineJoin = "round";
  ctx.lineWidth = 1.5;
  for (const s of shown) {
    if (!s.ts) continue;
    const { ts, vs } = s;
    ctx.strokeStyle = s.color;
    ctx.beginPath();
    let pen = false;
    for (let i = 0; i < ts.length; i++) {
      const v = vs[i];
      if (!Number.isFinite(v)) {
        pen = false;
        continue;
      }
      const px = X(ts[i]);
      const py = Y(s, v);
      if (pen) ctx.lineTo(px, py);
      else {
        ctx.moveTo(px, py);
        pen = true;
      }
    }
    ctx.stroke();
  }

  // Cursor: one vertical line at the probe time; the per-channel values at
  // the nearest sample live in the readout panel.
  if (cursor && cursor.t >= t0 && cursor.t <= t1) {
    ctx.strokeStyle = "#aab3c5";
    ctx.setLineDash([3, 3]);
    ctx.beginPath();
    ctx.moveTo(X(cursor.t), m.t);
    ctx.lineTo(X(cursor.t), h - m.b);
    ctx.stroke();
    ctx.setLineDash([]);
  }
}

// ---------------------------------------------------------------------------
// Plot interactions

function tAtX(x) {
  const m = margins();
  const pw = plotwrap.clientWidth - m.l - m.r;
  const frac = Math.min(1, Math.max(0, (x - m.l) / pw));
  return view.t0 + frac * (view.t1 - view.t0);
}

let dragging = null;

plot.addEventListener("wheel", (e) => {
  if (!view) return;
  e.preventDefault();
  const rect = plot.getBoundingClientRect();
  const x = e.clientX - rect.left;
  const focus = tAtX(x);
  const factor = Math.exp(Math.max(-120, Math.min(120, e.deltaY)) * 0.0015);
  view = {
    t0: focus + (view.t0 - focus) * factor,
    t1: focus + (view.t1 - focus) * factor,
  };
  clampView();
  requestAnimationFrame(draw);
  scheduleRequestAll();
}, { passive: false });

plot.addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || !view) return;
  plot.setPointerCapture(e.pointerId);
  dragging = { x: e.clientX, view: { ...view }, moved: false };
});

plot.addEventListener("pointermove", (e) => {
  const rect = plot.getBoundingClientRect();
  if (dragging) {
    dragging.moved = true;
    const m = margins();
    const pw = plotwrap.clientWidth - m.l - m.r;
    const span = dragging.view.t1 - dragging.view.t0;
    const dt = (-(e.clientX - dragging.x) / pw) * span;
    view = { t0: dragging.view.t0 + dt, t1: dragging.view.t1 + dt };
    clampView();
    requestAnimationFrame(draw);
    scheduleRequestAll();
    return;
  }
  if (!view || shown.length === 0) return;
  cursor = { t: tAtX(e.clientX - rect.left) };
  requestAnimationFrame(draw);
  sendSampleQuery(cursor.t);
});

plot.addEventListener("pointerup", (e) => {
  if (dragging) {
    dragging = null;
    try {
      plot.releasePointerCapture(e.pointerId);
    } catch {
      // capture already released (pointercancel handled it)
    }
  }
});

plot.addEventListener("pointercancel", () => {
  dragging = null;
});

plot.addEventListener("pointerleave", () => {
  if (dragging) return;
  cursor = null;
  readoutEl.hidden = true;
  requestAnimationFrame(draw);
});

plot.addEventListener("dblclick", () => {
  const ext = globalExtent();
  if (!ext) return;
  view = ext[0] < ext[1] ? { t0: ext[0], t1: ext[1] } : { t0: ext[0] - 0.5, t1: ext[1] + 0.5 };
  clampView();
  requestAnimationFrame(draw);
  requestAll();
});

csvBtn.addEventListener("click", () => {
  if (!selected || !view) return;
  plotMsg("");
  worker.postMessage({ type: "csv", name: selected, t0: view.t0, t1: view.t1 });
});

sharedYEl.addEventListener("change", () => requestAnimationFrame(draw));

new ResizeObserver(() => {
  requestAnimationFrame(draw);
  scheduleRequestAll(); // the point budget follows the canvas width
}).observe(plotwrap);

// ---------------------------------------------------------------------------
// File plumbing (drag & drop, picker, bundled sample) — unchanged in spirit

async function loadLocalFile(f) {
  setStatus(`Reading ${f.name}…`);
  try {
    const bytes = await f.arrayBuffer();
    openFile(bytes, f.name);
  } catch (e) {
    setStatus("");
    showError(`Could not read ${f.name}: ${e.message ?? e}`);
  }
}

function openFile(buffer, name) {
  errorEl.hidden = true;
  plotMsg("");
  shown = [];
  selected = null;
  view = null;
  epoch += 1;
  pending = 0;
  cursor = null;
  readoutEl.hidden = true;
  viewer.hidden = true;
  landing.hidden = false;
  setStatus(`Parsing ${name} (${humanBytes(buffer.byteLength)})…`);
  fileName = name;
  fileBytes = buffer.byteLength;
  fileOpen = false;
  // Hand the bytes over; the worker parses them off the main thread.
  worker.postMessage({ type: "open", bytes: buffer }, [buffer]);
}

function reset() {
  shown = [];
  selected = null;
  view = null;
  channels = [];
  cursor = null;
  readoutEl.hidden = true;
  viewer.hidden = true;
  landing.hidden = false;
  setStatus("Ready — drop a file, or load the bundled sample.");
  errorEl.hidden = true;
}

function wire() {
  dropzone.addEventListener("click", (e) => {
    if (e.target === sampleBtn) return;
    fileInput.click();
  });
  dropzone.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      fileInput.click();
    }
  });
  fileInput.addEventListener("change", () => {
    const f = fileInput.files[0];
    if (f) loadLocalFile(f);
    fileInput.value = "";
  });
  for (const ev of ["dragover", "dragenter"]) {
    dropzone.addEventListener(ev, (e) => {
      e.preventDefault();
      dropzone.classList.add("dragover");
    });
  }
  for (const ev of ["dragleave", "drop"]) {
    dropzone.addEventListener(ev, (e) => {
      e.preventDefault();
      dropzone.classList.remove("dragover");
    });
  }
  dropzone.addEventListener("drop", (e) => {
    const f = e.dataTransfer?.files?.[0];
    if (f) loadLocalFile(f);
  });
  sampleBtn.addEventListener("click", async (e) => {
    e.stopPropagation();
    try {
      setStatus("Fetching the bundled sample…");
      const res = await fetch("sample.mf4");
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      openFile(await res.arrayBuffer(), "sample.mf4 (synthetic)");
    } catch (err) {
      setStatus("");
      showError(`Could not load the sample: ${err.message ?? err}`);
    }
  });
  closeBtn.addEventListener("click", reset);
  filterInput.addEventListener("input", renderChannelList);
}

wire();
setStatus("Ready — drop a file, or load the bundled sample.");

// Deep link: ?file=<same-origin path> loads that recording on boot, the same
// code path as the sample button (fetch → worker). Lets a recording be shared
// by URL and lets automated checks drive real files without a file picker.
(async () => {
  const target = new URLSearchParams(location.search).get("file");
  if (!target || target.includes("://")) return;
  try {
    setStatus(`Fetching ${target}…`);
    const res = await fetch(target);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    openFile(
      await res.arrayBuffer(),
      target.split("/").pop() || target
    );
  } catch (err) {
    setStatus("");
    showError(`Could not load ${target}: ${err.message ?? err}`);
  }
})();
