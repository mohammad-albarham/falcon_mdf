// Drives the falcon-mdf-wasm binding from the page: file in, channel list,
// canvas plot, stats. Everything runs client-side; no network after load
// except the optional bundled sample file.
import init, { WasmMf4File } from "./pkg/falcon_mdf_wasm.js";

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
const channelTitle = $("channeltitle");
const plot = $("plot");
const statsEl = $("stats");

let ready = false;
let file = null;
let channelNames = [];
let selectedName = null;
let selected = null;

function showError(msg) {
  errorEl.hidden = false;
  errorEl.textContent = msg;
}

function setStatus(msg) {
  statusEl.textContent = msg;
}

// The wasm calls are synchronous and freeze the main thread; yield first so
// the status message actually paints before the freeze.
const paint = () => new Promise((resolve) => setTimeout(resolve, 30));

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

async function openFile(bytes, name) {
  errorEl.hidden = true;
  if (!ready) return showError("The WebAssembly module is still loading — try again in a moment.");
  landing.hidden = false;
  viewer.hidden = true;
  setStatus(`Parsing ${name} (${humanBytes(bytes.length)})…`);
  await paint();
  let parsed;
  try {
    parsed = new WasmMf4File(bytes);
  } catch (e) {
    setStatus("");
    return showError(`Could not open ${name}: ${e.message ?? e}`);
  }
  file = parsed;
  channelNames = JSON.parse(file.channel_names());
  const info = JSON.parse(file.info());

  fileInfo.replaceChildren();
  const nameEl = document.createElement("b");
  nameEl.textContent = name;
  fileInfo.append(nameEl, ` ${humanBytes(bytes.length)}`);
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
  filterInput.value = "";
  renderChannelList();
  if (channelNames.length > 0) {
    await selectChannel(channelNames[0]);
  } else {
    channelTitle.textContent = "This file has no channels.";
    statsEl.replaceChildren();
  }
}

function renderChannelList() {
  const q = filterInput.value.trim().toLowerCase();
  const shown = q ? channelNames.filter((n) => n.toLowerCase().includes(q)) : channelNames;
  channelList.replaceChildren();
  if (shown.length === 0) {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = "No matching channels";
    channelList.append(li);
    return;
  }
  for (const name of shown) {
    const li = document.createElement("li");
    li.textContent = name;
    li.title = name;
    if (name === selectedName) li.classList.add("selected");
    li.addEventListener("click", () => selectChannel(name));
    channelList.append(li);
  }
}

async function selectChannel(name) {
  if (!file) return;
  selectedName = name;
  selected = null;
  for (const li of channelList.children) {
    li.classList.toggle("selected", li.textContent === name);
  }
  channelTitle.replaceChildren(name);
  statsEl.replaceChildren();
  setStatus(`Decoding ${name}…`);
  await paint();
  let series;
  try {
    series = JSON.parse(file.signal(name));
  } catch (e) {
    setStatus("");
    return showError(`Could not decode ${name}: ${e.message ?? e}`);
  }
  setStatus("");
  selected = series;
  drawSeries(series);
  renderStats(series);
}

