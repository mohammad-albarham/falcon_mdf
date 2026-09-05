---
name: mf4-structure
description: Review or implement MDF4/MF4 block parsing, record decoding, writing, and channel handling in falcon_mdf, including its GUI and WASM consumers. Use for format correctness, compatibility, channel identity, validity, and data-layout changes.
---

# MDF4 structure and implementation

Use this skill when a change depends on what an MF4 file means. Start with
the affected reader/writer path and its tests, then read the relevant sections
of [the format reference](references/format.md). For file navigation and
verification targets, use [the repository map](references/repository.md).

## Evidence

- The [ASAM MDF wiki](https://www.asam.net/standards/detail/mdf/wiki/) is an
  explanatory overview, not the complete versioned specification. Use the
  applicable ASAM specification to settle requirements the wiki does not define;
  say when it is unavailable.
- [asammdf](https://github.com/danielhrisca/asammdf) is an independent
  implementation and conformance oracle, not the standard. Record its installed
  version or source commit when comparing behavior. Do not copy its code into
  this crate; use independently constructed fixtures and observable results.
- README limitations and old agent reports can lag implementation. Confirm
  support in current code and tests. A writer/reader round trip can share the
  same bug; use external read-back for writer changes.

## Decisions that preserve measurement meaning

1. Follow block links, not physical block order. Bound sizes, link counts,
   offsets and allocation requests against supplied bytes; handle cycles.
2. Keep sample count separate from byte length and flattened array element
   count. A record includes its declared data and invalidation area; unsorted
   data also requires the correct record ID and group layout.
3. Identify a channel by file plus group/channel location. Names and display
   names are labels, not globally unique keys. Caches, selections, asynchronous
   replies and exported metadata must retain that identity.
4. Preserve the selected channel's master, conversion, units and validity.
   Never turn an invalid sample into zero. Do not silently invent timestamps
   for a present but unusable master; sample indices are a fallback only when
   the group has no master.
5. Keep typed values through decoding and export. A scalar plot cannot consume
   flattened arrays as though each element had its own timestamp. Text, bytes,
   complex values and integers beyond exact f64 precision need explicit views.
6. Respect API scope: opening metadata, decoding a channel, streaming it,
   displaying it and writing it are separate capabilities.

## Working on a format change

Find the smallest reproducible block graph or record layout. Assert values,
shape, master and invalidity, not merely that opening succeeds. Run the new
regression against the old implementation before fixing it. Exercise relevant
compressed/uncompressed and sorted/unsorted paths without presuming all
combinations are supported.

Use the checks in repository AGENTS.md; ensure corpus-dependent tests actually
have fixtures and conformance tests can import asammdf. For performance claims,
read ../perf-benchmark/SKILL.md and follow its methodology. For browser boundary
changes, read ../rust-wasm/SKILL.md and test the worker protocol as well as Rust.

Update support documentation only to the extent demonstrated by tests. A
synthetic fixture establishes the tested layout, not complete vendor or
version compatibility.
