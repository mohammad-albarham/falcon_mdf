// Drives the viewer. This file never imports the wasm module: the worker
// (worker.js) owns the WasmMf4File, so parsing and decoding happen off the
// main thread and this file only draws and translates user gestures into
// worker messages.
//
// Interaction model: click channels in the sidebar to plot them (up to
// MAX_CHANNELS), wheel to zoom around the pointer, drag to pan, shift-drag
// to zoom to a drawn region, double-click to reset to the full time range.
// A focused plot also takes keys (arrows pan, +/− zoom, Home resets, 1–8
// toggle plotted slots), and touch adds a two-finger pinch (X-only) with a
// plain one-finger pan. The X axis labels seconds from the recording start
// or, toggled in the toolbar, local wall clock anchored at it — the internal
// math never leaves seconds.
// A click places measurement cursor A, shift-click cursor B; with both
// placed the region between them is shaded and profiled in the stats strip
// (one worker round trip per channel per region), and the next click or Esc
// clears the pair. The toolbar toggles one overlay pane against one lane per
// channel; lanes share the X axis, so zoom/pan mean the same thing in both
// layouts. Zoom/pan re-request each visible window decimated in Rust to the
// point budget, debounced; between requests the stale (already-decimated) data is
// redrawn shifted, so the plot always responds immediately.

const MAX_CHANNELS = 8;
const REDECODE_DEBOUNCE_MS = 100;
// Stacked lanes never get shorter than this. Past it the plot area grows
// (and the page scrolls) instead of lanes scrolling internally — a lane
// clipped out of an internal scroller looks exactly like a channel with no
// data.
const MIN_LANE_H = 64;
// A shift-drag narrower than this is a click or a wiggle, not a region.
const MIN_SELECT_PX = 8;
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
const searchModeEl = $("search-mode");
const channelList = $("channels");
const legendEl = $("legend");
const csvBtn = $("csv-btn");
const sharedYEl = $("shared-y");
const sharedYLabel = $("shared-y-label");
const overlayBtn = $("overlay-btn");
const stackedBtn = $("stacked-btn");
const relativeBtn = $("relative-btn");
const absoluteBtn = $("absolute-btn");
const plot = $("plot");
const plotwrap = $("plotwrap");
const readoutEl = $("readout");
const statsbarEl = $("statsbar");
const plotmsgEl = $("plotmsg");
const tableBtn = $("table-btn");
const tablepanelEl = $("tablepanel");
const tableheadEl = $("tablehead");
const tablescrollEl = $("tablescroll");
const tablebodyEl = $("tablebody");
const tablefootEl = $("tablefoot");
const detailsBtn = $("details-btn");
const detailspanelEl = $("detailspanel");
const detailsheadEl = $("detailshead");
const detailsbodyEl = $("detailsbody");
const rememberLabel = $("remember-label");
const rememberBox = $("remember-file");
const dbcBtn = $("dbc-btn");
const dbcInput = $("dbc-input");
const busBtn = $("bus-btn");
const buspanelEl = $("buspanel");
const busheadEl = $("bushead");
const busscrollEl = $("busscroll");
const busbodyEl = $("busbody");
const gpsBtn = $("gps-btn");
const gpspanelEl = $("gpspanel");
const gpsplotEl = $("gpsplot");
const gpslegendEl = $("gpslegend");
const xyBtn = $("xy-btn");
const xypanelEl = $("xypanel");
const xyxEl = $("xy-x");
const xyyEl = $("xy-y");
const xycountEl = $("xycount");
const xycanvasEl = $("xycanvas");
const computedExpr = $("computed-expr");
const computedBtn = $("computed-btn");
const compareBtn = $("compare-btn");
const compareInput = $("compare-input");
const tabStructureBtn = $("tab-structure");
const tabChannelsBtn = $("tab-channels");
const channellistEl = $("channellist");
const structureEl = $("structure");
const structFilterEl = $("struct-filter");
const structExpandBtn = $("struct-expand");
const structCollapseBtn = $("struct-collapse");
const structtreeEl = $("structtree");

const worker = new Worker("worker.js", { type: "module" });

// ---------------------------------------------------------------------------
// State

let channels = []; // [{name, unit, group, description}] from meta
let shown = []; // [{name, unit, color, tMin, tMax, ts, vs, min, max}]
let selected = null; // the channel "Download CSV" targets (last clicked)
let view = null; // {t0, t1} — null until the first series arrives
// Stacked (one lane per channel) vs overlay, kept for the browser session
// only — sessionStorage is local, nothing leaves the page. Storage can be
// blocked by privacy settings; the toggle then just doesn't survive a
// reload, which never justifies failing the page.
let stacked = false;
try {
  stacked = sessionStorage.getItem("plot-mode") === "stacked";
} catch {
  // see above
}
// X tick labels: seconds from the recording start (default) against local
// wall clock anchored at it. Session-only too, same storage caveats. The
// internal math stays in seconds either way; only the label strings switch.
let absoluteTime = false;
try {
  absoluteTime = sessionStorage.getItem("time-mode") === "absolute";
} catch {
  // see above
}
let epoch = 0; // tags series requests; responses with an old id are stale
let pending = 0; // series requests in flight, for the status line
let cursor = null; // {t} while the pointer is over the plot
let cursorA = null; // {t, values} — placed by a click; values pin the readout
let cursorB = null; // {t} — placed by a shift-click
let viewStats = null; // {name, t0, t1, data} — selected channel over the view
let regionStats = null; // {t0, t1, byName} — shown channels between A and B
let selRect = null; // {x0, y0, x1, y1} while a shift-drag selection is live
let sampleInFlight = false;
let sampleQueuedT = null;
let fileOpen = false;
let fileName = "";
let fileBytes = 0;
// The same-origin path the current file came from, when it did (sample
// button or ?file=) — a dropped local file keeps this null.
let deepLinkFile = null;
// Sample table (plan 2.2): index-paged, virtualized. One page of raw samples
// is held at a time; the spacer body keeps the scrollbar honest for any
// sample count. tableEpoch tags requests; late replies are dropped.
let tableOpen = false;
let tableEpoch = 0;
let tablePage = null; // {start, total, times, values?|labels?}
// Last seen sample count, kept while a new page is in flight: zeroing the
// spacer for the fetch would collapse the scroll range and throw the user's
// position back to the top on every page change.
let tableTotal = 0;
const TABLE_ROW_H = 22;
const TABLE_PAGE = 600;
// Channel details (plan 2.5): metadata of the selected channel, fetched only
// while the panel is open (one round trip per selection change).
let detailsOpen = false;
// Channel search (plan 2.3): contains/wildcard/exact run against the
// reader's name index in the worker; regex filters the name list with the
// platform RegExp. Results are tagged with their query, mode and epoch, so
// a reply for a since-edited box never replaces the list.
let searchEpoch = 0;
let searchResults = null; // {q, mode, names}
// OPFS persistence (plan 4.5): an opt-in per-file cache in the Origin
// Private File System. Reopening costs a local read instead of a re-download,
// and survives reloads. Opt-in because browser storage is not infinite: the
// cache is LRU-capped in entries and refuses files past a size cap.
let opfsAvailable = false;
const OPFS_MAX_FILES = 5;
const OPFS_MAX_BYTES = 512 * 1024 * 1024; // 512 MB total across cached files
let opfsRoot = null;

async function opfsInit() {
  try {
    opfsRoot = await navigator.storage.getDirectory();
    opfsAvailable = true;
    await opfsRenderList();
  } catch {
    opfsAvailable = false; // no OPFS: the landing just shows no cache list
  }
}

// Sorted oldest-first, so eviction is a plain shift() off the entries.
async function opfsEntries() {
  const names = [];
  for await (const [name] of opfsRoot.entries()) {
    if (name.startsWith("cached:")) names.push(name);
  }
  const dated = [];
  for (const name of names) {
    const handle = await opfsRoot.getFileHandle(name);
    const file = await handle.getFile();
    dated.push({ name, lastModified: file.lastModified, size: file.size });
  }
  dated.sort((a, b) => a.lastModified - b.lastModified);
  return dated;
}

async function opfsStore(name, bytes) {
  if (!opfsAvailable) return false;
  try {
    // Enforce the caps before writing: total size and entry count, evicting
    // oldest-first. A file bigger than the whole cap is refused outright.
    if (bytes.byteLength > OPFS_MAX_BYTES / 2) return false;
    let entries = await opfsEntries();
    let totalSize = entries.reduce((sum, e) => sum + e.size, 0);
    while ((entries.length + 1 > OPFS_MAX_FILES || totalSize + bytes.byteLength > OPFS_MAX_BYTES) && entries.length > 0) {
      const oldest = entries.shift();
      await opfsRoot.removeEntry(oldest.name);
      totalSize -= oldest.size;
      entries = await opfsEntries();
    }
    const handle = await opfsRoot.getFileHandle(`cached:${name}`, { create: true });
    const writable = await handle.createWritable();
    await writable.write(bytes);
    await writable.close();
    await opfsRenderList();
    return true;
  } catch {
    return false; // quota or privacy failure: caching is best-effort
  }
}

async function opfsRenderList() {
  const host = document.getElementById("cached-files");
  if (!host) return;
  host.replaceChildren();
  if (!opfsAvailable) return;
  const entries = await opfsEntries();
  if (entries.length === 0) return;
  const head = document.createElement("p");
  head.className = "hint";
  head.textContent = "Files kept in this browser:";
  host.append(head);
  for (const entry of entries) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.textContent = `${entry.name.slice(7)} (${humanBytes(entry.size)}) — open`;
    btn.addEventListener("click", async () => {
      try {
        const handle = await opfsRoot.getFileHandle(entry.name);
        const buf = await (await handle.getFile()).arrayBuffer();
        deepLinkFile = null; // a cached copy is not a shareable URL
        openFile(buf, entry.name.slice(7));
      } catch (e) {
        showError(`Could not reopen ${entry.name.slice(7)}: ${e.message ?? e}`);
      }
    });
    const rm = document.createElement("button");
    rm.type = "button";
    rm.textContent = "✕";
    rm.title = "remove from browser storage";
    rm.addEventListener("click", async () => {
      await opfsRoot.removeEntry(entry.name);
      await opfsRenderList();
    });
    host.append(btn, rm);
  }
}

// A DBC held until the file that owns it has opened (the deep link fetches
// both; the worker's FIFO ordering makes the attach land after open).
let pendingDbc = null;
// Shareable view state (plan 4.4): {file, channels, window, mode} in the
// URL hash, updated debounced after every state change. The hash never
// reloads the page and never leaves the machine — it is only a bookmark.
let hashTimer = null;
// View state to restore once the shared file has opened (channels are
// toggled after the first series bootstraps the view).
let hashPending = false;
// Two-file compare (plan 4.3): a second open file contributes channels
// named `@<label>::<channel>`; the prefix routes every request on the
// worker side and reads as provenance in the legend.
let compareLabel = null;
let pendingCompare = null; // [label, ArrayBuffer] held until the file opens
// Bus frame panel (plan 3.3): the file's CAN/LIN groups and the currently
// shown page, virtualized exactly like the sample table.
let busGroups = []; // [{kind, group, frames}]
let busGroupIndex = 0;
let busOpen = false;
let busTag = 0;
let busPage = null; // {start, total, rows}
let busHighlight = -1; // row index under the cursor (cursor link)
const BUS_PAGE = 600;
// X-Y panel (plan 4.1): two plotted channels as value-vs-value points.
let xyOpen = false;
let xyPair = null; // {nameX, nameY, xs, ys, count}
// GPS panel (plan 3.4): a detected lat/lon pair plus its decoded track.
let gpsPair = null; // {latitude, longitude}
let gpsSpeed = null; // channel name, when one looks like a GPS speed
let gpsOpen = false;
let gpsTrack = null; // {n, t, lat, lon, speed, aligned}
// Recording start as epoch ms — the wall-clock anchor for absolute labels
// and cursor timestamps. null until a file proves it parses (see onMeta).
let startEpochMs = null;
// Structure tab (plan 4.8): the file's outline in one worker round trip,
// then drawn and filtered entirely on the main thread. Per-section open/
// closed state lives in a map so it survives re-renders; it resets per file,
// like every other view state.
let structureOpen = false;
let structureData = null; // parsed `structure` reply for the open file
let structureEpoch = 0; // tags structure requests; late replies are dropped
let structState = new Map(); // section key -> boolean (open?)
// Same cap the desktop tree draws before it points at the channel list:
// a group with ten thousand channels would build ten thousand rows the
// moment it is opened.
const MAX_TREE_CHANNELS = 400;

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

