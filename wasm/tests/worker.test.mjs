import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

// Exercise the shipped message handler; only the generated WASM boundary is
// replaced so file identity, cache ownership and transferred buffers are real JS.
const source = readFileSync(new URL("../demo/worker.js", import.meta.url), "utf8")
  .replace(/^import init,.*;$/m, "");

function worker() {
  const instances = [];
  const replies = [];
  class WasmMf4File {
    constructor(bytes) {
      if (!bytes[0]) throw new Error("invalid file");
      this.id = bytes[0];
      this.freed = false;
      this.reads = 0;
      instances.push(this);
    }
    free() { assert.equal(this.freed, false); this.freed = true; }
    channel_count() { return 1; }
    channels() { return "[]"; }
    channel_names() { return '["Speed"]'; }
    channel_kind(name) { return name === "State" || this.id === 3 ? "text" : "f64"; }
    signal_arrays() {
      this.reads++;
      return { timestamps: new Float64Array([0, 1, 2]), values: new Float64Array([this.id, this.id * 10, this.id * 100]) };
    }
    signal_window() {
      return { unit: "km/h", timestamps: new Float64Array([1]), values: new Float64Array([this.id * 10]) };
    }
    signal_text(name, t0, t1) {
      const timestamps = [0, 1, 2].filter(t => t >= t0 && t <= t1);
      return JSON.stringify({ unit: "", timestamps, labels: timestamps.map(t => `${this.id}:${t}`), truncated: false });
    }
  }
  const context = vm.createContext({
    WasmMf4File, init: async () => {}, Float64Array, Uint8Array,
    self: { postMessage: (msg, transfer) => replies.push(structuredClone(msg, { transfer })) },
  });
  vm.runInContext(source, context);
  return {
    instances,
    async send(data) {
      replies.length = 0;
      await context.self.onmessage({ data });
      return replies[0];
    },
  };
}

const open = (id = 1) => ({ type: "open", bytes: new Uint8Array([id]).buffer });
const second = (id = 2) => ({ type: "open-second", label: "B", bytes: new Uint8Array([id]).buffer });

test("equal channel names from two files retain independent cursor and table samples", async () => {
  const w = worker();
  await w.send(open());
  await w.send(second());
  const sample = await w.send({ type: "sample", names: ["Speed", "@B::Speed"], t: 1 });
  assert.deepEqual(sample.values, { Speed: 10, "@B::Speed": 20 });
  const page = await w.send({ type: "table", name: "@B::Speed", start: 0, count: 3 });
  assert.deepEqual([...page.values], [2, 20, 200]);
});

test("channel kinds and text caches belong to their file", async () => {
  const w = worker();
  await w.send(open());
  await w.send(second(3));
  const sample = await w.send({ type: "sample", names: ["Speed", "@B::Speed", "State", "@B::State"], t: 1 });
  assert.deepEqual(sample.values, { Speed: 10, "@B::Speed": "3:1", State: "1:1", "@B::State": "3:1" });
});

test("zoomed text replies pair each label with its window timestamp", async () => {
  const w = worker();
  await w.send(open());
  const series = await w.send({ type: "series", name: "State", t0: 1, t1: 1, maxPoints: 10 });
  assert.deepEqual([...series.timestamps], [1]);
  assert.deepEqual(series.labels, ["1:1"]);
  assert.equal(series.tMin, 0);
  assert.equal(series.tMax, 2);
});

test("dropping a comparison channel evicts only its own samples", async () => {
  const w = worker();
  await w.send(open());
  await w.send(second());
  const query = { type: "sample", names: ["Speed", "@B::Speed"], t: 1 };
  await w.send(query);
  await w.send({ type: "drop", names: ["@B::Speed"] });
  await w.send(query);
  assert.deepEqual(w.instances.map(f => f.reads), [1, 2]);
});

test("successful replacements free old files; failed replacements preserve them", async () => {
  const w = worker();
  await w.send(open());
  await w.send(second());
  assert.equal((await w.send(second(0))).type, "error");
  assert.equal(w.instances[1].freed, false);
  await w.send(second(3));
  assert.equal(w.instances[1].freed, true);
  assert.equal((await w.send(open(0))).type, "error");
  assert.equal(w.instances[0].freed, false);
  await w.send(open(4));
  assert.equal(w.instances[0].freed, true);
  assert.equal(w.instances[2].freed, true);
  assert.equal((await w.send({ type: "sample", names: ["@B::Speed"], t: 1 })).type, "error");
});

test("XY pairs use all X timestamps even when Y has fewer samples", async () => {
  const w = worker();
  await w.send(open());
  await w.send(second());
  w.instances[1].signal_arrays = () => ({ timestamps: new Float64Array([0, 2]), values: new Float64Array([20, 40]) });
  const xy = await w.send({ type: "xy", nameX: "Speed", nameY: "@B::Speed" });
  assert.equal(xy.count, 3);
  assert.deepEqual([...xy.xs], [1, 10, 100]);
  assert.deepEqual([...xy.ys], [20, 20, 40]);
});
