# MDF4 format reference

Sources checked 2026-09-05:
[ASAM overview](https://www.asam.net/standards/detail/mdf/wiki/),
[asammdf block definitions](https://github.com/danielhrisca/asammdf/blob/master/src/asammdf/blocks/v4_blocks.py),
[asammdf MDF4 reader](https://github.com/danielhrisca/asammdf/blob/master/src/asammdf/blocks/mdf_v4.py).
These moving source links are entry points; pin a commit for a specific
compatibility investigation. This guide also describes the local Rust APIs;
it is not a replacement for the applicable ASAM specification.

## Block graph

```text
ID (64 bytes) -> HD
                 |-> DG -> next DG
                 |    |-> CG -> next CG
                 |    |    |-> CN -> next CN / composition
                 |    |    |    |-> TX/MD, SI, CC, CA or signal data
                 |    |    |-> acquisition metadata / sample reductions
                 |    |-> record storage (DT, DL/HL/DZ; version-dependent LD)
                 |-> FH, CH, AT, EV and file metadata
```

The MDF4 common header occupies 24 bytes: identifier, reserved bytes,
u64 total block length, u64 link count. Links are u64 absolute file offsets;
zero denotes an absent link. ID is special; HD begins at offset 64.
Header fields use little-endian encoding; channel payload endianness is
independently described by CN.

Do not require the whole declared block in a header-only parser, but validate
the actual slice before reading its links or payload. A link table must fit
both its declared block and the bytes available to the operation. Check
addition and multiplication overflow before allocation. A u64 disk offset or
count may not fit usize on wasm32; do not truncate it with an unchecked cast.
Inspect parser/links.rs for cycle handling rather than treating every repeated
reference to shared metadata as a cycle.

## Records and storage

DG groups describe storage; CG groups describe record layouts; CN channels
describe the location and representation of each field. Compute sample stride
from the owning CG, not from the selected channel's width. Invalidation bytes
are additional to CG data bytes. Record IDs distinguish interleaved CG records;
do not assume their width is always one byte.

A sorted DG has one CG layout. That format property does not make corrupted
master timestamps safe for binary search. Preserve equal-time sample order
where the existing API accepts duplicate timestamps; do not silently sort
timestamps without applying the same permutation to values and validity.

| Storage | What the reader must preserve |
| --- | --- |
| DT | Record bytes and stride, including invalidation |
| DL / HL | Logical data order across linked chunks, even if records span chunks |
| DZ | Original block kind, codec, decoded length and optional transposition |
| SD | Variable-length signal payloads; this is not a “sorted data” block |
| SR / RD | Sample reduction descriptors / reduction records, not ordinary samples |
| LD / DV / DI | MDF 4.20 linked values and separate invalidation data |

For transposed DZ payloads, undo the declared layout and preserve any
non-transposed tail. A generic matrix transpose can appear correct for square
fixtures while failing rectangular ones. Use the existing external conformance
test. Do not turn the wiki's historical compression discussion into an
unconditional size cap for all MDF versions/codecs.

## Channels, masters and validity

CN encodes byte offset, bit offset, bit width, type and conversion reference.
Signed sub-byte integers require sign extension from the declared width;
non-native endian data needs the corresponding extraction path.

The master can describe time or another synchronization domain. Keep its unit
and synchronization type. Virtual channels derive values from the record index;
they do not necessarily occupy bytes. Sync/media channels are not scalar plots.

CN flags can mark all samples invalid or select a bit in the CG invalidation
area. On disk a set invalidation bit means invalid; the Rust API returns
true for valid. Invalid raw bits still exist in the file and must not become
measurements in statistics, plots or exports. A valid data value does not make
an invalid master value a valid coordinate.

CC describes conversion from raw to physical representation. Preserve text
conversion results as text. Do not apply a conversion twice or assume all
conversions are linear. Metadata strings and sample strings use different
rules: TX/MD metadata is UTF-8; sample encoding comes from CN.

## Arrays, structures and variable-length data

CA carries shape, storage and dependencies. CN-template array elements can
share one record; their flattened values are not independent samples. Fixed
arrays have an element stride; dynamic arrays need each sample's actual
boundaries. Invalidation may be element-specific.

CN composition may describe nested structures. Preserve the field layout
and channel location when flattening a display tree.

VLSD can use signal-data offsets or a companion channel group. Follow the
declared storage form rather than assuming an offset points directly into DT.
Check payload lengths and offsets before slicing, and keep streaming support
separate from full materialization support.

## Version boundaries

Use ID version and feature fields together with the encountered block type.
MDF3 has different headers/links and its own decoder under src/mdf3; the common
ID signature is not evidence that MDF4 offsets apply. An unfinished file needs
the specific recovery indicated by its flags; do not blanket-ignore damaged
block lengths. Unknown features should produce a located capability error
where possible, while unrelated readable channels remain useful.
