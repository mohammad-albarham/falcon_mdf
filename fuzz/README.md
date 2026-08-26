# Fuzzing `falcon_mdf`

This directory contains fuzz harnesses powered by `libFuzzer` and `cargo-fuzz`.

## Prerequisites

A nightly Rust toolchain and `cargo-fuzz` are required:

```bash
cargo install cargo-fuzz
```

## Targets

### 1. `parse`
Fuzzes the whole MF4 read path (opening files and decoding every channel).

```bash
cargo +nightly fuzz run parse
```

Optionally seed corpus from test files:
```bash
mkdir -p fuzz/corpus/parse
find test_data -name '*.MF4' -exec cp {} fuzz/corpus/parse/ \;
```

### 2. `roundtrip`
Fuzzes the write-read path symmetry end-to-end (`Mf4Writer` samples -> read back -> verify bit-equality).

```bash
cargo +nightly fuzz run roundtrip
```

### 3. `differential`
Parses the same input twice (in-process with debug assertions vs release helper binary) to detect differential output divergence.

```bash
cargo +nightly fuzz build helper
HELPER_PATH=fuzz/target/release/helper cargo +nightly fuzz run differential
```

### 4. `read_bits`
Fuzzes low-level bit extraction (`read_bits` / `read_uint`) against an independent, bit-by-bit reference oracle with explicit bounds checking.

**Standard run (debug assertions ON):**
```bash
cargo +nightly fuzz run read_bits
```

**Release run (debug assertions OFF, shipping consumer profile):**
By default, `cargo-fuzz` compiles targets with debug assertions and integer overflow checks enabled (`-a, --debug-assertions`). In a shipping release build, debug assertions and overflow checks are disabled. To run the fuzzer against the true release profile without debug assertions, pass `-O` / `--release`:

```bash
cargo +nightly fuzz run --release read_bits
```
