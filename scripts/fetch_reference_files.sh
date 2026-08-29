#!/usr/bin/env bash
#
# Downloads the MF4 files the reference tests compare against.
#
# These are other vendors' files and are not redistributed here: the script
# fetches them into `test_data/`, which is gitignored, and `tests/data/
# reference_golden.json` — which *is* checked in — holds only the decoded values
# they produce. A fresh clone therefore has the ground truth but not the files,
# and `tests/reference.rs` skips rather than fails until this is run.
#
# Four sources, all public and unauthenticated:
#
#   1. The ASAM vendor reference set — Vector, dSPACE and ETAS output, the
#      collection the openATFX-MDF project validates against. This is what makes
#      the suite worth having: it exercises 13 of 17 data types and 11 of 12
#      conversion types, against 3 and 2 for a corpus of bus logs.
#   2. Five files from asammdf's own test resources, two of which were written
#      by third-party tools (TGT 15.0 and dax3.0.0) and two of which are cut
#      halves of a batch measurement.
#   3. Three files PEAK System-Technik ships with its ASAM ODS example plugin,
#      including one that covers every data type and one compressed with
#      deflate — material the vendor publishes for third-party readers.
#   4. Four test files from CSS Electronics' MDF4 converter suite. Everything
#      above is a finalized 4.10 file; these are 4.11, three of the four are
#      *unfinalized* — the state a logger leaves a file in when it stops
#      without writing back the record counts — and one logs LIN rather than
#      CAN. `multiple.MF4` and `multiple_fin.MF4` are the same measurement
#      before and after finalization, so the pair pins that both read alike.
#      They are stored with Git LFS, hence the separate media.githubusercontent
#      host: raw.githubusercontent serves the pointer file, not the bytes.
#
# Usage: scripts/fetch_reference_files.sh

set -euo pipefail

DEST="$(cd "$(dirname "$0")/.." && pwd)/test_data/reference"
OPENATFX="https://raw.githubusercontent.com/Jsparrow/org.eclipse.mdm.openatfx.mdf/master/src/test/resources/org/eclipse/mdm/openatfx/mdf"
ASAMMDF="https://raw.githubusercontent.com/danielhrisca/asammdf/master/test/asammdf/gui/resources"
PEAK="https://raw.githubusercontent.com/peak-solution/asam_ods_exd_api_mdf4/main"
# Git LFS content, which only the media host serves as bytes.
CSS="https://media.githubusercontent.com/media/CSS-Electronics/mdf4-converters/master/Tools/SystemTests/Common/TestData"

mkdir -p "$DEST"