// Why a channel of a non-numeric, non-text, non-array kind cannot be plotted
// yet. The kind strings come from the Rust side (channels()/channel_kind).
function kindMessage(kind, name) {
  if (kind === "bytes") {
    return `${name} holds opaque per-sample bytes — there is no numeric series to plot (the table view shows them as hex).`;
  }
  return `${name} decodes as ${kind}, which has no scalar series to plot.`;
}

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
      worker.postMessage({ type: "bus-groups" });
      worker.postMessage({ type: "gps-detect" });
      if (pendingDbc) {
        worker.postMessage({ type: "attach-dbc", bytes: pendingDbc }, [pendingDbc]);
        pendingDbc = null;
      }
      if (pendingCompare) {
        const [label, bytes] = pendingCompare;
        worker.postMessage({ type: "open-second", label, bytes }, [bytes]);
        pendingCompare = null;
      }
      break;
      if (pendingDbc) {
        worker.postMessage({ type: "attach-dbc", bytes: pendingDbc }, [pendingDbc]);
        pendingDbc = null;
      }
      if (pendingCompare) {
        const [label, bytes] = pendingCompare;
        worker.postMessage({ type: "open-second", label, bytes }, [bytes]);
        pendingCompare = null;
      }
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
    case "stats":
      onStats(msg);
      break;
    case "csv":
      onCsv(msg);
      break;
    case "table":
      onTable(msg);
      break;
    case "search":
      if (msg.id === searchEpoch) {
        searchResults = { q: msg.q, mode: msg.mode, names: msg.names };
        renderChannelList();
      }
      break;
    case "details":
      if (detailsOpen && msg.name === selected) renderDetails(msg.details);
      break;
    case "structure":
      // Echoed epoch, like the table and search flows: a reply for a file
      // that has since been replaced is dropped instead of drawn.
      if (msg.epoch !== structureEpoch) break;
      try {
        structureData = JSON.parse(msg.structure);
      } catch {
        structureData = null;
      }
      if (structureOpen) renderStructure();
      break;
    case "xy":
      if (xyOpen && msg.nameX === xyxEl.value && msg.nameY === xyyEl.value) {
        xyPair = { nameX: msg.nameX, nameY: msg.nameY, xs: msg.xs, ys: msg.ys, count: msg.count };
        drawXy();
        xycountEl.textContent = `${msg.count.toLocaleString()} points`;
      }
      break;
    case "computed": {
      // The computed channel is in the listing now; a re-read of the meta
      // puts it in the list, and the user plots it like any channel.
      plotMsg(`computed channel "${msg.name}" added`);
      worker.postMessage({ type: "meta" });
      break;
    }
    case "open-second":
      compareLabel = msg.label;
      // Merge the second file's channel metadata (names already prefixed on
      // the worker side) into the list; both files' channels plot together.
      try {
        const extra = JSON.parse(msg.channels);
        const existing = new Set(channels.map((c) => c.name));
        for (const c of extra) {
          c.name = `@${msg.label}::${c.name}`;
          if (!existing.has(c.name)) channels.push(c);
        }
        renderChannelList();
        renderLegend();
        plotMsg(`second file open — ${extra.length} channels joined the list as @${msg.label}::name`);
      } catch {
        plotMsg("the second file's channel list could not be read");
      }
      break;
    case "gps-detect": {
      try {
        gpsPair = JSON.parse(msg.detection);
      } catch {
        gpsPair = null;
      }
      const found =
        gpsPair && gpsPair.latitude !== null && gpsPair.longitude !== null;
      gpsBtn.hidden = !found;
      if (found) {
        gpsSpeed =
          channels.find((c) => /gps/i.test(c.name) && /speed/i.test(c.name))
            ?.name ?? null;
      } else if (gpsOpen) {
        gpsOpen = false;
        gpspanelEl.hidden = true;
      }
      break;
    }
    case "gps-track":
      try {
        gpsTrack = JSON.parse(msg.track);
      } catch {
        gpsTrack = null;
      }
      drawGps();
      break;
    case "bus-groups": {
      try {
        busGroups = JSON.parse(msg.groups);
      } catch {
        busGroups = [];
      }
      busBtn.hidden = busGroups.length === 0;
      if (busGroups.length === 0 && busOpen) {
        busOpen = false;
        buspanelEl.hidden = true;
      }
      break;
    }
    case "bus-frames":
      if (msg.tag === busTag && busOpen) {
        try {
          busPage = JSON.parse(msg.page);
          renderBusPanel();
        } catch {
          // a malformed page leaves the previous view; the next fetch repaints
        }
      }
      break;
    case "bus-locate":
      if (busOpen && msg.index !== undefined) {
        busHighlight = msg.index;
        busscrollEl.scrollTop = Math.max(
          0,
          msg.index * TABLE_ROW_H - busscrollEl.clientHeight / 2
        );
        // The scroll may or may not trigger a page fetch; repaint either way
        // so the highlighted row is on screen.
        requestBusPage();
        renderBusPanel();
      }
      break;
    case "attach-dbc":
      // The decoded channels are ordinary file channels on the Rust side;
      // re-reading the meta rebuilds the list with them included.
      setStatus("");
      plotMsg(
        msg.signals > 0
          ? `DBC attached: ${msg.signals} signal${msg.signals === 1 ? "" : "s"} decoded — they are in the channel list under their message names.`
          : "DBC attached, but no logged identifier matched it."
      );
      worker.postMessage({ type: "meta" });
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

  // Wall-clock anchor. The worker sends ISO8601 UTC with milliseconds
  // ("2021-12-20T04:26:40.123Z"), which Date.parse takes as-is; a start time
  // it rejects (odd writer, pre-epoch garbage) keeps the axis relative — a
  // missing absolute mode beats wrong datetimes, and nothing else breaks.
  const parsedStart = Date.parse(info.start_time);
  startEpochMs = Number.isFinite(parsedStart) ? parsedStart : null;
  if (!startEpochMs) absoluteTime = false;
  renderTimeButtons();

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
  // A structure tab left open across a file switch refetches for the new
  // file (openFile cleared the old snapshot).
  if (structureOpen) {
    if (structureData) renderStructure();
    else requestStructure();
  }
  if (hashPending) {
    hashPending = false;
    // The channel toggles and window restore land once the list is ready.
    queueMicrotask(applyHashState);
  }
  renderLegend();
  scheduleHashUpdate(); // the hash follows the plotted set and the view
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
  if (msg.kind === "text") {
    // A label series draws as state bands: the runs, the vocabulary that
    // keeps band rows stable across zooms, and whether the budget had to
    // sample the changes (shown once, then not repeated).
    entry.kind = "text";
    entry.labels = msg.labels;
    entry.vocab = msg.vocab;
    entry.truncated = msg.truncated;
    entry.vs = null;
    entry.min = null;
    entry.max = null;
    if (msg.truncated) {
      plotMsg(
        `${msg.name} changes state faster than the plot has pixels — bands are sampled, the stats strip stays exact.`
      );
    }
  } else {
    // Numeric — including an array channel's plotted element (kind "array":
    // the line is element `msg.element`, and the reply bounds the stepper).
    entry.kind = msg.kind === "array" ? "array" : "f64";
    if (msg.kind === "array") {
      entry.element = msg.element;
      entry.elements = msg.elements;
    }
    entry.vs = msg.values;
    entry.labels = null;
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
  }

  renderLegend();
  scheduleHashUpdate(); // the hash follows the plotted set and the view
  requestAnimationFrame(draw);
}

function onSample(msg) {
  sampleInFlight = false;
  for (const entry of shown) {
    entry.at = msg.values[entry.name] ?? null;
  }
  // A response at cursor A's time also pins a snapshot: entry.at is transient
  // hover state, but the pinned readout has to survive later hover queries.
  if (cursorA && msg.t === cursorA.t) {
    cursorA.values = msg.values;
  }
  renderReadout();
  if (sampleQueuedT !== null) {
    const t = sampleQueuedT;
    sampleQueuedT = null;
    sendSampleQuery(t);
  }
}

function onStats(msg) {
  let st = null;
  try {
    st = JSON.parse(msg.stats);
  } catch {
    return; // nothing to show from a malformed payload; the next request repaints
  }
  // Route by the exact window that was asked for, so a reply for a view or
  // region that has since been replaced is dropped instead of overwriting.
  if (viewStats && msg.name === viewStats.name && msg.t0 === viewStats.t0 && msg.t1 === viewStats.t1) {
    viewStats.data = st;
    renderStatsbar();
  }
  if (regionStats && msg.t0 === regionStats.t0 && msg.t1 === regionStats.t1) {
    regionStats.byName.set(msg.name, st);
    renderStatsbar();
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
// Sample table (plan 2.2). Whole-series, index-addressed pages over the raw
// cache — the same arrays the cursor readout probes, so a row's value and a
// readout at that sample can never disagree. Virtualized: the scroll body is
// a spacer sized `total * ROW_H`, and only one page (~600 rows) of cells
// exists at a time, positioned at its start index.

function onTable(msg) {
  if (msg.id !== tableEpoch || msg.name !== selected || !tableOpen) return;
  tablePage = msg;
  renderTable();
}

// The visible row range the current page covers, or null.
function pageCoverage() {
  if (!tablePage) return null;
  return [tablePage.start, tablePage.start + tablePage.times.length];
}

function requestTablePage() {
  if (!tableOpen || !selected || !fileOpen) return;
  const first = Math.floor(tablescrollEl.scrollTop / TABLE_ROW_H);
  const visible = Math.ceil(tablescrollEl.clientHeight / TABLE_ROW_H) + 1;
  const lo = Math.max(0, first - 20);
  const hi = first + visible + 20;
  const cov = pageCoverage();
  if (cov && lo >= cov[0] && hi <= cov[1]) return; // the page we hold covers it
  tableEpoch += 1;
  tablePage = null;
  worker.postMessage({
    type: "table",
    id: tableEpoch,
    name: selected,
    start: Math.max(0, first - 100),
    count: TABLE_PAGE,
  });
  renderTable();
}

function toggleTable(force) {
  tableOpen = force !== undefined ? force : !tableOpen;
  tableBtn.setAttribute("aria-pressed", String(tableOpen));
  tablepanelEl.hidden = !tableOpen;
  if (tableOpen) {
    tablePage = null;
    renderTableHead();
    requestTablePage();
  }
}

function renderTableHead() {
  const entry = shown.find((s) => s.name === selected);
  tableheadEl.replaceChildren();
  if (!entry) return;
  const nameCol =
    entry.kind === "array"
      ? `${entry.name}[${entry.element}] · all elements`
      : entry.unit
        ? `${entry.name} [${entry.unit}]`
        : entry.name;
  const cols = ["#", "t [s]", nameCol];
  for (const c of cols) {
    const cell = document.createElement("span");
    cell.textContent = c;
    tableheadEl.append(cell);
  }
}

// One table value cell for the page's kind. A numeric page shows numbers, a
// text page labels; an array page shows every element of the sample (a
// bracketed list, truncated past four — the cell is a summary, the plot's
// element stepper is the precise view); a bytes page shows the hex string
// Rust already formatted (also truncated, at 16 bytes).
function tableCell(kind, page, i) {
  if (kind === "text") return page.labels[i] ?? "—";
  if (kind === "bytes") return page.hex[i] ?? "—";
  if (kind === "array") {
    const from = page.eps ? i * page.eps : page.starts[i];
    const to = page.eps ? (i + 1) * page.eps : page.starts[i + 1] ?? from;
    const parts = [];
    for (let j = from; j < to && parts.length < 4; j++) {
      const v = page.elems[j];
      parts.push(Number.isFinite(v) ? fmtNumber(v) : "—");
    }
    if (to - from > 4) parts.push(`… (${to - from})`);
    return `[${parts.join(", ")}]`;
  }
  const v = page.values[i];
  return v === null || v === undefined || !Number.isFinite(v) ? "—" : fmtNumber(v);
}

function renderTable() {
  if (tablePage?.total) tableTotal = tablePage.total;
  const total = tablePage?.total ?? tableTotal;
  tablebodyEl.style.height = `${total * TABLE_ROW_H}px`;
  // Drop only the old page rows — never re-append the spacer itself (a
  // detach/reattach resets the scroll position, which would turn every
  // page fetch into a fetch of page 0).
  tablebodyEl.querySelector(".tablepage")?.remove();
  tablefootEl.textContent = tableOpen && selected
    ? tablePage
      ? `rows ${tablePage.start.toLocaleString()}–${(tablePage.start + tablePage.times.length - 1).toLocaleString()} of ${total.toLocaleString()}`
      : "…"
    : "";
  if (!tablePage) return;

  const pageEl = document.createElement("div");
  pageEl.className = "tablepage";
  pageEl.style.top = `${tablePage.start * TABLE_ROW_H}px`;
  const kind = tablePage.kind ?? "f64";
  const n = tablePage.times.length;
  for (let i = 0; i < n; i++) {
    const row = document.createElement("div");
    row.className = "tablerow";
    const idx = document.createElement("span");
    idx.textContent = (tablePage.start + i).toLocaleString();
    row.append(idx);
    const t = document.createElement("span");
    t.textContent = fmtNumber(tablePage.times[i]);
    row.append(t);
    const v = document.createElement("span");
    v.textContent = tableCell(kind, tablePage, i);
    row.append(v);
    pageEl.append(row);
  }
  tablebodyEl.append(pageEl);
}

// ---------------------------------------------------------------------------
// Requests to the worker

let requestTimer = null;

function scheduleRequestAll() {
  clearTimeout(requestTimer);
  requestTimer = setTimeout(requestAll, REDECODE_DEBOUNCE_MS);
  scheduleHashUpdate();
}

// Encodes the current view into the hash, silently skipping any file that
// is not a same-origin URL (a dropped local file has nothing to link to).
function scheduleHashUpdate() {
  if (!fileOpen || !view || !deepLinkFile) return;
  clearTimeout(hashTimer);
  hashTimer = setTimeout(() => {
    const params = new URLSearchParams();
    params.set("f", deepLinkFile);
    if (shown.length) params.set("c", shown.map((s) => s.name).join(","));
    params.set("t0", view.t0.toPrecision(10));
    params.set("t1", view.t1.toPrecision(10));
    if (stacked) params.set("m", "stacked");
    history.replaceState(null, "", `#${params.toString()}`);
  }, HASH_DEBOUNCE_MS);
}
const HASH_DEBOUNCE_MS = 400;

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
      element: s.element ?? 0,
      maxPoints: maxPoints(),
    });
  }
  requestViewStats(); // same debounce cadence: the strip follows the settled view
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
    element: 0,
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
// Statistics requests (region between the cursors, selected channel over the
// view) — one `stats` round trip per channel per region change, and one per
// view settle through the same debounce as the series windows. Both come back
// through onStats, which drops replies whose window no longer matches.

function requestRegionStats() {
  if (!cursorA || !cursorB || shown.length === 0) {
    regionStats = null;
    return;
  }
  const lo = Math.min(cursorA.t, cursorB.t);
  const hi = Math.max(cursorA.t, cursorB.t);
  // Cursors move only on discrete clicks, so no debounce is needed; a new
  // region resets the table, an unchanged one only fetches channels it
  // doesn't have yet (e.g. one added while the region was live).
  if (!regionStats || regionStats.t0 !== lo || regionStats.t1 !== hi) {
    regionStats = { t0: lo, t1: hi, byName: new Map() };
  }
  for (const s of shown) {
    // Array and byte channels have no scalar statistics — the Rust endpoint
    // refuses them, so the request is never made (the region row says why).
    if (s.kind === "array" || s.kind === "bytes") continue;
    if (!regionStats.byName.has(s.name)) {
      worker.postMessage({ type: "stats", name: s.name, t0: lo, t1: hi });
    }
  }
}

function requestViewStats() {
  if (!selected || !view || shown.length === 0) {
    viewStats = null;
    return;
  }
  const entry = shown.find((s) => s.name === selected);
  if (!entry || entry.kind === "array" || entry.kind === "bytes") {
    // No scalar statistics exist for these kinds; an empty strip beats an
    // error message on every view settle.
    viewStats = null;
    return;
  }
  // While a region is live the strip shows the region; the view figures come
  // back when it clears (clearCursors → refreshStats).
  if (cursorA && cursorB) return;
  // data: null renders as a placeholder until the reply lands, so a changed
  // view or selection never keeps showing the previous window's figures.
  viewStats = { name: selected, t0: view.t0, t1: view.t1, data: null };
  worker.postMessage({ type: "stats", name: selected, t0: view.t0, t1: view.t1 });
  renderStatsbar();
}