function drawSeries(series) {
  const wrap = plot.parentElement;
  const w = wrap.clientWidth;
  const h = wrap.clientHeight;
  const dpr = window.devicePixelRatio || 1;
  plot.width = Math.max(1, Math.round(w * dpr));
  plot.height = Math.max(1, Math.round(h * dpr));
  const ctx = plot.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);

  channelTitle.replaceChildren(series.name);
  if (series.unit) {
    const unit = document.createElement("span");
    unit.className = "unit";
    unit.textContent = ` [${series.unit}]`;
    channelTitle.append(unit);
  }

  // The binding emits non-finite floats (NaN, ±inf — including samples the
  // file marks invalid) as null, so both axes need null-safe walks.
  const ts = series.timestamps;
  const vs = series.values;
  let n = 0;
  let xMin = Infinity;
  let xMax = -Infinity;
  let yMin = Infinity;
  let yMax = -Infinity;
  for (let i = 0; i < ts.length; i++) {
    const t = ts[i];
    const v = vs[i];
    if (t === null || v === null || !Number.isFinite(t) || !Number.isFinite(v)) continue;
    n++;
    if (t < xMin) xMin = t;
    if (t > xMax) xMax = t;
    if (v < yMin) yMin = v;
    if (v > yMax) yMax = v;
  }
  if (n === 0) {
    ctx.fillStyle = "#8b93a7";
    ctx.font = "13px system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText("No finite samples to plot", w / 2, h / 2);
    return;
  }
  if (xMax === xMin) xMax = xMin + 1;
  if (yMax === yMin) {
    yMin -= 0.5;
    yMax += 0.5;
  }
  const padY = (yMax - yMin) * 0.05;
  yMin -= padY;
  yMax += padY;

  const m = { l: 62, r: 14, t: 10, b: 26 };
  const pw = w - m.l - m.r;
  const ph = h - m.t - m.b;
  const X = (t) => m.l + ((t - xMin) / (xMax - xMin)) * pw;
  const Y = (v) => m.t + (1 - (v - yMin) / (yMax - yMin)) * ph;

  ctx.strokeStyle = "#242a38";
  ctx.fillStyle = "#8b93a7";
  ctx.font = "11px system-ui, sans-serif";
  ctx.lineWidth = 1;
  const gridY = 5;
  for (let g = 0; g <= gridY; g++) {
    const v = yMin + ((yMax - yMin) * g) / gridY;
    const y = Y(v);
    ctx.beginPath();
    ctx.moveTo(m.l, y);
    ctx.lineTo(w - m.r, y);
    ctx.stroke();
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    ctx.fillText(fmtNumber(v), m.l - 6, y);
  }
  const gridX = 6;
  for (let g = 0; g <= gridX; g++) {
    const t = xMin + ((xMax - xMin) * g) / gridX;
    const x = X(t);
    ctx.beginPath();
    ctx.moveTo(x, m.t);
    ctx.lineTo(x, h - m.b);
    ctx.stroke();
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    ctx.fillText(fmtNumber(t), Math.min(Math.max(x, m.l + 18), w - m.r - 18), h - m.b + 6);
  }

  // Stride instead of tessellating every sample; a demo plot gains nothing
  // from the points in between, and channels can hold millions.
  const maxPoints = 24000;
  const stride = Math.max(1, Math.ceil(ts.length / maxPoints));
  ctx.strokeStyle = "#4f8cff";
  ctx.lineWidth = 1.5;
  ctx.lineJoin = "round";
  ctx.beginPath();
  let drawing = false;
  for (let i = 0; i < ts.length; i += stride) {
    const t = ts[i];
    const v = vs[i];
    if (t === null || v === null || !Number.isFinite(t) || !Number.isFinite(v)) {
      drawing = false;
      continue;
    }
    if (drawing) ctx.lineTo(X(t), Y(v));
    else {
      ctx.moveTo(X(t), Y(v));
      drawing = true;
    }
  }
  ctx.stroke();
}

function renderStats(series) {
  const vs = series.values;
  let finite = 0;
  let gaps = 0;
  let min = Infinity;
  let max = -Infinity;
  let sum = 0;
  for (const v of vs) {
    if (v === null || !Number.isFinite(v)) {
      gaps++;
      continue;
    }
    finite++;
    if (v < min) min = v;
    if (v > max) max = v;
    sum += v;
  }
  const parts = [
    ["samples", series.timestamps.length.toLocaleString()],
    ["min", fmtNumber(min)],
    ["max", fmtNumber(max)],
    ["mean", finite ? fmtNumber(sum / finite) : "—"],
  ];
  statsEl.replaceChildren();
  for (const [label, value] of parts) {
    const span = document.createElement("span");
    span.append(`${label} `);
    const b = document.createElement("b");
    b.textContent = value;
    span.append(b);
    statsEl.append(span);
  }
  if (gaps > 0) {
    const note = document.createElement("span");
    note.className = "note";
    note.textContent = `${gaps.toLocaleString()} invalid / non-finite samples drawn as gaps`;
    statsEl.append(note);
  }
}

async function loadLocalFile(f) {
  setStatus(`Reading ${f.name}…`);
  await paint();
  try {
    const bytes = new Uint8Array(await f.arrayBuffer());
    await openFile(bytes, f.name);
  } catch (e) {
    setStatus("");
    showError(`Could not read ${f.name}: ${e.message ?? e}`);
  }
}

function reset() {
  file = null;
  channelNames = [];
  selectedName = null;
  selected = null;
  viewer.hidden = true;
  landing.hidden = false;
  setStatus("");
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
      const bytes = new Uint8Array(await res.arrayBuffer());
      await openFile(bytes, "sample.mf4 (synthetic)");
    } catch (err) {
      setStatus("");
      showError(`Could not load the sample: ${err.message ?? err}`);
    }
  });
  closeBtn.addEventListener("click", reset);
  filterInput.addEventListener("input", renderChannelList);
  window.addEventListener("resize", () => {
    if (selected) drawSeries(selected);
  });
}

async function boot() {
  wire();
  try {
    await init();
  } catch (e) {
    return showError(`Failed to load the WebAssembly module: ${e.message ?? e}`);
  }
  ready = true;
  setStatus("Ready — drop a file, or load the bundled sample.");
}

boot();
