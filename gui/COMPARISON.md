# falcon against the other MF4 viewers

Written to keep this project honest about what it is. The comparison is with
[asammdf](https://github.com/danielhrisca/asammdf), which is the tool most
people reach for and the one this viewer is measured against, and with the
vendor tools — CANape, INCA, ControlDesk — that most measurement files are
recorded by.

None of this is a claim that falcon should replace them. It is a claim about
which questions each answers quickly.

## What falcon does that the others do not

**It shows the file as a file.** Every block from byte 0 to the last one, in
address order, with its length, its links under the format's own names
(`cn_tx_name`, `dg_cg_first`), the blocks that point back at it, and its raw
bytes. asammdf has a file-information dialog and a channel tree; it does not
have a block explorer, and neither do the vendor tools. When a file will not
open, or opens with a channel missing, this is the view that says why — and
it is a view the format's own specification is written in terms of.

**The links are navigable.** A `##CN` block's conversion link is a button;
pressing it selects the `##CC` block it names. Back and forward walk that
history. Reading a measurement file's structure by hand normally means a hex
editor and the specification open beside it.

**It accounts for every byte.** The block map reports what the blocks cover
and what they do not, split into alignment padding and larger uncovered
regions. On an unfinalized file — where the writer never recorded the last
data block's length — it says so, rather than leaving "50% covered" to be
read as damage.

**It refuses rather than approximates.** A channel this build cannot decode
carries a ⚠ and the reason on hover; it is never drawn as a flat line or
silently dropped. Statistics over an array channel say they cover every
element of every sample rather than quietly reporting a sample count that is
64 times too large. That rule comes from the library underneath, which fails
a read instead of returning part of the data.

## What asammdf does that falcon does not

Listed so nobody discovers it the hard way.

| Capability | asammdf | falcon |
| --- | --- | --- |
| Editing and writing | full — cut, resample, filter, concatenate, convert between MDF versions | a simple writer only: one group per channel, float64, no conversions, no round trip |
| Batch processing | a whole tab for it, over many files | one file at a time |
| Export formats | CSV, HDF5, MAT, Parquet, ASAM MDF | CSV and MF4 |
| Computed channels | expressions over other channels | none |
| Bus databases | DBC, ARXML **and LDF**, for CAN, LIN and FlexRay | DBC and ARXML, CAN only for decoding; LIN frames are read but not decoded |
| Windows and layouts | arbitrary window arrangements (Plot, Numeric, Tabular, GPS), saved as window layouts | six fixed content tabs (including Plot, Numeric, and a sortable, filterable Samples table); plotted set and active tabs remembered per file |
| Sample reduction | reads the reduced data | lists the levels; the reduced values are not read |
| Platform reach | anywhere Python runs, plus a packaged app | a native binary per platform |
| Maturity | years of use across the industry | this repository |

The honest summary: **asammdf is a measurement workbench and falcon is a
reader with a good window onto the file.** If the job is to convert, resample
or batch-process, asammdf is the tool. If the job is to open a file quickly
and find out what is actually in it — including when it is malformed —
falcon answers faster and in more detail.

## Where falcon is faster

Speed is the library's, not the window's. On the reference OBD2 CANedge log
(326,623 samples) reading is roughly 3.9× faster for decoding and 4.8× for a
whole read than the comparison implementation, and between 3.1× and 31.9×
across other uncompressed files — though at parity or slower on some
vendor-compressed files. The spread is real; the README's Performance section
carries the measured numbers and the files they came from.

What the viewer adds on top of that is bounded work rather than fast work:
the block list, the sample table and the frame list all build only the rows
on screen, so a file with a hundred thousand blocks and a group with ten
million samples both open at the same speed as a small one. Plotting
decimates to the pixel columns available, and a decode that could stall the
frame loop runs on a worker thread instead — which is why the window stays
responsive while a large group is being read.

## Where the comparison is unfair to falcon

Two of asammdf's advantages are not gaps in this design:

- **Python's ecosystem.** asammdf hands data to numpy, pandas and matplotlib
  in one line. falcon hands it to Rust. Neither is better; they are different
  jobs.
- **Feature count.** A viewer that reads and does not write has fewer
  features by construction, and a smaller surface to be wrong on. The
  library's promise is that a channel decodes correctly or reading it fails
  with a reason, and every feature added is a feature that promise has to
  hold across.

## Where the comparison is unfair to asammdf

- asammdf is verified by years of industrial use across far more files than
  this project's 67-file reference set and its bus-log corpus.
- Its GUI does things this one has not attempted — computed channels, saved
  window layouts, GPS views — and doing them well is more work than the list
  above makes it look.
- falcon's ground truth is generated *by* asammdf. Where the two disagree on
  a decoded value, the burden of proof is on falcon.
