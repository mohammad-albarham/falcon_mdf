# Repository map and checks

Paths are relative to the repository root. Verified against base commit
5cf939c478c318251d85492c7b192fd0aa7ce1f8; follow symbols rather than old line numbers.

| Concern | Implementation | Useful tests |
| --- | --- | --- |
| Bytes and bounds | src/io/, src/parser/binary.rs | tests/read_system.rs, tests/robustness.rs |
| Common headers and link tables | src/blocks/common.rs | tests/block_tests.rs, tests/link_bounds.rs |
| Linked graphs and version selection | src/parser/links.rs, src/parser/version.rs | tests/parser_tests.rs, tests/synthetic_blocks.rs |
| Channel/group metadata | src/model/mod.rs, src/blocks/channel.rs | tests/source_info.rs, tests/model_tests.rs |
| Physical/typed samples and invalidation | src/model/signal.rs, src/model/values.rs | tests/golden.rs, tests/reference.rs |
| Data loading and selection | src/file.rs, src/cache.rs | tests/batched_read.rs, tests/stream_aligned.rs |
| Bounded signal reads | src/stream.rs | tests/stream_chunks.rs, tests/vlsd_stream.rs |
| MDF 4.20 linked data | src/file.rs, src/blocks/data_block.rs, src/data_index.rs | tests/asammdf_ld_conformance.rs |
| DZ transposition | src/blocks/data_block.rs | tests/asammdf_untranspose_conformance.rs |
| Arrays and VLSD writing | src/write.rs | tests/write_ca.rs, tests/write_vlsd.rs |
| Interleaved CG writing | src/write.rs | tests/write_multi_cg.rs |
| Rewrite/export paths | src/write.rs, src/export/ | tests/write_rmw.rs, tests/write_conformance.rs |
| Time-domain operations | src/time_ops.rs, src/multi_ops.rs | tests/time_ops.rs, tests/multi_ops.rs |
| GUI scalar loading | gui/src/signal_loader.rs | gui/tests/master_axis.rs |
| GUI typed table and plotting | gui/src/panels/table.rs, gui/src/decimate.rs | gui/tests/table_export.rs, gui/tests/invalid_range_plots_a_gap.rs |
| WASM API | wasm/src/lib.rs | wasm/tests/*.rs |
| Browser ownership, caches, replies | wasm/demo/worker.js, wasm/demo/main.js | wasm/tests/worker.test.mjs |

The table includes regressions added with this skill. Read AGENTS.md for the
complete CI-equivalent checks. Browser protocol tests need only Node:
`node --test wasm/tests/worker.test.mjs`. The generated wasm/pkg and
wasm/demo/pkg directories must be rebuilt, not hand-edited.

Verify corpus presence with `rg --files --no-ignore test_data`; ordinary
`rg --files` hides gitignored MF4 fixtures. The local conformance interpreter
is `.venv/bin/python`; check `import asammdf` before trusting external tests.

## Known documentation drift to recheck

At the base commit, README still says arrays, VLSD, multiple CGs per DG and
modifying existing files cannot be written. However, src/write.rs exposes
add_channel_array, add_channel_vlsd, add_group_in and from_file, with dedicated
tests above. It also has an external test for 4.20 LD reading. Treat these as
specific tested capabilities, not proof of complete standard coverage.

The GUI's scalar plot and the browser's name-based API have narrower
capabilities than the core typed reader. In particular, two same-named channels
within one file still need an explicit location-based browser API; per-file
cache isolation alone does not solve that.