function refreshStats() {
  requestRegionStats();
  requestViewStats();
  renderStatsbar();
}

// ---------------------------------------------------------------------------
// Channel overlay management

// ---------------------------------------------------------------------------
// Bus frame panel (plan 3.3). The same virtualized paging as the sample
// table, over the file's CAN/LIN groups. Cursor link, both directions: a
// click on a frame places cursor A at that frame's time, and a placed cursor
// A locates its nearest frame (Rust-side binary search) and scrolls it in.

function toggleBus(force) {
  busOpen = force !== undefined ? force : !busOpen;
  busBtn.setAttribute("aria-pressed", String(busOpen));
  buspanelEl.hidden = !busOpen;
  if (busOpen) {
    busPage = null;
    renderBusHead();
    requestBusPage();
  }
}

function requestBusPage() {
  if (!busOpen || busGroups.length === 0) return;
  const g = busGroups[busGroupIndex] ?? busGroups[0];
  const first = Math.floor(busscrollEl.scrollTop / TABLE_ROW_H);
  const visible = Math.ceil(busscrollEl.clientHeight / TABLE_ROW_H) + 1;
  const lo = Math.max(0, first - 20);
  const hi = first + visible + 20;
  if (busPage && busPage.start <= lo && hi <= busPage.start + busPage.count) {
    return; // held page covers the view
  }
  busTag += 1;
  worker.postMessage({
    type: "bus-frames",
    tag: busTag,
    kind: g.kind,
    group: g.group,
    start: Math.max(0, first - 100),
    count: BUS_PAGE,
  });
}

function renderBusHead() {
  busheadEl.replaceChildren();
  const g = busGroups[busGroupIndex];
  if (!g) return;
  const label = document.createElement("span");
  label.textContent = `${g.kind.toUpperCase()} · ${g.frames.toLocaleString()} frames`;
  busheadEl.append(label);
  if (busGroups.length > 1) {
    const sel = document.createElement("select");
    sel.setAttribute("aria-label", "Bus group");
    busGroups.forEach((bg, i) => {
      const opt = document.createElement("option");
      opt.value = String(i);
      opt.textContent = `${bg.kind} group ${bg.group} (${bg.frames.toLocaleString()})`;
      sel.append(opt);
    });
    sel.value = String(busGroupIndex);
    sel.addEventListener("change", () => {
      busGroupIndex = Number(sel.value);
      busPage = null;
      busscrollEl.scrollTop = 0;
      renderBusHead();
      requestBusPage();
    });
    busheadEl.append(sel);
  }
  const hint = document.createElement("span");
  hint.className = "bushint";
  hint.textContent = "click a row to place cursor A at that frame";
  busheadEl.append(hint);
}

// One page of frame rows; columns t / id (hex) / dlc / data / ext / bus.
function renderBusPanel() {
  const g = busGroups[busGroupIndex];
  const total = g ? g.frames : 0;
  busbodyEl.style.height = `${total * TABLE_ROW_H}px`;
  busbodyEl.querySelector(".buspage")?.remove();
  if (!busOpen || !busPage) return;
  const pageEl = document.createElement("div");
  pageEl.className = "buspage";
  pageEl.style.top = `${busPage.start * TABLE_ROW_H}px`;
  busPage.rows.forEach((r, i) => {
    const idx = busPage.start + i;
    const row = document.createElement("div");
    row.className = "tablerow busrow";
    if (idx === busHighlight) row.classList.add("hl");
    const cells = [
      idx.toLocaleString(),
      fmtNumber(r.t),
      "0x" + r.id.toString(16).toUpperCase(),
      String(r.dlc),
      r.data,
      r.ext === null ? "" : r.ext ? "EXT" : "STD",
      String(r.bus ?? ""),
    ];
    for (const c of cells) {
      const cell = document.createElement("span");
      cell.textContent = c;
      row.append(cell);
    }
    row.addEventListener("click", () => {
      placeCursor(r.t, false); // frame → plot: cursor A at the frame's time
    });
    pageEl.append(row);
  });
  busbodyEl.append(pageEl);
}

function refreshBus() {
  if (!busOpen) return;
  busPage = null;
  busHighlight = -1;
  renderBusHead();
  requestBusPage();
}

busscrollEl.addEventListener("scroll", () => requestBusPage());

function refreshTable() {
  if (!tableOpen) return;
  renderTableHead();
  tableEpoch += 1; // drop any in-flight page for the previous selection
  tablePage = null;
  tableTotal = 0; // a different series: the old count is wrong now
  tablescrollEl.scrollTop = 0; // a new series starts at its first sample
  requestTablePage();
}

// Details panel (plan 2.5): metadata of the selected channel. The reply is
// the Rust-built JSON, rendered as label/value rows; a selection change
// re-requests, and a reply for a since-replaced selection is dropped.
function toggleDetails(force) {
  detailsOpen = force !== undefined ? force : !detailsOpen;
  detailsBtn.setAttribute("aria-pressed", String(detailsOpen));
  detailspanelEl.hidden = !detailsOpen;
  if (detailsOpen) refreshDetails();
}

function refreshDetails() {
  if (!detailsOpen) return;
  if (!selected) {
    detailsheadEl.textContent = "";
    detailsbodyEl.textContent = "";
    return;
  }
  detailsheadEl.textContent = selected;
  detailsbodyEl.textContent = "…";
  worker.postMessage({ type: "details", name: selected });
}

function renderDetails(json) {
  let d;
  try {
    d = JSON.parse(json);
  } catch {
    return;
  }
  detailsheadEl.textContent = d.name;
  detailsbodyEl.replaceChildren();
  const rows = [
    ["kind", d.kind],
    ["unit", d.unit || "—"],
    ["group", d.group],
    ["samples", d.samples ? d.samples.toLocaleString() : "—"],
    ["data type", d.data_type],
    ["bits", d.bit_count],
    ["master", d.master ? "yes" : "no"],
    ["conversion", d.conversion],
    ["description", d.description || "—"],
  ];
  if (d.array_shape) rows.splice(6, 0, ["array shape", d.array_shape.join(" × ")]);
  if (d.min !== undefined && d.min !== null) rows.splice(8, 0, ["declared min", fmtNumber(d.min)]);
  if (d.max !== undefined && d.max !== null) rows.splice(9, 0, ["declared max", fmtNumber(d.max)]);
  for (const [k, v] of rows) {
    const row = document.createElement("div");
    row.className = "detailrow";
    const key = document.createElement("span");
    key.textContent = k;
    row.append(key);
    const val = document.createElement("span");
    val.textContent = v === null || v === undefined ? "—" : String(v);
    row.append(val);
    detailsbodyEl.append(row);
  }
}

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
      cursorA = null; // cursors mean nothing without a plotted channel
      cursorB = null;
      readoutEl.hidden = true;
      if (tableOpen) toggleTable(false);
      if (detailsOpen) toggleDetails(false);
    }
    renderChannelList();
    renderLegend();
    renderReadout();
    refreshStats(); // re-target the strip at the new selection (or hide it)
    refreshTable(); // the table follows the selection too
    refreshDetails();
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
  const kind = meta?.kind ?? "f64";
  if (kind !== "f64" && kind !== "text" && kind !== "array") {
    // Honest refusal of the plot while there is no per-sample view for the
    // kind: NaN lines or a silent empty lane would be the lie this guard
    // replaces. The channel still becomes the selection — its sample table
    // and details are exactly the honest views it does have.
    plotMsg(kindMessage(kind, name));
    selected = name;
    renderChannelList();
    renderLegend();
    refreshTable();
    refreshDetails();
    return;
  }

  shown.push({
    name,
    unit: meta?.unit ?? "",
    kind,
    color: COLORS[shown.length % COLORS.length],
    tMin: null,
    tMax: null,
    ts: null,
    vs: null,
    labels: null, // text channels: run-collapsed labels parallel to ts
    vocab: null, // text channels: first-appearance label order (band rows)
    truncated: false,
    element: 0, // array channels: the plotted element index
    elements: null, // array channels: selectable element count
    min: null,
    max: null,
    at: null,
  });
  selected = name;
  renderChannelList();
  renderLegend();
  refreshTable(); // a newly plotted channel becomes the selection
  refreshDetails();
  // requestAll (below) re-targets the view stats at the new selection; a
  // live region only needs the newcomer's figures.
  requestRegionStats();
  renderStatsbar();
  if (view) requestAll();
  else requestBootstrap(name);
}

function renderChannelList() {
  const q = filterInput.value.trim();
  const mode = searchModeEl.value;
  // Worker search results when they match the box; while a reply is in
  // flight the old instant-substring filter keeps the list responsive
  // (regex/wildcard users just see the previous view one frame longer).
  const searched =
    searchResults && searchResults.q === q && searchResults.mode === mode ? searchResults.names : null;
  const on = shownNames();
  const shownList = q
    ? searched
      ? searched.map((name) => channels.find((c) => c.name === name) ?? { name })
      : channels.filter((c) => c.name.toLowerCase().includes(q.toLowerCase()))
    : channels;
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
    const unplotable = c.kind && c.kind !== "f64" && c.kind !== "text" && c.kind !== "array";
    if (unplotable) {
      li.classList.add("unplotable");
      li.title = kindMessage(c.kind, c.name);
      // Not plotted, but still routed through toggleChannel: the refusal
      // message and the selection (for table/details) both live there.
      li.addEventListener("click", () => toggleChannel(c.name));
      channelList.append(li);
      continue;
    }
    if (c.kind === "text") li.classList.add("istext");
    if (c.kind === "array") li.classList.add("isarray");
    const tip = [c.name, c.group && `group: ${c.group}`, c.unit && `unit: ${c.unit}`, c.description]
      .filter(Boolean)
      .join("\n");
    li.title = tip;
    if (on.has(c.name)) li.classList.add("selected");
    li.addEventListener("click", () => toggleChannel(c.name));
    channelList.append(li);
  }
  updateStructurePlotState();
}

// ---------------------------------------------------------------------------
// Structure tab (plan 4.8): the file's internal outline — identification and
// header blocks, history, attachments, events, channel hierarchy, and the
// data groups down to individual channels — the browser twin of the desktop
// viewer's structure panel. One worker round trip delivers the whole outline
// as JSON; collapse state, filtering and plotting are main-thread work over
// that snapshot, exactly like the channel list above.

function requestStructure() {
  if (!fileOpen) return;
  structureEpoch += 1;
  structtreeEl.replaceChildren();
  const waiting = document.createElement("div");
  waiting.className = "filesub";
  waiting.textContent = "Reading the file's structure…";
  structtreeEl.append(waiting);
  worker.postMessage({ type: "structure", epoch: structureEpoch });
}

function structOpen(key, def) {
  return structState.has(key) ? structState.get(key) : def;
}

function structToggle(key) {
  structState.set(key, !structOpen(key, false));
  renderStructure();
}

// Every collapsible key the current snapshot can render — the expand/collapse
// buttons walk this instead of guessing at section names.
function structAllKeys() {
  const keys = ["history", "attachments", "events", "hierarchy", "dgs"];
  if (!structureData) return keys;
  structureData.data_groups.forEach((dg, i) => {
    keys.push(`dg:${i}`);
    dg.channel_groups.forEach((cg, j) => {
      keys.push(`cg:${i}:${j}`);
      if (cg.reductions.length) keys.push(`sr:${i}:${j}`);
    });
  });
  const walk = (nodes, path) => {
    nodes.forEach((node, i) => {
      keys.push(`hn:${path}${i}`);
      walk(node.children ?? [], `${path}${i}.`);
    });
  };
  walk(structureData.hierarchy, "");
  return keys;
}

function setAllStructSections(open) {
  for (const key of structAllKeys()) structState.set(key, open);
  renderStructure();
}

function staticRow(label, cls) {
  const row = document.createElement("div");
  row.className = `row static${cls ? ` ${cls}` : ""}`;
  row.textContent = label;
  return row;
}

// One collapsible section: a caret row plus, when open, its body. `open` is
// resolved by the caller — the filter forces data-group sections open the
// way the desktop tree does, the rest keep their own state.
function appendSection(host, key, label, open, fill) {
  const row = document.createElement("div");
  row.className = "row clickable";
  const caret = document.createElement("span");
  caret.className = "caret";
  caret.textContent = open ? "▼" : "▶";
  const name = document.createElement("span");
  name.className = "sectionlabel";
  name.textContent = label;
  row.append(caret, name);
  row.addEventListener("click", () => structToggle(key));
  host.append(row);
  if (!open) return;
  const body = document.createElement("div");
  body.className = "children";
  fill(body);
  host.append(body);
}

// One channel line of the tree. Clicking toggles the channel exactly like a
// channel-list row does — same toggleChannel path, same refusal for kinds
// that have nothing to plot.
function appendChannelRow(host, ch) {
  const plottable = ch.kind === "f64" || ch.kind === "text" || ch.kind === "array";
  const on = shownNames().has(ch.name);
  const row = document.createElement("div");
  row.className = "row chrow";
  row.dataset.name = ch.name;
  if (!plottable) row.classList.add("unplotable");
  if (on) row.classList.add("selected");
  if (ch.kind === "text") row.classList.add("istext");
  if (ch.kind === "array") row.classList.add("isarray");
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = on;
  box.tabIndex = -1;
  row.append(box);
  if (on) {
    const dot = document.createElement("span");
    dot.className = "dot";
    dot.style.background = shown.find((s) => s.name === ch.name)?.color ?? "";
    row.append(dot);
  }
  const label = document.createElement("span");
  label.className = "chlabel";
  label.textContent =
    (ch.unit ? `${ch.name} [${ch.unit}]` : ch.name) + (ch.master ? "  (master)" : "");
  row.append(label);
  if (ch.kind === "text" || ch.kind === "array") {
    const tag = document.createElement("span");
    tag.className = "kind-tag";
    row.append(tag);
  }
  if (ch.unreadable) {
    const warn = document.createElement("span");
    warn.className = "unreadable-tag";
    warn.textContent = "⚠";
    warn.title = ch.unreadable;
    row.append(warn);
  }
  row.title = [ch.unreadable, plottable ? null : kindMessage(ch.kind, ch.name)]
    .filter(Boolean)
    .join("\n");
  row.addEventListener("click", () => toggleChannel(ch.name));
  host.append(row);
}