VENDOR_PATHS=(
  mdf4/arrays/simple/Vector_ArrayWithFixedAxes.MF4
  mdf4/arrays/simple/Vector_MeasurementArrays.mf4
  mdf4/arrays/simple/dSPACE_MeasurementArrays.mf4
  mdf4/datatypes/can_open_types/Vector_CANOpenDate.mf4
  mdf4/datatypes/can_open_types/Vector_CANOpenTime.mf4
  mdf4/datatypes/string_types/Vector_FixedLengthStringSBC.mf4
  mdf4/datatypes/string_types/Vector_FixedLengthStringUTF8.mf4
  mdf4/datatypes/string_types/Vector_FixedLengthStringUTF16_LE.mf4
  mdf4/datatypes/string_types/Vector_FixedLengthStringUTF16_BE.mf4
  mdf4/datatypes/bytearray/Vector_ByteArrayFixedLength.mf4
  mdf4/datatypes/integer_types/Vector_IntegerTypes.MF4
  mdf4/datatypes/integer_types/dSPACE_IntegerTypes.mf4
  mdf4/datatypes/integer_types/ETAS_IntegerTypes.mf4
  mdf4/datatypes/real_types/Vector_RealTypes.MF4
  mdf4/datatypes/real_types/dSPACE_RealTypes.mf4
  mdf4/conversion/text_conversion/Vector_AlgebraicConversionQuadratic.mf4
  mdf4/conversion/text_conversion/Vector_AlgebraicConversionRational.mf4
  mdf4/conversion/text_conversion/Vector_AlgebraicConversionSinus.mf4
  mdf4/conversion/text_conversion/dSPACE_AlgebraicConversion.mf4
  mdf4/conversion/lookup_conversion/Vector_Value2TextConversion.mf4
  mdf4/conversion/lookup_conversion/Vector_ValueRange2TextConversion.mf4
  mdf4/conversion/lookup_conversion/Vector_Value2ValueConversionInterpolation.mf4
  mdf4/conversion/lookup_conversion/Vector_Value2ValueConversionNoInterpolation.mf4
  mdf4/conversion/lookup_conversion/Vector_ValueRange2ValueConversion.mf4
  mdf4/conversion/lookup_conversion/dSPACE_Value2TextConversion.mf4
  mdf4/conversion/lookup_conversion/dSPACE_Value2ValueConversionInterpolation.mf4
  mdf4/conversion/lookup_conversion/dSPACE_Value2ValueConversionNoInterpolation.mf4
  mdf4/conversion/lookup_conversion/dSPACE_ValueRange2TextConversion.mf4
  mdf4/conversion/string_conversion/Vector_Text2ValueConversion.mf4
  mdf4/conversion/string_conversion/Vector_Text2TextConversion.mf4
  mdf4/conversion/partial_conversion/Vector_PartialConversionLinearIdentityAlgebraic.mf4
  mdf4/conversion/partial_conversion/Vector_PartialConversionValueRange2TextRational.mf4
  mdf4/conversion/partial_conversion/Vector_StatusStringTableConversionAlgebraic.mf4
  mdf4/conversion/rational_conversion/Vector_RationalConversionIntParams.mf4
  mdf4/conversion/rational_conversion/Vector_RationalConversionRealParams.mf4
  mdf4/conversion/rational_conversion/Vector_RationalConversionZeroedParams.mf4
  mdf4/conversion/linear_conversion/Vector_LinearConversion.mf4
  mdf4/conversion/linear_conversion/dSPACE_LinearConversion.mf4
  mdf4/attachments/embedded/Vector_Embedded.MF4
  mdf4/attachments/embeddedcompressed/Vector_EmbeddedCompressed.MF4
  mdf4/attachments/external/Vector_External.MF4
  mdf4/channelinfo/attachmentref/Vector_AttachmentRef.mf4
  mdf4/events/marker/dSPACE_Bookmarks.mf4
  mdf4/events/recording/dSPACE_CaptureBlocks.mf4
  mdf4/events/trigger/dSPACE_HILAPITrigger.mf4
  mdf4/events/trigger/dSPACE_HILAPITimeout.mf4
  mdf4/compressed_data/simple/Vector_SingleDZ_Deflate.mf4
  mdf4/compressed_data/simple/Vector_SingleDZ_TransposeDeflate.mf4
  mdf4/compressed_data/datalist/Vector_DataList_Deflate.mf4
  mdf4/compressed_data/datalist/Vector_DataList_TransposeDeflate.mf4
  mdf4/channelinfo/defaultx/Vector_DefaultX.mf4
  mdf4/metadata/customextensions/Vector_CustomExtensions_CNcomment.mf4
  mdf4/simple/Vector_MinimumFile.MF4
  mdf4/simple/Vector_CANape.MF4
  mdf4/simple/ETAS_SimpleSorted.mf4
)

ASAMMDF_FILES=(
  ASAP2_Demo_V171.mf4
  test_batch.mf4
  test_metadata.mf4
  test_batch_cut_0.mf4
  test_batch_cut_1.mf4
)

# Sample measurements shipped with PEAK's ASAM ODS example plugin.
PEAK_FILES=(
  data/simple.mf4
  data/examples/all_datatypes_test.mf4
  data/examples/asammdf_dimensional_demo.mf4
)

# CSS Electronics' converter test files: unfinalized 4.11 logs, one on LIN, and
# a finalized twin of `multiple.MF4`.
CSS_FILES=(
  single_can_bus_1.MF4
  single_lin_bus_1.MF4
  multiple.MF4
  multiple_fin.MF4
)

ok=0
failed=0

fetch() {
  local url="$1" out="$2"
  if curl -sfL --retry 2 -o "$out" "$url"; then
    # Every MF4 file begins with one of two eight-byte identifiers. Checking it
    # here turns an HTML error page saved under a .mf4 name into a failure now
    # rather than a confusing parse error later.
    local magic
    magic="$(head -c 8 "$out")"
    if [ "$magic" = "MDF     " ] || [ "$magic" = "UnFinMF " ]; then
      ok=$((ok + 1))
      return 0
    fi
    echo "  not an MF4 file: $out" >&2
  fi
  rm -f "$out"
  failed=$((failed + 1))
  echo "  FAILED: $url" >&2
}

echo "Fetching reference files into $DEST"

for path in "${VENDOR_PATHS[@]}"; do
  fetch "$OPENATFX/$path" "$DEST/$(basename "$path")"
done

for name in "${ASAMMDF_FILES[@]}"; do
  fetch "$ASAMMDF/$name" "$DEST/$name"
done

for path in "${PEAK_FILES[@]}"; do
  fetch "$PEAK/$path" "$DEST/$(basename "$path")"
done

for name in "${CSS_FILES[@]}"; do
  fetch "$CSS/$name" "$DEST/$name"
done

echo "$ok file(s) fetched, $failed failed"
[ "$failed" -eq 0 ]
