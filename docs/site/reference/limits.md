# Limits and errors

Falcon MDF reports unsupported layouts instead of returning plausible but incomplete samples. Check these boundaries when handling files from a new recorder or vendor.

## Known format limits

- MDF 4.20 `##LD` chains with separate invalidation and incompatible channel-group layouts cannot be interleaved unambiguously. Opening such a file fails. Compatible linked-data layouts are supported.
- Arrays with more than one dynamically sized dimension, or a sizing channel in another record stream, are unsupported.
- CSV, MAT, and HDF5 exporters cannot represent arrays whose length varies per sample. Arrow and Parquet write those arrays as list columns.
- Editing an arbitrary existing file through `Mf4Writer::from_file` is not lossless: unreadable channels and some metadata may be omitted. Use `from_file_with_report` to inspect what was not carried over.

Big-endian channels have synthetic test coverage, but no available vendor file in the project's test corpus. The vendor reference corpus is primarily MDF 4.11. A generated MDF 4.20 linked-data file is tested; this does not establish full cross-vendor MDF 4.20 coverage.

See the [format review](https://github.com/mohammad-albarham/falcon_mdf/blob/main/docs/mf4-review.md) and [field reference](https://mohammad-albarham.github.io/falcon_mdf/field-reference.html) for deeper implementation detail. The field reference names the commit it describes; later changes should be checked against the current tree.

## Error handling

Most operations return `Result<T, Mf4Error>`. Common variants include `Io`, `InvalidSignature`, `UnsupportedVersion`, `InvalidBlockId`, `InvalidBlockSize`, `Decompression`, `ChannelNotFound`, and `Unsupported`.

`Unsupported` names the channel or feature this build cannot decode. Most such errors affect only that channel; a file can still open and other channels can be read. The incompatible MDF 4.20 linked-data case above is a whole-file error during opening.

If a compressed data block uses zstd or LZ4 and the corresponding feature is disabled, the error names the missing codec. Enable the feature rather than interpreting the undecoded bytes as values.