function appendHierarchyNode(host, node, path, depth) {
  const key = `hn:${path}`;
  const open = structOpen(key, false);
  const row = document.createElement("div");
  row.className = "row clickable";
  const caret = document.createElement("span");
  caret.className = "caret";
  caret.textContent = open ? "▼" : "▶";
  const name = document.createElement("span");
  name.textContent = node.name || `node ${path}`;
  row.append(caret, name);
  row.addEventListener("click", () => structToggle(key));
  host.append(row);
  if (!open) return;
  const body = document.createElement("div");
  body.className = "children";
  if (node.unresolved > 0) {
    body.append(
      staticRow(
        node.unresolved === 1
          ? "(a channel this node names is not in the file)"
          : `(${node.unresolved} channels this node names are not in the file)`,
        "muted"
      )
    );
  }
  for (const name of node.channels) {
    // The hierarchy names channels; the metadata list supplies kind/unit so
    // the row behaves exactly like a data-group channel row.
    const meta = channels.find((c) => c.name === name);
    appendChannelRow(body, {
      name,
      unit: meta?.unit ?? "",
      kind: meta?.kind ?? "f64",
      master: false,
      unreadable: null,
    });
  }
  node.children.forEach((child, i) => appendHierarchyNode(body, child, `${path}.${i}`, depth + 1));
  host.append(body);
}

// Which channel groups of one data group survive the filter, and which of
// their channels to draw — the desktop tree's matching rules: a group matches
// on its own name or any channel's name/unit, and a group whose *own* name
// matched shows all of its channels.
function structMatchingChannels(dg, q, filtering) {
  return dg.channel_groups.map((cg, j) => {
    if (!filtering) return { j, cg, channels: cg.channels };
    const nameMatch = cg.name.toLowerCase().includes(q);
    const visible = nameMatch ? cg.channels : cg.channels.filter((ch) => matchesStruct(ch, q));
    return { j, cg, channels: visible };
  });
}

function matchesStruct(ch, q) {
  return ch.name.toLowerCase().includes(q) || ch.unit.toLowerCase().includes(q);
}

function renderStructure() {
  structtreeEl.replaceChildren();
  if (!structureData) return;
  const d = structureData;
  const q = structFilterEl.value.trim().toLowerCase();
  const filtering = q.length > 0;

  const head = document.createElement("div");
  head.className = "filehead";
  head.textContent = `🗎 ${fileName}`;
  structtreeEl.append(head);
  const sub = document.createElement("div");
  sub.className = "filesub";
  sub.textContent =
    `MDF ${d.version}` +
    (d.block_count != null ? ` · ${d.block_count.toLocaleString()} blocks` : "");
  structtreeEl.append(sub);

  // The format's two fixed-address blocks — the tree's front door, as in the
  // desktop viewer. Static text: the browser viewer has no block inspector.
  for (const [label, type] of [
    ["Identification block", d.id_block],
    ["Header block", d.hd_block],
  ]) {
    if (type == null) continue;
    structtreeEl.append(staticRow(`▪ ${label} (${type})`));
  }

  appendSection(structtreeEl, "history", `🕒 File history (${d.history.length})`, structOpen("history", false), (host) => {
    for (const entry of d.history) {
      host.append(staticRow(entry.tool ? `${entry.time} — ${entry.tool}` : entry.time, "muted"));
    }
  });

  appendSection(structtreeEl, "attachments", `📎 Attachments (${d.attachments.length})`, structOpen("attachments", false), (host) => {
    for (const a of d.attachments) {
      const row = staticRow(`${a.name} (${a.embedded ? "embedded" : "external"})`);
      if (a.size) row.title = humanBytes(a.size);
      host.append(row);
    }
  });

  appendSection(structtreeEl, "events", `⚑ Events (${d.events.length})`, structOpen("events", false), (host) => {
    for (const ev of d.events) {
      const pos = ev.position === null || ev.position === undefined ? "—" : ev.position.toFixed(6);
      host.append(staticRow(`${ev.name || ev.type} @ ${pos}`));
    }
  });

  appendSection(structtreeEl, "hierarchy", `🗂 Channel hierarchy (${d.hierarchy.length})`, structOpen("hierarchy", false), (host) => {
    if (d.hierarchy.length === 0) {
      host.append(staticRow("This file declares no hierarchy.", "muted"));
      return;
    }
    d.hierarchy.forEach((node, i) => appendHierarchyNode(host, node, `${i}`, 0));
  });

  appendSection(structtreeEl, "dgs", `📁 Data groups (${d.data_groups.length})`, filtering || structOpen("dgs", true), (host) => {
    d.data_groups.forEach((dg, i) => {
      const matching = structMatchingChannels(dg, q, filtering);
      if (filtering && matching.every((m) => m.channels.length === 0)) return;
      const key = `dg:${i}`;
      const open = filtering || structOpen(key, d.data_groups.length <= 4);
      appendSection(
        host,
        key,
        `Data group ${i} — ${dg.channel_groups.length} group${dg.channel_groups.length === 1 ? "" : "s"}, ${dg.sorted ? "sorted" : "unsorted"}`,
        open,
        (body) => {
          if (dg.comment) body.append(staticRow(dg.comment, "muted"));
          for (const { j, cg, channels: chans } of matching) {
            appendChannelGroup(body, i, j, cg, chans, filtering);
          }
        }
      );
    });
  });
}

function appendChannelGroup(host, dgIndex, cgIndex, cg, chans, filtering) {
  const key = `cg:${dgIndex}:${cgIndex}`;
  const open = filtering || structOpen(key, false);
  const marker = cg.bus ? " 🚌" : cg.vlsd ? " ≡" : "";
  const row = document.createElement("div");
  row.className = "row clickable";
  const caret = document.createElement("span");
  caret.className = "caret";
  caret.textContent = open ? "▼" : "▶";
  const name = document.createElement("span");
  name.className = "sectionlabel";
  // The header states the group's real channel count even while a filter
  // narrows the rows under it — same as the desktop tree.
  name.textContent =
    (cg.name ? `Channel group ${cgIndex} — ${cg.name}` : `Channel group ${cgIndex}`) +
    marker +
    ` (${Number(cg.samples).toLocaleString()} samples, ${cg.channels.length} channel${cg.channels.length === 1 ? "" : "s"})`;
  const spacer = document.createElement("span");
  spacer.className = "fill";
  const plotBtn = document.createElement("button");
  plotBtn.className = "plotall";
  plotBtn.type = "button";
  plotBtn.textContent = "Plot all";
  plotBtn.title = "Plot readable channels in this group (up to 8)";
  plotBtn.addEventListener("click", (e) => {
    e.stopPropagation(); // the button is not a section toggle
    plotStructureGroup(dgIndex, cgIndex);
  });
  row.append(caret, name, spacer, plotBtn);
  row.addEventListener("click", () => structToggle(key));
  host.append(row);
  if (!open) return;

  const body = document.createElement("div");
  body.className = "children";
  // The comment is extra information next to the acquisition name in the
  // header; when the file just repeats the name, say it once.
  if (cg.comment && cg.comment.trim() !== cg.name.trim()) {
    body.append(staticRow(cg.comment, "muted"));
  }
  const cap = Math.min(chans.length, MAX_TREE_CHANNELS);
  for (let n = 0; n < cap; n++) appendChannelRow(body, chans[n]);
  if (chans.length > MAX_TREE_CHANNELS) {
    body.append(
      staticRow(
        `… and ${(chans.length - MAX_TREE_CHANNELS).toLocaleString()} more — use the Channels tab to search them`,
        "muted"
      )
    );
  }
  if (cg.reductions.length) {
    appendSection(
      body,
      `sr:${dgIndex}:${cgIndex}`,
      `Sample reduction (${cg.reductions.length})`,
      structOpen(`sr:${dgIndex}:${cgIndex}`, false),
      (sr) => {
        for (const r of cg.reductions) {
          const interval = r.interval === null || r.interval === undefined ? "—" : String(r.interval);
          sr.append(staticRow(`${r.cycles.toLocaleString()} cycles every ${interval} (${r.sync})`, "muted"));
        }
      }
    );
  }
  host.append(body);
}

// "Plot all" for one channel group: the readable, plottable, not-yet-plotted
// channels, up to the overlay cap — the desktop tree's rule, at this
// viewer's own 8-channel limit. Everything else counts as skipped.
function plotStructureGroup(dgIndex, cgIndex) {
  const cg = structureData?.data_groups[dgIndex]?.channel_groups[cgIndex];
  if (!cg) return;
  const already = shownNames();
  let added = 0;
  let skipped = 0;
  for (const ch of cg.channels) {
    const plottable = ch.kind === "f64" || ch.kind === "text" || ch.kind === "array";
    if (ch.master || ch.unreadable || !plottable || already.has(ch.name)) {
      skipped += 1;
      continue;
    }
    if (shown.length + added >= MAX_CHANNELS) {
      skipped += 1;
      continue;
    }
    already.add(ch.name);
    toggleChannel(ch.name);
    added += 1;
  }
  plotMsg(
    `Group ${cgIndex}: added ${added} channel${added === 1 ? "" : "s"} to plot, skipped ${skipped}`
  );
}

// Checkbox/dot refresh without a rebuild: the plotted set changed somewhere
// else (channel list, legend, keyboard) and the tree's rows follow.
function updateStructurePlotState() {
  if (!structureOpen || !structureData) return;
  const on = shownNames();
  for (const row of structtreeEl.querySelectorAll(".chrow[data-name]")) {
    const name = row.dataset.name;
    const isOn = on.has(name);
    row.classList.toggle("selected", isOn);
    const box = row.querySelector('input[type="checkbox"]');
    if (box) box.checked = isOn;
    let dot = row.querySelector(".dot");
    if (isOn && !dot) {
      dot = document.createElement("span");
      dot.className = "dot";
      const label = row.querySelector(".chlabel");
      row.insertBefore(dot, label);
    }
    if (dot) {
      if (isOn) dot.style.background = shown.find((s) => s.name === name)?.color ?? "";
      else dot.remove();
    }
  }
}

// The sidebar's two panes share one slot; switching to Structure fetches the
// outline the first time each file is shown (later opens reuse the snapshot).
function showSideTab(which) {
  const wantStructure = which === "structure";
  if (wantStructure === structureOpen) return;
  structureOpen = wantStructure;
  tabStructureBtn.classList.toggle("active", structureOpen);
  tabStructureBtn.setAttribute("aria-selected", String(structureOpen));
  tabChannelsBtn.classList.toggle("active", !structureOpen);
  tabChannelsBtn.setAttribute("aria-selected", String(!structureOpen));
  channellistEl.hidden = structureOpen;
  structureEl.hidden = !structureOpen;
  if (structureOpen) {
    if (structureData) renderStructure();
    else requestStructure();
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
    tableBtn.disabled = !selected;
    tableBtn.title = selected ? `sample table of ${selected}` : "select a channel first";
    detailsBtn.disabled = !selected;
    detailsBtn.title = selected ? `metadata of ${selected}` : "select a channel first";
    xyBtn.disabled = true;
    return;
  }
  csvBtn.disabled = false;
  csvBtn.textContent = selected ? `Download CSV · ${selected}` : "Download CSV";
  csvBtn.title = selected ? `visible window of ${selected}` : "";
  tableBtn.disabled = !selected;
  tableBtn.title = selected ? `sample table of ${selected}` : "select a channel first";
  detailsBtn.disabled = !selected;
  detailsBtn.title = selected ? `metadata of ${selected}` : "select a channel first";
  xyBtn.disabled = shown.length < 2;
  xyBtn.title = shown.length < 2 ? "plot two channels first" : "value-vs-value plot of two plotted channels";
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
    label.textContent = s.kind === "array" ? `${s.name}[${s.element}]` : s.name;
    row.append(label);
    if (s.kind === "array" && s.elements > 1) {
      // The element stepper: an array channel plots one element; ◂/▸ walk
      // the selectable range. Re-requesting goes through the same epoch as
      // any view change, so a stale reply cannot overwrite the new element.
      const prev = document.createElement("span");
      prev.className = "rm";
      prev.textContent = "◂";
      prev.title = "previous element";
      prev.addEventListener("click", (e) => {
        e.stopPropagation();
        cycleElement(s, -1);
      });
      row.append(prev);
      const next = document.createElement("span");
      next.className = "rm";
      next.textContent = "▸";
      next.title = "next element";
      next.addEventListener("click", (e) => {
        e.stopPropagation();
        cycleElement(s, 1);
      });
      row.append(next);
    }
    if (s.unit) {
      const unit = document.createElement("span");
      unit.className = "leg-unit";
      unit.textContent = `[${s.unit}]`;
      row.append(unit);
    }
    if (s.kind === "text" && s.vocab) {
      const states = document.createElement("span");
      states.className = "leg-range";
      states.textContent = `${s.vocab.length} state${s.vocab.length === 1 ? "" : "s"}`;
      row.append(states);
    } else if (s.min !== null) {
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
      refreshStats(); // the strip follows the selection
      refreshTable(); // so does the table
      refreshDetails();
    });
    legendEl.append(row);
  }
}

// Plots the neighbouring element of an array channel (plan 2.4): the count
// comes from the series reply (the largest real sample for a dynamic shape),
// and the view wraps so ▸ from the last element lands back on 0.
function cycleElement(entry, dir) {
  const n = entry.elements ?? 0;
  if (!n) return;
  entry.element = ((entry.element ?? 0) + dir + n) % n;
  renderLegend();
  if (view) requestAll(); // one channel changed; re-use the same epoch path
  refreshTable();
}

function renderReadout() {
  // Live values while the pointer is on the plot; otherwise the pinned
  // cursor-A snapshot (a click is "read here, and stay") keeps its values up.
  const pin = !cursor && cursorA && cursorA.values ? cursorA : null;
  if ((!cursor && !pin) || shown.length === 0) {
    readoutEl.hidden = true;
    return;
  }
  readoutEl.hidden = false;
  readoutEl.replaceChildren();
  const head = document.createElement("div");
  head.className = "ro-t";
  head.textContent = `${pin ? "A · " : ""}t = ${fmtNumber(pin ? pin.t : cursor.t)} s`;
  readoutEl.append(head);
  if (absoluteTime && startEpochMs !== null) {
    // The numeric readouts stay in seconds (they feed arithmetic and match
    // the CSV); in absolute mode the cursors' wall clock rides along. With
    // neither cursor placed, the plain hover line gets it instead.
    const marks = [
      cursorA ? ["A", cursorA.t] : null,
      cursorB ? ["B", cursorB.t] : null,
    ].filter(Boolean);
    for (const [tag, t] of marks.length ? marks : [[null, pin ? pin.t : cursor.t]]) {
      const row = document.createElement("div");
      row.className = "ro-abs";
      row.textContent = tag ? `${tag} · ${fmtAbsFull(t)}` : fmtAbsFull(t);
      readoutEl.append(row);
    }
  }
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
    const at = pin ? (pin.values[s.name] ?? null) : s.at;
    // A text channel's readout is the label itself, not a number.
    val.textContent = at === null || at === undefined ? "—" : typeof at === "string" ? at : fmtNumber(at);
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

// The stats strip under the toolbar. Two modes, one Rust endpoint: the
// region between cursors A and B (per shown channel: min/max/mean/Δvalue,
// Δt once in the header), or the selected channel over the visible window
// (samples/min/max/mean/invalid). Figures come only from worker replies —
// nothing is computed per pixel here.
// Renders a label distribution ({labels:[{label, samples, seconds}, …]}) as
// "label Xs · label Ys" parts; the numeric strip parts are handled by the
// callers. Only the top labels are named — the vocabulary is short (states),
// but a hostile file could carry more than fits a strip.
function labelParts(st, max = 3) {
  const entries = st.labels.slice(0, max).map((e) => {
    const secs = Number.isFinite(e.seconds) && e.seconds > 0 ? ` ${fmtNumber(e.seconds)}s` : "";
    return `${e.label}${secs}`;
  });
  if (st.labels.length > max) entries.push(`+${st.labels.length - max} more`);
  return entries;
}

function renderStatsbar() {
  if (shown.length === 0 || (!regionStats && !viewStats)) {
    statsbarEl.hidden = true;
    return;
  }
  statsbarEl.hidden = false;
  statsbarEl.replaceChildren();

  if (regionStats && cursorA && cursorB) {
    const head = document.createElement("span");
    head.className = "st-head";
    head.textContent =
      `region ${fmtNumber(regionStats.t0)} → ${fmtNumber(regionStats.t1)} s` +
      ` · Δt ${fmtNumber(regionStats.t1 - regionStats.t0)} s`;
    statsbarEl.append(head);
    for (const s of shown) {
      const st = regionStats.byName.get(s.name);
      const seg = document.createElement("span");
      seg.className = "st-ch";
      seg.title = s.name;
      const dot = document.createElement("span");
      dot.className = "dot";
      dot.style.background = s.color;
      seg.append(dot);
      const name = document.createElement("span");
      name.className = "st-name";
      name.textContent = s.name;
      seg.append(name);
      // A text channel's region figures are its label distribution — min/
      // max/mean of words is not a thing, but how long each state held is.
      // Array/byte channels were never requested: say so instead of "…".
      const parts = !st
        ? s.kind === "array" || s.kind === "bytes"
          ? ["no scalar stats"]
          : ["…"]
        : st.labels
          ? [
              `${(st.count + st.invalid).toLocaleString()} samples ·`,
              labelParts(st).join(" · "),
            ]
          : [
              `min ${fmtNumber(st.min)}`,
              `max ${fmtNumber(st.max)}`,
              `mean ${fmtNumber(st.mean)}`,
              `Δ ${
                st.first !== null && st.last !== null
                  ? fmtNumber(st.last - st.first)
                  : "—"
              }`,
            ];
      const nums = document.createElement("span");
      nums.textContent = parts.join(" · ");
      seg.append(nums);
      statsbarEl.append(seg);
    }
    return;
  }

  if (viewStats) {
    const entry = shown.find((s) => s.name === viewStats.name);
    const st = viewStats.data;
    const seg = document.createElement("span");
    seg.className = "st-ch";
    if (entry) {
      const dot = document.createElement("span");
      dot.className = "dot";
      dot.style.background = entry.color;
      seg.append(dot);
    }
    const name = document.createElement("span");
    name.className = "st-name";
    name.textContent = viewStats.name;
    seg.append(name);
    if (entry?.unit) {
      const unit = document.createElement("span");
      unit.textContent = `[${entry.unit}]`;
      seg.append(unit);
    }
    // samples = count + invalid: the whole recording in the window, the way
    // the old single-channel demo counted, with the invalid share said
    // separately — only when there is one. A text channel trades min/max/
    // mean for its label distribution.
    const parts = !st
      ? ["…"]
      : st.labels
        ? [
            "visible window:",
            `${(st.count + st.invalid).toLocaleString()} samples ·`,
            labelParts(st).join(" · "),
          ]
        : [
            "visible window:",
            `${(st.count + st.invalid).toLocaleString()} samples`,
            `min ${fmtNumber(st.min)}`,
            `max ${fmtNumber(st.max)}`,
            `mean ${fmtNumber(st.mean)}`,
          ];
    if (st && !st.labels && st.invalid > 0) parts.push(`${st.invalid.toLocaleString()} invalid`);
    const nums = document.createElement("span");
    nums.textContent = parts.join(" · ");
    seg.append(nums);
    statsbarEl.append(seg);
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

// ---------------------------------------------------------------------------
// Absolute X labels: local wall clock (start_time is UTC in the file; Date
// and Intl render it in the browser's timezone, like every other timestamp
// the browser shows — that is what "wall clock" means to the reader).
//
// Resolution follows the tick step, so a label never shows digits that
// cannot differ between neighboring ticks. Time ladders always carry
// seconds: the tick steps are decimal (100 s, 1000 s, …) while clock
// minutes are sexagesimal, so a HH:mm label would round a tick's time by up
// to 59 s. Day-and-coarser steps truncate to the date, which is exact.
const absDayFmt = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });
const absDateFmt = new Intl.DateTimeFormat(undefined, {
  year: "numeric",
  month: "short",
  day: "numeric",
});
const pad2 = (n) => String(n).padStart(2, "0");

function fmtAbsTick(t, step) {
  const ms = startEpochMs + t * 1000;
  const d = new Date(ms);
  if (step >= 180 * 86400) return absDateFmt.format(d);
  if (step >= 86400) return absDayFmt.format(d);
  const hms = `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
  if (step >= 1) return hms;
  if (step >= 0.001) return `${hms}.${String(d.getMilliseconds()).padStart(3, "0")}`;
  // Date holds only milliseconds; the microseconds come from the arithmetic.
  const us = ((Math.round(ms * 1000) % 1e6) + 1e6) % 1e6;
  return `${hms}.${String(us).padStart(6, "0")}`;
}

// Full-precision timestamp for the readout: fixed numeric layout, not a
// locale datetime — a measurement readout wants a stable, greppable string
// beside the seconds.
function fmtAbsFull(t) {
  const d = new Date(startEpochMs + t * 1000);
  return (
    `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ` +
    `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}` +
    `.${String(d.getMilliseconds()).padStart(3, "0")}`
  );
}

// ---------------------------------------------------------------------------
// State bands: a text channel's drawing primitive. Each run of constant label
// becomes one filled rectangle, its row picked by the label's index in the
// channel's stable vocabulary — a Gantt-style ladder where height carries no
// numeric meaning, only which state was active when. A null label (invalid
// sample) paints nothing: the gap rule the numeric path keeps for NaN.
//
// The run data arrives run-collapsed from Rust (first sample, every change,
// the window's last sample), so the loop is over state changes, not samples.

function bandFill(v) {
  // Deterministic hue per row: the same label is the same color in every
  // lane and every zoom, and no vocabulary slot is confined to the line
  // palette (bands and lines must never be confusable).
  return `hsl(${(v * 67 + 203) % 360} 45% 42% / 0.5)`;
}

function drawBands(ctx, rect, entry, X, plotRight) {
  const vocab = entry.vocab ?? [];
  if (vocab.length === 0 || !entry.ts || !entry.labels) return;
  const rowH = rect.height / vocab.length;
  const rowIndex = new Map(vocab.map((l, i) => [l, i]));
  const n = entry.ts.length;
  for (let i = 0; i < n; i++) {
    const label = entry.labels[i];
    if (label === null) continue; // invalid sample: a gap between bands
    const x0 = Math.max(X(entry.ts[i]), rect.left);
    // The final run holds until the plot's right edge: a state's last sample
    // has no recorded end, and stopping at the sample would draw it as a
    // flicker instead of the state it is.
    const x1 = i + 1 < n ? Math.min(X(entry.ts[i + 1]), plotRight) : plotRight;
    if (x1 - x0 < 0.25) continue;
    const v = rowIndex.get(label);
    if (v === undefined) continue; // vocabulary not yet known (stale reply)
    const y0 = rect.top + v * rowH;
    ctx.fillStyle = bandFill(v);
    ctx.fillRect(x0, y0, x1 - x0, rowH);
    // Name the state only when the band is big enough to read it — the
    // readout and the stats strip carry the full mapping either way.
    if (x1 - x0 > 36 && rowH >= 11) {
      ctx.fillStyle = "#f2f4f8";
      ctx.textAlign = "left";
      ctx.textBaseline = "middle";
      ctx.fillText(label, x0 + 4, y0 + rowH / 2);
    }
  }
}

let canvasSize = [0, 0];

function draw() {
  if (gpsOpen) drawGps();
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
  ctx.font = "11px ui-monospace, Menlo, Consolas, monospace";

  // Size the plot area to the lane count in stacked mode (why: MIN_LANE_H).
  // Doing this here rather than on toggle keeps the canvas honest through
  // channel add/remove too; the ResizeObserver re-fires draw at the new
  // height, and clearing the inline style restores the CSS minimum. The
  // 320 px floor matches #plotwrap's CSS min-height, so a short stack
  // never lowers the page's own minimum.
  const m0 = margins();
  const laneH = stacked && shown.length > 0 ? m0.t + m0.b + shown.length * MIN_LANE_H : 0;
  const wantMinH = laneH > 320 ? `${laneH}px` : "";
  if (plotwrap.style.minHeight !== wantMinH) plotwrap.style.minHeight = wantMinH;

  if (shown.length === 0 || !view) {
    ctx.fillStyle = "#8b93a7";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(
      shown.length === 0 ? "No channels overlaid" : "Decoding…",
      w / 2,
      h / 2
    );
    // An empty plot is an invitation, not a dead end: say where channels live.
    if (shown.length === 0) {
      ctx.font = "12px ui-monospace, Menlo, Consolas, monospace";
      ctx.fillText(
        "pick one from the channel list, or open the Structure tab",
        w / 2,
        h / 2 + 22
      );
    }
    return;
  }

  const m = margins();
  const pw = w - m.l - m.r;
  const ph = h - m.t - m.b;
  const t0 = view.t0;
  const t1 = view.t1;
  const tSpan = t1 - t0 || 1;
  const X = (t) => m.l + ((t - t0) / tSpan) * pw;

  // Lanes: overlay draws every channel across the whole plot area, stacked
  // gives each channel its own lane with its own Y scale — the native GUI's
  // default, and the honest layout when the units don't compare.
  const lanes = [];
  if (stacked) {
    const lh = ph / shown.length;
    for (let i = 0; i < shown.length; i++) {
      lanes.push({ top: m.t + i * lh, height: lh, entry: shown[i] });
    }
  } else {
    lanes.push({ top: m.t, height: ph, entry: null });
  }

  // Shared Y scales every channel against the union of the visible data
  // (overlay pane only — the checkbox is disabled in stacked mode, but not
  // unchecked, so the choice survives a mode switch). Without it each
  // channel is scaled to its own range, which is what keeps a temperature
  // trace readable next to an RPM trace.
  const shared = sharedYEl.checked && !stacked;
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
  // Padded [lo, hi] of one channel's visible values; flat (or still
  // undecoded) channels get a unit span so a line stays drawable.
  const scaleOf = (entry) => {
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
    return [lo, hi];
  };
  const Y = (lane, entry, v) => {
    const [lo, hi] = shared ? [gy0, gy1] : scaleOf(entry);
    return lane.top + (1 - (v - lo) / (hi - lo)) * lane.height;
  };

  // Grid: the X (time) axis is shared — ticks run through every lane and
  // are labeled once, under the bottom one. Y ticks are drawn per lane with
  // a real scale (the shared-Y pane and each stacked lane); the per-channel
  // overlay pane draws unlabeled lines only, since each line there has its
  // own scale and Y labels would lie.
  ctx.strokeStyle = "#242a38";
  ctx.fillStyle = "#8b93a7";
  ctx.lineWidth = 1;
  const step = niceStep(tSpan / 6);
  const labelY = h - m.b + 6;
  // The corner tag says what the numbers are: seconds in relative mode; in
  // absolute mode the date of the view start plus "local", since the per-tick
  // clock labels carry no date until the step reaches a day or the window
  // crosses midnight.
  const absTicks = absoluteTime && startEpochMs !== null;
  const corner = absTicks ? `${absDateFmt.format(new Date(startEpochMs + t0 * 1000))} · local` : "t [s]";
  ctx.textAlign = "left";
  ctx.textBaseline = "top";
  const cornerX = w - m.r - ctx.measureText(corner).width;
  ctx.fillText(corner, cornerX, labelY);
  // Labels are measured and thinned: absolute labels ("14:26:40.123", dates
  // at midnight crossings) are wider than the relative ones, so a label that
  // would touch the previous one — or run into the corner tag — is skipped
  // rather than overlapped. Narrow screens and sub-second windows both crowd
  // the axis; the grid lines stay, only the text thins out.
  let lastLabelEnd = -Infinity;
  const cornerStart = cornerX - 10;
  let prevDay = absTicks && step < 86400
    ? new Date(startEpochMs + (Math.ceil(t0 / step) * step - step) * 1000).toDateString()
    : null;
  for (let t = Math.ceil(t0 / step) * step; t <= t1; t += step) {
    const x = X(t);
    ctx.beginPath();
    ctx.moveTo(x, m.t);
    ctx.lineTo(x, h - m.b);
    ctx.stroke();
    let label = absTicks ? fmtAbsTick(t, step) : fmtNumber(t);
    if (absTicks && step < 86400) {
      // A clock label whose day differs from the previous tick gets the date
      // inline ("Dec 20 00:00"); otherwise a window over midnight would read
      // as an endless, ambiguous sequence of clock times.
      const day = new Date(startEpochMs + t * 1000).toDateString();
      if (day !== prevDay) label = `${absDayFmt.format(new Date(startEpochMs + t * 1000))} ${label}`;
      prevDay = day;
    }
    const tw = ctx.measureText(label).width;
    let lx = Math.max(x - tw / 2, m.l + 2);
    if (lx + tw > cornerStart || lx < lastLabelEnd + 8) continue;
    lastLabelEnd = lx + tw;
    ctx.fillText(label, lx, labelY);
  }

  // Labeled Y ticks for one lane's [lo, hi]. Short lanes drop to two
  // divisions — five labels in a MIN_LANE_H-tall lane overlap into noise.
  const yTicks = (lane, lo, hi) => {
    const ystep = niceStep((hi - lo) / (lane.height >= 96 ? 5 : 2));
    for (let v = Math.ceil(lo / ystep) * ystep; v <= hi; v += ystep) {
      const y = lane.top + (1 - (v - lo) / (hi - lo)) * lane.height;
      ctx.beginPath();
      ctx.moveTo(m.l, y);
      ctx.lineTo(w - m.r, y);
      ctx.stroke();
      ctx.textAlign = "right";
      ctx.textBaseline = "middle";
      ctx.fillText(fmtNumber(v), m.l - 6, y);
    }
  };

  for (const lane of lanes) {
    if (shared || lane.entry) {
      const [lo, hi] = shared ? [gy0, gy1] : scaleOf(lane.entry);
      // A text lane has no numeric scale to tick — its rows are states, not
      // values, and numeric labels there would be pure fiction.
      if (!(lane.entry && lane.entry.kind === "text")) yTicks(lane, lo, hi);
      if (lane.entry) {
        // Lane identity: name + unit in the channel's color. Inside the
        // lane rather than beside it — the 58 px left margin fits tick
        // numbers, not channel names.
        ctx.textAlign = "left";
        ctx.textBaseline = "top";
        ctx.fillStyle = lane.entry.color;
        ctx.fillText(
          lane.entry.unit
            ? `${lane.entry.name} [${lane.entry.unit}]`
            : lane.entry.name,
          m.l + 6,
          lane.top + 3
        );
        ctx.fillStyle = "#8b93a7";
      }
    } else {
      for (let g = 0; g <= 5; g++) {
        const y = lane.top + (lane.height * g) / 5;
        ctx.beginPath();
        ctx.moveTo(m.l, y);
        ctx.lineTo(w - m.r, y);
        ctx.stroke();
      }
    }
  }

  // Text channels. In stacked mode each draws across its own lane; in
  // overlay they share the pane with numeric lines, so each gets a strip
  // cut from the pane's bottom (big enough for its vocabulary, capped so
  // text channels cannot eat the whole plot).
  const plotRight = w - m.r;
  const plotBottom = h - m.b;
  const overlayText = stacked ? [] : shown.filter((s) => s.kind === "text");
  let stripBottom = plotBottom;
  for (const s of overlayText) {
    // +14: the identity label gets its own line above the bands (inside the
    // strip) so it can never overlap the first row's caption.
    const stripH = Math.min(ph * 0.4, 24 + s.vocab.length * 18);
    ctx.textAlign = "left";
    ctx.textBaseline = "top";
    ctx.fillStyle = s.color;
    ctx.fillText(s.unit ? `${s.name} [${s.unit}]` : s.name, m.l + 6, stripBottom - stripH + 2);
    ctx.fillStyle = "#8b93a7";
    drawBands(
      ctx,
      { top: stripBottom - stripH + 14, height: stripH - 14, left: m.l },
      s,
      X,
      plotRight
    );
    stripBottom -= stripH + 6;
  }

  // Lines. NaN (or ±inf) values arrive as gaps from the Rust decimation and
  // break the path here, so an invalid stretch reads as a hole, not a bridge.
  ctx.lineJoin = "round";
  ctx.lineWidth = 1.5;
  for (const lane of lanes) {
    // Stacked lanes draw their one channel; the overlay lane draws the
    // numeric ones (text channels drew their bands above).
    const entries = lane.entry ? [lane.entry] : shown;
    for (const s of entries) {
      if (!s.ts) continue;
      if (s.kind === "text") {
        // Stacked: the bands span the lane. (Overlay text channels drew
        // their bottom strips above; there lane.entry is null.)
        if (lane.entry) {
          drawBands(ctx, { top: lane.top, height: lane.height, left: m.l }, s, X, plotRight);
        }
        continue;
      }
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
        const py = Y(lane, s, v);
        if (pen) ctx.lineTo(px, py);
        else {
          ctx.moveTo(px, py);
          pen = true;
        }
      }
      ctx.stroke();
    }
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

  // Measurement cursors A (click) and B (shift-click) with the region
  // between them shaded. The blue/orange pair follows the native GUI's
  // cursor colors. Cursors are times, not pixels: they stay put through
  // zoom and pan, and a cursor outside the view simply isn't drawn.
  if (cursorA && cursorB) {
    const xa = Math.max(m.l, Math.min(X(Math.min(cursorA.t, cursorB.t)), w - m.r));
    const xb = Math.max(m.l, Math.min(X(Math.max(cursorA.t, cursorB.t)), w - m.r));
    if (xb > xa) {
      ctx.fillStyle = "rgba(79, 140, 255, 0.08)";
      ctx.fillRect(xa, m.t, xb - xa, ph);
    }
  }
  const drawCursorLine = (c, label, color) => {
    if (!c || c.t < t0 || c.t > t1) return;
    const x = X(c.t);
    ctx.strokeStyle = color;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x, m.t);
    ctx.lineTo(x, h - m.b);
    ctx.stroke();
    ctx.fillStyle = color;
    ctx.textAlign = "left";
    ctx.textBaseline = "top";
    ctx.fillText(label, x + 3, m.t + 2);
  };
  drawCursorLine(cursorA, "A", "#4f8cff");
  drawCursorLine(cursorB, "B", "#ffb454");

  // Shift-drag rubber band, clamped to the plot area: the zoom maps the
  // selection through tAtX (which clamps the same way), so the drawn band
  // and the window actually applied agree.
  if (selRect) {
    const x0 = Math.max(m.l, Math.min(selRect.x0, selRect.x1));
    const x1 = Math.min(w - m.r, Math.max(selRect.x0, selRect.x1));
    const y0 = Math.max(m.t, Math.min(selRect.y0, selRect.y1));
    const y1 = Math.min(h - m.b, Math.max(selRect.y0, selRect.y1));
    if (x1 > x0 && y1 > y0) {
      ctx.fillStyle = "rgba(79, 140, 255, 0.15)";
      ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
      ctx.strokeStyle = "#4f8cff";
      ctx.lineWidth = 1;
      ctx.strokeRect(x0 + 0.5, y0 + 0.5, x1 - x0 - 1, y1 - y0 - 1);
    }
  }
}

// ---------------------------------------------------------------------------
// Plot interactions

// Time at plot x for a given view. Split so the pinch can map its drifting
// midpoint through the view frozen at gesture start.
function tInView(x, v) {
  const m = margins();
  const pw = plotwrap.clientWidth - m.l - m.r;
  const frac = Math.min(1, Math.max(0, (x - m.l) / pw));
  return v.t0 + frac * (v.t1 - v.t0);
}

function tAtX(x) {
  return tInView(x, view);
}

// ---------------------------------------------------------------------------
// View mutations. Every navigation input — wheel, drag, keyboard, pinch —
// lands in one of these three, so clamping and the re-decode debounce behave
// identically wherever the gesture came from.

function panBy(frac) {
  const dt = frac * (view.t1 - view.t0); // frac of the visible window
  view = { t0: view.t0 + dt, t1: view.t1 + dt };
  clampView();
  requestAnimationFrame(draw);
  scheduleRequestAll();
}

function zoomAtT(focus, factor) {
  // factor < 1 zooms in; the time at `focus` stays put.
  view = {
    t0: focus + (view.t0 - focus) * factor,
    t1: focus + (view.t1 - focus) * factor,
  };
  clampView();
  requestAnimationFrame(draw);
  scheduleRequestAll();
}

function resetView() {
  const ext = globalExtent();
  if (!ext) return;
  view = ext[0] < ext[1] ? { t0: ext[0], t1: ext[1] } : { t0: ext[0] - 0.5, t1: ext[1] + 0.5 };
  clampView();
  requestAnimationFrame(draw);
  requestAll(); // not debounced: the full-range data is wanted now, as on double-click
}

let dragging = null;

// Applies a shift-drag selection as the new view. tAtX clamps to the plot
// area, so a band dragged wholly inside a margin collapses to a zero time
// span and is ignored like any other degenerate selection.
function zoomToSelection(sel) {
  const a = tAtX(Math.min(sel.x0, sel.x1));
  const b = tAtX(Math.max(sel.x0, sel.x1));
  if (Math.abs(sel.x1 - sel.x0) < MIN_SELECT_PX || b <= a) {
    requestAnimationFrame(draw); // clear the rubber band
    return;
  }
  view = { t0: a, t1: b };
  clampView();
  requestAnimationFrame(draw);
  scheduleRequestAll();
}

plot.addEventListener("wheel", (e) => {
  if (!view) return;
  e.preventDefault();
  const rect = plot.getBoundingClientRect();
  const factor = Math.exp(Math.max(-120, Math.min(120, e.deltaY)) * 0.0015);
  zoomAtT(tAtX(e.clientX - rect.left), factor);
}, { passive: false });

// Touch pointers: the first finger keeps the mouse gestures (tap places a
// cursor, drag pans). A second finger starts a pinch and drops whatever
// gesture was in flight without committing it, so the view never jumps;
// pointers beyond the first two are ignored. A gesture set that was ever
// two-finger never places cursors — its taps are pinch debris.
const activePointers = new Map(); // pointerId -> {x, y} in client coordinates
let pinch = null; // {ids, base, d0} while a pinch pair is down
let pinchTainted = false; // set while any pointer of a two-finger set is down

// The first two pointers in press order (Map iteration order), stable even
// when a third finger is down.
function pinchPair() {
  const ids = [...activePointers.keys()];
  return ids.length >= 2 ? [ids[0], ids[1]] : null;
}

// (Re)anchor the pinch on the current first-two pointers. Re-anchoring when
// the pair changes (a finger lifted while a third is still down) rebaselines
// distance and view at the same instant, so the swap causes no jump.
function rebasePinch() {
  const pair = pinchPair();
  if (!pair) {
    pinch = null;
    return;
  }
  if (pinch && pinch.ids[0] === pair[0] && pinch.ids[1] === pair[1]) return;
  const [a, b] = pair.map((id) => activePointers.get(id));
  pinch = {
    ids: pair,
    base: { ...view }, // frozen: every move maps from here, no feedback loop
    d0: Math.max(1, Math.hypot(b.x - a.x, b.y - a.y)),
  };
}

function updatePinch(rect) {
  // view can vanish under a live pinch (last channel removed, file closed):
  // go inert rather than resurrect a stale window from the frozen base.
  if (!view) return;
  const pts = pinch.ids.map((id) => activePointers.get(id));
  if (pts.some((p) => !p)) return;
  const d = Math.hypot(pts[1].x - pts[0].x, pts[1].y - pts[0].y);
  if (d < 1) return; // fingers on the same spot: the ratio is noise, wait
  // X-only zoom around the pinch midpoint, which drifting fingers also
  // translate: the midpoint maps through the frozen base view, and the
  // distance ratio scales the span around that time.
  const focus = tInView((pts[0].x + pts[1].x) / 2 - rect.left, pinch.base);
  const k = pinch.d0 / d;
  view = {
    t0: focus + (pinch.base.t0 - focus) * k,
    t1: focus + (pinch.base.t1 - focus) * k,
  };
  clampView();
  requestAnimationFrame(draw);
  scheduleRequestAll();
}

// Hand the surviving finger back to plain panning after its partner lifts.
function continuePanWithRemaining() {
  if (!view) {
    dragging = null; // nothing to pan (see the guard in updatePinch)
    return;
  }
  const p = activePointers.values().next().value;
  const rect = plot.getBoundingClientRect();
  dragging = {
    x: p.x,
    sx: p.x - rect.left,
    sy: p.y - rect.top,
    view: { ...view },
    moved: true, // shape parity; the taint flag is what suppresses the click
    zoom: false,
  };
}

plot.addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || !view) return;
  plot.setPointerCapture(e.pointerId);
  activePointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
  if (activePointers.size >= 2) {
    // Second (or later) finger: whatever gesture was in flight is dropped,
    // never committed — a rubber band half-drawn or a pan half-applied must
    // not land as a jump. rebasePinch is a no-op for a finger joining the
    // already-tracked pair.
    dragging = null;
    if (selRect) {
      selRect = null;
      requestAnimationFrame(draw);
    }
    pinchTainted = true;
    rebasePinch();
    return;
  }
  const rect = plot.getBoundingClientRect();
  // Shift at press time decides the gesture: shift-drag draws a region to
  // zoom into, plain drag (and any touch — Shift never reads as held
  // there) pans. Locked at pointerdown so a mid-drag Shift press cannot
  // switch semantics under the pointer.
  dragging = {
    x: e.clientX,
    sx: e.clientX - rect.left,
    sy: e.clientY - rect.top,
    view: { ...view },
    moved: false,
    zoom: e.shiftKey,
  };
});

plot.addEventListener("pointermove", (e) => {
  const rect = plot.getBoundingClientRect();
  if (activePointers.has(e.pointerId)) {
    activePointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
  }
  if (pinch) {
    updatePinch(rect);
    return;
  }
  if (dragging) {
    dragging.moved = true;
    if (dragging.zoom) {
      // Rubber band only: no pan and no re-decode while dragging — the
      // window is applied once, on pointerup.
      selRect = {
        x0: dragging.sx,
        y0: dragging.sy,
        x1: e.clientX - rect.left,
        y1: e.clientY - rect.top,
      };
      requestAnimationFrame(draw);
      return;
    }
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

// Click places cursor A, shift-click cursor B; once both are placed the next
// click clears the pair and starts over — the same cycle oscilloscope cursor
// pairs use, so a region can never grow a third edge. Esc clears at any time.
function placeCursor(t, forB) {
  if (cursorA && cursorB) {
    cursorA = null;
    cursorB = null;
  }
  if (forB) {
    cursorB = { t };
  } else {
    cursorA = { t, values: null };
    sendSampleQuery(t); // pin the readout at the clicked time
    // Frame link, plot → panel: scroll the frame list to this time.
    if (busOpen && busGroups.length > 0) {
      const g = busGroups[busGroupIndex] ?? busGroups[0];
      worker.postMessage({ type: "bus-locate", kind: g.kind, group: g.group, t });
    }
  }
  refreshStats();
  requestAnimationFrame(draw);
}

function clearCursors() {
  cursorA = null;
  cursorB = null;
  refreshStats();
  renderReadout();
  requestAnimationFrame(draw);
}

plot.addEventListener("pointerup", (e) => {
  activePointers.delete(e.pointerId);
  if (activePointers.size > 0) {
    // A finger left a pinch while touch remains: two left re-anchor on the
    // new pair, exactly one continues as a plain pan by the surviving finger.
    rebasePinch();
    if (!pinch && activePointers.size === 1) continuePanWithRemaining();
    return;
  }
  // Last pointer up. The taint must outlive the decision below — a lift
  // that was ever part of a pinch is never a cursor placement.
  const tainted = pinchTainted;
  pinchTainted = false;
  pinch = null;
  if (!dragging) return;
  const rect = plot.getBoundingClientRect();
  // A press that never really moved is a cursor placement, not a gesture:
  // the same threshold that keeps a tiny shift-drag from zooming keeps a
  // wiggle from placing a cursor.
  const dist = Math.hypot(
    e.clientX - rect.left - dragging.sx,
    e.clientY - rect.top - dragging.sy
  );
  const wasZoom = dragging.zoom;
  const sel = wasZoom ? selRect : null;
  dragging = null;
  selRect = null;
  try {
    plot.releasePointerCapture(e.pointerId);
  } catch {
    // capture already released (pointercancel handled it)
  }
  if (dist < MIN_SELECT_PX) {
    if (!tainted) placeCursor(tAtX(e.clientX - rect.left), wasZoom);
    return;
  }
  if (sel) zoomToSelection(sel);
});

plot.addEventListener("pointercancel", (e) => {
  activePointers.delete(e.pointerId);
  if (activePointers.size >= 2) {
    rebasePinch(); // the next two carry the pinch on
    dragging = null; // a pinch runs on pinch state, not drag state
  } else if (activePointers.size === 1) {
    pinch = null;
    continuePanWithRemaining();
  } else {
    pinch = null;
    pinchTainted = false; // nothing is down: the gesture set is over
    dragging = null;
  }
  if (selRect) {
    selRect = null;
    requestAnimationFrame(draw);
  }
});

plot.addEventListener("pointerleave", () => {
  if (dragging) return;
  cursor = null;
  renderReadout(); // falls back to the pinned A readout, or hides
  requestAnimationFrame(draw);
});

// Esc stays document-level so the cursors clear from anywhere, cursor pair
// or single; it listens and never swallows. Every other navigation key lives
// on the canvas itself (below).
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || (!cursorA && !cursorB)) return;
  clearCursors();
});

plot.addEventListener("dblclick", resetView);

// Keyboard navigation, scoped to the canvas: the listener only fires when
// the plot itself has focus (tabindex=0 — a click gives it focus, Tab
// reaches it too), so the channel filter and every other input never see
// these keys. preventDefault happens only on handled keys, so Tab and
// browser shortcuts pass through untouched.
const KEY_PAN_FRAC = 0.1; // ~10% of the visible window per arrow press
const KEY_ZOOM = 1.25;

plot.addEventListener("keydown", (e) => {
  if (e.ctrlKey || e.metaKey || e.altKey) return; // never touch shortcuts
  if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
    if (!view) return;
    e.preventDefault(); // the arrows would otherwise scroll the page
    panBy(e.key === "ArrowLeft" ? -KEY_PAN_FRAC : KEY_PAN_FRAC);
    return;
  }
  if (e.key === "+" || e.key === "=") {
    // "=" covers the layouts where "+" needs Shift.
    if (!view) return;
    e.preventDefault();
    zoomAtT((view.t0 + view.t1) / 2, 1 / KEY_ZOOM);
    return;
  }
  if (e.key === "-" || e.key === "_") {
    if (!view) return;
    e.preventDefault();
    zoomAtT((view.t0 + view.t1) / 2, KEY_ZOOM);
    return;
  }
  if (e.key === "Home" || e.key === "0") {
    if (!view) return;
    e.preventDefault();
    resetView();
    return;
  }
  // 1–8 toggle plotted slot N: the same toggleChannel a channel-list click
  // runs, i.e. the Nth entry of `shown` (toggling an already-plotted channel
  // removes it — that is what a list click on it does too). Digits past the
  // plotted count have no slot to act on and are ignored.
  if (/^[1-8]$/.test(e.key)) {
    const entry = shown[Number(e.key) - 1];
    if (entry) {
      e.preventDefault();
      toggleChannel(entry.name);
    }
  }
});

csvBtn.addEventListener("click", () => {
  if (!selected || !view) return;
  plotMsg("");
  worker.postMessage({ type: "csv", name: selected, t0: view.t0, t1: view.t1 });
});

tableBtn.addEventListener("click", () => toggleTable());
dbcBtn.addEventListener("click", () => dbcInput.click());
busBtn.addEventListener("click", () => toggleBus());
compareBtn.addEventListener("click", () => compareInput.click());
compareInput.addEventListener("change", () => {
  const f = compareInput.files[0];
  if (!f) return;
  compareInput.value = "";
  const label = f.name.replace(/\.[^.]+$/, "").replace(/[^\w.-]+/g, "_");
  setStatus(`Reading ${f.name}…`);
  f.arrayBuffer()
    .then((buffer) => {
      setStatus("");
      worker.postMessage({ type: "open-second", label, bytes: buffer }, [buffer]);
    })
    .catch((e) => showError(`Could not read ${f.name}: ${e.message ?? e}`));
});
computedBtn.addEventListener("click", () => {
  const expr = computedExpr.value.trim();
  if (!expr || !fileOpen) return;
  // Name: first unused ChannelN, or the user's implicit choice later via
  // the legend rename — a simple counter keeps the box one field.
  let n = 1;
  const taken = new Set([...channels.map((c) => c.name), ...shown.map((s2) => s2.name)]);
  while (taken.has(`Computed ${n}`)) n += 1;
  worker.postMessage({ type: "computed", name: `Computed ${n}`, expr });
  computedExpr.value = "";
});
dbcInput.addEventListener("change", () => {
  const f = dbcInput.files[0];
  if (!f) return;
  dbcInput.value = "";
  setStatus(`Reading ${f.name}…`);
  f.arrayBuffer()
    .then((buffer) => {
      setStatus("");
      worker.postMessage({ type: "attach-dbc", bytes: buffer }, [buffer]);
    })
    .catch((e) => showError(`Could not read the DBC: ${e.message ?? e}`));
});
detailsBtn.addEventListener("click", () => toggleDetails());
// Scroll-driven paging: requestTablePage re-checks coverage on every event,
// so consecutive scrolls inside the held page cost nothing.
tablescrollEl.addEventListener("scroll", () => requestTablePage());

sharedYEl.addEventListener("change", () => requestAnimationFrame(draw));

function renderModeButtons() {
  overlayBtn.classList.toggle("active", !stacked);
  stackedBtn.classList.toggle("active", stacked);
  // Shared Y is an overlay-only choice. Disable rather than uncheck, so
  // the setting is still there after a switch to stacked and back.
  sharedYEl.disabled = stacked;
  sharedYLabel.classList.toggle("off", stacked);
}

function setStacked(v) {
  stacked = v;
  try {
    sessionStorage.setItem("plot-mode", v ? "stacked" : "overlay");
  } catch {
    // blocked storage: the choice just won't persist (see the state block)
  }
  renderModeButtons();
  requestAnimationFrame(draw);
}

overlayBtn.addEventListener("click", () => setStacked(false));
stackedBtn.addEventListener("click", () => setStacked(true));
renderModeButtons(); // reflect a choice restored from sessionStorage

function renderTimeButtons() {
  relativeBtn.classList.toggle("active", !absoluteTime);
  absoluteBtn.classList.toggle("active", absoluteTime);
  // A file whose start timestamp didn't parse gets no absolute axis: the
  // button disables with the reason in its tooltip instead of a broken axis.
  absoluteBtn.disabled = startEpochMs === null;
  absoluteBtn.title = startEpochMs === null
    ? "This file's start timestamp is unusable — relative time only"
    : "Wall-clock time anchored at the file's start timestamp, in your local timezone";
}

function setTimeMode(v) {
  absoluteTime = v && startEpochMs !== null;
  try {
    sessionStorage.setItem("time-mode", absoluteTime ? "absolute" : "relative");
  } catch {
    // blocked storage: the choice just won't persist (see the state block)
  }
  renderTimeButtons();
  renderReadout(); // the cursor readout gains or loses its wall-clock lines
  requestAnimationFrame(draw);
}

relativeBtn.addEventListener("click", () => setTimeMode(false));
absoluteBtn.addEventListener("click", () => setTimeMode(true));
renderTimeButtons(); // pre-file state: absolute stays disabled until meta lands

new ResizeObserver(() => {
  requestAnimationFrame(draw);
  scheduleRequestAll(); // the point budget follows the canvas width
}).observe(plotwrap);

// ---------------------------------------------------------------------------
// GPS panel (plan 3.4): the coordinate pair as a canvas track, colored by
// speed where a speed channel exists. The plot's cursors mark their position
// on the track, and a track click places cursor A at that point's time —
// position and plot are two views of one timeline.

function toggleGps(force) {
  gpsOpen = force !== undefined ? force : !gpsOpen;
  gpsBtn.setAttribute("aria-pressed", String(gpsOpen));
  gpspanelEl.hidden = !gpsOpen;
  if (gpsOpen) {
    gpsTrack = null;
    worker.postMessage({
      type: "gps-track",
      lat: gpsPair.latitude,
      lon: gpsPair.longitude,
      speed: gpsSpeed,
    });
  }
}

gpsBtn.addEventListener("click", () => toggleGps());

// Index of the track point nearest time `t` (the track's t is sorted —
// it rides the latitude channel's master).
function trackIndexAt(t) {
  if (!gpsTrack) return -1;
  const ts = gpsTrack.t;
  let lo = 0;
  let hi = ts.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (ts[mid] < t) lo = mid + 1;
    else hi = mid;
  }
  if (lo <= 0) return ts.length ? 0 : -1;
  if (lo >= ts.length) return ts.length - 1;
  return t - ts[lo - 1] <= ts[lo] - t ? lo - 1 : lo;
}

function drawGps() {
  if (!gpsOpen || !gpsTrack || gpsTrack.n === 0) {
    gpslegendEl.textContent = gpsOpen ? "no finite coordinates in this file" : "";
    return;
  }
  const dpr = window.devicePixelRatio || 1;
  const w = gpspanelEl.clientWidth;
  const h = 320;
  if (gpsplotEl.width !== Math.round(w * dpr)) {
    gpsplotEl.width = Math.round(w * dpr);
  }
  gpsplotEl.height = Math.round(h * dpr);
  const ctx = gpsplotEl.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);

  const { lat, lon, speed } = gpsTrack;
  // Equirectangular projection with a cosine latitude correction — degrees
  // are not square off the equator, and a squashed track reads wrong.
  let latLo = Infinity, latHi = -Infinity, lonLo = Infinity, lonHi = -Infinity;
  for (let i = 0; i < gpsTrack.n; i++) {
    if (Number.isFinite(lat[i])) {
      latLo = Math.min(latLo, lat[i]);
      latHi = Math.max(latHi, lat[i]);
    }
    if (Number.isFinite(lon[i])) {
      lonLo = Math.min(lonLo, lon[i]);
      lonHi = Math.max(lonHi, lon[i]);
    }
  }
  if (!(latLo < latHi) && !(lonLo < lonHi)) {
    gpslegendEl.textContent = "no finite coordinates in this file";
    return;
  }
  if (!(latLo < latHi)) { latLo -= 0.01; latHi += 0.01; }
  if (!(lonLo < lonHi)) { lonLo -= 0.01; lonHi += 0.01; }
  const meanLat = (latLo + latHi) / 2;
  const kx = Math.cos((meanLat * Math.PI) / 180);
  const pad = 24;
  const spanX = (lonHi - lonLo) * kx || 1e-9;
  const spanY = latHi - latLo || 1e-9;
  const scale = Math.min((w - pad * 2) / spanX, (h - pad * 2) / spanY);
  const X = (lo) => pad + ((lo - lonLo) * kx) * scale + (w - pad * 2 - spanX * scale) / 2;
  const Y = (la) => h - pad - (la - latLo) * scale - (h - pad * 2 - spanY * scale) / 2;

  // Speed coloring: green → red over the track's own speed range.
  let spLo = Infinity, spHi = -Infinity;
  if (speed) {
    for (const v of speed) {
      if (Number.isFinite(v)) {
        spLo = Math.min(spLo, v);
        spHi = Math.max(spHi, v);
      }
    }
  }
  const colorOf = (i) => {
    if (!speed) return "#4f8cff";
    const v = speed[i];
    if (!Number.isFinite(v) || !(spHi > spLo)) return "#8b93a7";
    const f = (v - spLo) / (spHi - spLo);
    return `hsl(${(1 - f) * 130} 70% 45%)`;
  };

  ctx.lineWidth = 2;
  let pen = false;
  let px = 0;
  let py = 0;
  for (let i = 0; i < gpsTrack.n; i++) {
    const la = lat[i];
    const lo = lon[i];
    if (!Number.isFinite(la) || !Number.isFinite(lo)) {
      pen = false;
      continue;
    }
    const x = X(lo);
    const y = Y(la);
    if (pen && speed) {
      // Per-segment stroke keeps the color honest at the cost of more paths;
      // tracks are short enough after decimation that this is fine.
      ctx.strokeStyle = colorOf(i - 1);
      ctx.beginPath();
      ctx.moveTo(px, py);
      ctx.lineTo(x, y);
      ctx.stroke();
    } else if (pen) {
      ctx.strokeStyle = "#4f8cff";
      ctx.beginPath();
      ctx.moveTo(px, py);
      ctx.lineTo(x, y);
      ctx.stroke();
    } else {
      ctx.fillStyle = colorOf(i);
      ctx.beginPath();
      ctx.arc(x, y, 2, 0, 7);
      ctx.fill();
    }
    px = x;
    py = y;
    pen = true;
  }

  // Start marker.
  if (Number.isFinite(lat[0]) && Number.isFinite(lon[0])) {
    ctx.fillStyle = "#62d96b";
    ctx.beginPath();
    ctx.arc(X(lon[0]), Y(lat[0]), 4, 0, 7);
    ctx.fill();
  }

  // Cursors → position: A and B marked where they land on the track.
  const mark = (c, label, color) => {
    if (!c) return;
    const i = trackIndexAt(c.t);
    if (i < 0) return;
    const x = X(lon[i]);
    const y = Y(lat[i]);
    ctx.strokeStyle = color;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.arc(x, y, 6, 0, 7);
    ctx.stroke();
    ctx.fillStyle = color;
    ctx.font = "bold 11px system-ui, sans-serif";
    ctx.fillText(label, x + 9, y - 6);
    ctx.font = "11px system-ui, sans-serif";
  };
  mark(cursorA, "A", "#4f8cff");
  mark(cursorB, "B", "#ffb454");

  const spNote = speed
    ? ` · speed ${fmtNumber(spLo)}–${fmtNumber(spHi)} ${shown.find((s) => s.name === gpsSpeed)?.unit ?? ""}`
    : "";
  gpslegendEl.textContent =
    `${gpsTrack.n.toLocaleString()} points · ` +
    `${latLo.toFixed(4)}, ${lonLo.toFixed(4)} → ${latHi.toFixed(4)}, ${lonHi.toFixed(4)}` +
    (gpsTrack.aligned ? "" : " · channels not sample-aligned") +
    spNote;
}

gpsplotEl.addEventListener("click", (e) => {
  // Track → plot: cursor A at the clicked point's time.
  if (!gpsTrack) return;
  const rect = gpsplotEl.getBoundingClientRect();
  const i = trackIndexAt(0); // replaced below by pixel-based lookup
  void i;
  // Find the nearest drawn point by pixel distance.
  let best = -1;
  let bestD = Infinity;
  const t0 = gpsTrack.t[0];
  const dpr = window.devicePixelRatio || 1;
  const w = gpspanelEl.clientWidth;
  const h = 320;
  void dpr;
  void w;
  void h;
  void t0;
  void bestD;
  void best;
  // Pixel lookup needs the projection; recompute it exactly as drawGps did.
  const { lat, lon } = gpsTrack;
  let latLo = Infinity, latHi = -Infinity, lonLo = Infinity, lonHi = -Infinity;
  for (let k = 0; k < gpsTrack.n; k++) {
    if (Number.isFinite(lat[k])) {
      latLo = Math.min(latLo, lat[k]);
      latHi = Math.max(latHi, lat[k]);
    }
    if (Number.isFinite(lon[k])) {
      lonLo = Math.min(lonLo, lon[k]);
      lonHi = Math.max(lonHi, lon[k]);
    }
  }
  if (!(latLo < latHi) && !(lonLo < lonHi)) return;
  if (!(latLo < latHi)) { latLo -= 0.01; latHi += 0.01; }
  if (!(lonLo < lonHi)) { lonLo -= 0.01; lonHi += 0.01; }
  const meanLat = (latLo + latHi) / 2;
  const kx = Math.cos((meanLat * Math.PI) / 180);
  const pad = 24;
  const spanX = (lonHi - lonLo) * kx || 1e-9;
  const spanY = latHi - latLo || 1e-9;
  const scale = Math.min((rect.width - pad * 2) / spanX, (rect.height - pad * 2) / spanY);
  const px0 = pad + (spanX * scale) / 2;
  const py0 = rect.height - pad - (spanY * scale) / 2;
  for (let k = 0; k < gpsTrack.n; k++) {
    if (!Number.isFinite(lat[k]) || !Number.isFinite(lon[k])) continue;
    const dx = pad + (lon[k] - lonLo) * kx * scale + (rect.width - pad * 2 - spanX * scale) / 2 - (e.clientX - rect.left);
    const dy = rect.height - pad - (lat[k] - latLo) * scale - (rect.height - pad * 2 - spanY * scale) / 2 - (e.clientY - rect.top);
    const d = dx * dx + dy * dy;
    if (d < bestD) {
      bestD = d;
      best = k;
    }
  }
  if (best >= 0 && bestD < 40 * 40) {
    placeCursor(gpsTrack.t[best], false);
  }
  void px0;
});

// ---------------------------------------------------------------------------
// X-Y panel (plan 4.1): two plotted channels as value-vs-value points. The
// worker samples Y at X's timestamps; the canvas draws the polyline with
// auto-scaled axes and channel labels. A hover readout names the nearest
// point's pair and time.

function toggleXy(force) {
  xyOpen = force !== undefined ? force : !xyOpen;
  xyBtn.setAttribute("aria-pressed", String(xyOpen));
  xypanelEl.hidden = !xyOpen;
  if (xyOpen) rebuildXySelectors();
}

xyBtn.addEventListener("click", () => toggleXy());

function rebuildXySelectors() {
  const names = shown.map((s) => s.name);
  const prevX = xyxEl.value;
  const prevY = xyyEl.value;
  for (const sel of [xyxEl, xyyEl]) {
    sel.replaceChildren();
    for (const n of names) {
      const opt = document.createElement("option");
      opt.value = n;
      opt.textContent = n;
      sel.append(opt);
    }
  }
  xyxEl.value = names.includes(prevX) ? prevX : names[0] ?? "";
  xyyEl.value = names.includes(prevY) ? prevY : names[1] ?? names[0] ?? "";
  requestXy();
}

function requestXy() {
  if (!xyOpen || !xyxEl.value || !xyyEl.value) return;
  worker.postMessage({ type: "xy", nameX: xyxEl.value, nameY: xyyEl.value });
}

xyxEl.addEventListener("change", requestXy);
xyyEl.addEventListener("change", requestXy);

function drawXy() {
  if (!xyOpen || !xyPair || xyPair.count === 0) return;
  const dpr = window.devicePixelRatio || 1;
  const w = xypanelEl.clientWidth;
  const h = 340;
  xycanvasEl.width = Math.round(w * dpr);
  xycanvasEl.height = Math.round(h * dpr);
  const ctx = xycanvasEl.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  ctx.font = "11px system-ui, sans-serif";

  const { xs, ys, count } = xyPair;
  let xLo = Infinity, xHi = -Infinity, yLo = Infinity, yHi = -Infinity;
  for (let i = 0; i < count; i++) {
    if (Number.isFinite(xs[i])) {
      xLo = Math.min(xLo, xs[i]);
      xHi = Math.max(xHi, xs[i]);
    }
    if (Number.isFinite(ys[i])) {
      yLo = Math.min(yLo, ys[i]);
      yHi = Math.max(yHi, ys[i]);
    }
  }
  if (!(xLo < xHi)) { xLo -= 0.5; xHi += 0.5; }
  if (!(yLo < yHi)) { yLo -= 0.5; yHi += 0.5; }
  const padX = 64;
  const padY = 24;
  const px = (v) => padX + ((v - xLo) / (xHi - xLo)) * (w - padX - 16);
  const py = (v) => h - padY - ((v - yLo) / (yHi - yLo)) * (h - padY * 2);

  // Grid: 5 divisions each way, labeled from the real ranges.
  ctx.strokeStyle = "#242a38";
  ctx.fillStyle = "#8b93a7";
  ctx.lineWidth = 1;
  for (let g = 0; g <= 5; g++) {
    const x = padX + ((w - padX - 16) * g) / 5;
    const y = padY + ((h - padY * 2) * g) / 5;
    ctx.beginPath();
    ctx.moveTo(x, padY);
    ctx.lineTo(x, h - padY);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(padX, y);
    ctx.lineTo(w - 16, y);
    ctx.stroke();
    ctx.textAlign = "center";
    ctx.fillText(fmtNumber(xLo + ((xHi - xLo) * g) / 5), x, h - padY + 14);
    ctx.textAlign = "right";
    ctx.fillText(fmtNumber(yHi - ((yHi - yLo) * g) / 5), padX - 6, y + 4);
  }

  ctx.strokeStyle = xyPair.nameX === xyyEl.value ? "#ffb454" : "#4f8cff";
  ctx.lineJoin = "round";
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  let pen = false;
  for (let i = 0; i < count; i++) {
    if (!Number.isFinite(xs[i]) || !Number.isFinite(ys[i])) {
      pen = false;
      continue;
    }
    if (pen) ctx.lineTo(px(xs[i]), py(ys[i]));
    else {
      ctx.moveTo(px(xs[i]), py(ys[i]));
      pen = true;
    }
  }
  ctx.stroke();

  ctx.fillStyle = "#8b93a7";
  ctx.textAlign = "left";
  ctx.fillText(`${xyPair.nameY} vs ${xyPair.nameX}`, padX, 14);
}

new ResizeObserver(() => {
  if (xyOpen) drawXy();
}).observe(xypanelEl);

new ResizeObserver(() => {
  if (gpsOpen) drawGps();
}).observe(gpspanelEl);

// ---------------------------------------------------------------------------
// File plumbing (drag & drop, picker, bundled sample) — unchanged in spirit

async function loadLocalFile(f) {
  setStatus(`Reading ${f.name}…`);
  try {
    const bytes = await f.arrayBuffer();
    rememberLabel.hidden = false;
    rememberBox.onchange = async () => {
      if (rememberBox.checked) {
        rememberBox.checked = false; // one-shot: the cache either takes it or not
        const ok = await opfsStore(f.name, bytes);
        setStatus(ok ? `${f.name} kept in browser storage` : "");
        if (!ok) setStatus("could not keep the file in browser storage");
      }
    };
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
  cursorA = null;
  cursorB = null;
  viewStats = null;
  regionStats = null;
  readoutEl.hidden = true;
  tableOpen = false;
  tablePage = null;
  tablepanelEl.hidden = true;
  tableBtn.setAttribute("aria-pressed", "false");
  detailsOpen = false;
  detailspanelEl.hidden = true;
  detailsBtn.setAttribute("aria-pressed", "false");
  busOpen = false;
  busPage = null;
  busGroups = [];
  busHighlight = -1;
  buspanelEl.hidden = true;
  busBtn.hidden = true;
  gpsOpen = false;
  gpsTrack = null;
  gpsPair = null;
  gpspanelEl.hidden = true;
  gpsBtn.hidden = true;
  // Structure tab state dies with the file; the tree refetches on demand.
  structureData = null;
  structState.clear();
  structFilterEl.value = "";
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
  cursorA = null;
  cursorB = null;
  viewStats = null;
  regionStats = null;
  readoutEl.hidden = true;
  tableOpen = false;
  tablePage = null;
  tablepanelEl.hidden = true;
  tableBtn.setAttribute("aria-pressed", "false");
  detailsOpen = false;
  detailspanelEl.hidden = true;
  detailsBtn.setAttribute("aria-pressed", "false");
  busOpen = false;
  busPage = null;
  busGroups = [];
  busHighlight = -1;
  buspanelEl.hidden = true;
  busBtn.hidden = true;
  gpsOpen = false;
  gpsTrack = null;
  gpsPair = null;
  gpspanelEl.hidden = true;
  gpsBtn.hidden = true;
  structureData = null;
  structState.clear();
  structFilterEl.value = "";
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
      deepLinkFile = "sample.mf4";
      openFile(await res.arrayBuffer(), "sample.mf4 (synthetic)");
    } catch (err) {
      setStatus("");
      showError(`Could not load the sample: ${err.message ?? err}`);
    }
  });
  closeBtn.addEventListener("click", reset);
  // The filter box: instant local substring for the first keystroke feel,
  // then the debounced worker search (reader index / RegExp) decides the
  // list. Mode switches re-run the search even for an unchanged query.
  let searchTimer = null;
  const scheduleSearch = () => {
    renderChannelList();
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      const q = filterInput.value.trim();
      if (!q || !fileOpen) {
        searchResults = null;
        renderChannelList();
        return;
      }
      searchEpoch += 1;
      worker.postMessage({
        type: "search",
        id: searchEpoch,
        q,
        mode: searchModeEl.value,
      });
    }, 150);
  };
  filterInput.addEventListener("input", scheduleSearch);
  searchModeEl.addEventListener("change", scheduleSearch);
  // The structure pane: tab switching, its own filter, bulk open/close.
  tabStructureBtn.addEventListener("click", () => showSideTab("structure"));
  tabChannelsBtn.addEventListener("click", () => showSideTab("channels"));
  structFilterEl.addEventListener("input", () => {
    if (structureOpen) renderStructure();
  });
  structExpandBtn.addEventListener("click", () => setAllStructSections(true));
  structCollapseBtn.addEventListener("click", () => setAllStructSections(false));
}

wire();
setStatus("Ready — drop a file, or load the bundled sample.");
opfsInit();

// Deep link: ?file=<same-origin path> loads that recording on boot, the same
// code path as the sample button (fetch → worker). Lets a recording be shared
// by URL and lets automated checks drive real files without a file picker.
function readHashState() {
  if (!location.hash || location.hash === "#") return null;
  const p = new URLSearchParams(location.hash.slice(1));
  const state = { channels: [], t0: null, t1: null, mode: null };
  const f = p.get("f");
  const c = p.get("c");
  if (c) state.channels = c.split(",").filter(Boolean);
  const t0 = parseFloat(p.get("t0"));
  const t1 = parseFloat(p.get("t1"));
  if (Number.isFinite(t0) && Number.isFinite(t1)) {
    state.t0 = t0;
    state.t1 = t1;
  }
  if (p.get("m") === "stacked") state.mode = "stacked";
  return f ? { ...state, file: f } : null;
}

// Applies a shared view once the file has opened: restores the layout mode,
// toggles the shared channels (the first series bootstraps the view), and
// then narrows the view into the shared window.
function applyHashState() {
  const st = readHashState();
  if (!st) return;
  if (st.mode === "stacked") setStacked(true);
  const wanted = st.channels.filter((n) => channels.some((c) => c.name === n));
  for (const name of wanted.slice(0, MAX_CHANNELS)) {
    if (!shownNames().has(name)) toggleChannel(name);
  }
  if (st.t0 !== null && view) {
    view = { t0: st.t0, t1: st.t1 };
    clampView();
    requestAll();
  }
}

(async () => {
  const params = new URLSearchParams(location.search);
  const target = params.get("file");
  if (!target || target.includes("://")) return;
  hashPending = Boolean(readHashState());
  const compareTarget = params.get("compare");
  if (compareTarget && !compareTarget.includes("://")) {
    try {
      const res = await fetch(compareTarget);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const label = (compareTarget.split("/").pop() || "second")
        .replace(/\.[^.]+$/, "")
        .replace(/[^\w.-]+/g, "_");
      pendingCompare = [label, await res.arrayBuffer()];
    } catch (err) {
      showError(`Could not load the compare file ${compareTarget}: ${err.message ?? err}`);
    }
  }
  const dbcTarget = params.get("dbc");
  if (dbcTarget && !dbcTarget.includes("://")) {
    try {
      const res = await fetch(dbcTarget);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      pendingDbc = await res.arrayBuffer();
    } catch (err) {
      showError(`Could not load the DBC ${dbcTarget}: ${err.message ?? err}`);
    }
  }
  try {
    setStatus(`Fetching ${target}…`);
    const res = await fetch(target);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    deepLinkFile = target;
    openFile(
      await res.arrayBuffer(),
      target.split("/").pop() || target
    );
  } catch (err) {
    setStatus("");
    showError(`Could not load ${target}: ${err.message ?? err}`);
  }
})();
