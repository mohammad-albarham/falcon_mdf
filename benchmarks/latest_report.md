## Performance: falcon_mdf vs asammdf

**Machine**: macOS-26.6.2-arm64-arm-64bit-Mach-O
**Processor**: arm
**Generated**: 2026-08-29T20:33:44+02:00
**Python**: 3.14.7
**asammdf**: 8.7.2
**falcon_mdf**: git ba1e278
**Files tested**: 78

### Summary

| Metric | Value |
|---|---|
| Geometric mean speedup (vs `get()`) | 27.6× |
| Geometric mean speedup (vs `select()`) | 25.5× |
| Median speedup (vs `get()`) | 42.6× |
| Min speedup | 2.9× |
| Max speedup | 64.7× |
| Files where falcon faster | 78/78 |

### Results by File Size

Fixed overhead (asammdf's `MDF()` construction, ~5 ms) dominates the
smallest files, so the aggregate over the whole corpus overstates the
decoding advantage. Quote the `> 1 MB` row.

`Files` counts only files where both libraries decoded the same
number of samples; see Sample-Count Agreement below for the rest.

| Size bucket | Files | Geo. mean vs `get()` | Geo. mean vs `select()` | Worst vs `select()` |
|---|---|---|---|---|
| < 100 KB | 58 | 41.5× | 41.0× | 6.0× |
| 100 KB – 1 MB | 5 | 8.4× | 7.2× | 4.4× |
| > 1 MB | 10 | 5.0× | 3.2× | 1.1× |

### Sample-Count Agreement

falcon and asammdf decoded identical sample counts on **73/78** files.

These files are excluded from the equal-work aggregates above, because a ratio between different amounts of work is not a speedup:

| File | Size | falcon samples | asammdf samples |
|---|---|---|---|
| Vector_ArrayWithFixedAxes.MF4 | 2.2 KB | 49 | 2 |
| dSPACE_MeasurementArrays.mf4 | 6.3 KB | 205 | 20 |
| Vector_MeasurementArrays.mf4 | 12.2 KB | 1,169 | 78 |
| dSPACE_HILAPITimeout.mf4 | 1.0 MB | 50,010 | 25,005 |
| dSPACE_HILAPITrigger.mf4 | 1.0 MB | 50,010 | 25,005 |

### Per-File Results

| File | Size | falcon (s) | asammdf get (s) | asammdf select (s) | Speedup (get) | Speedup (select) |
|---|---|---|---|---|---|---|
| Vector_ByteArrayFixedLength.mf4 | 1.6 KB | 0.0001 | 0.0051 | 0.0049 | 64.1× | 60.9× |
| Vector_CANOpenTime.mf4 | 1.6 KB | 0.0001 | 0.0049 | 0.0051 | 61.2× | 63.4× |
| Vector_CANOpenDate.mf4 | 1.6 KB | 0.0001 | 0.0041 | 0.0049 | 51.5× | 61.4× |
| Vector_FixedLengthStringSBC.mf4 | 1.7 KB | 0.0001 | 0.0049 | 0.0047 | 61.1× | 59.0× |
| Vector_FixedLengthStringUTF8.mf4 | 1.7 KB | 0.0001 | 0.0050 | 0.0049 | 62.2× | 61.5× |
| Vector_FixedLengthStringUTF16_BE.mf4 | 1.7 KB | 0.0001 | 0.0048 | 0.0049 | 53.8× | 54.9× |
| Vector_FixedLengthStringUTF16_LE.mf4 | 1.7 KB | 0.0001 | 0.0051 | 0.0049 | 64.0× | 61.4× |
| video_sync.mf4 | 1.8 KB | 0.0001 | 0.0051 | 0.0049 | 63.7× | 61.0× |
| Vector_LinearConversion.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0050 | 49.2× | 49.7× |
| Vector_Value2TextConversion.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0050 | 54.5× | 55.0× |
| Vector_AlgebraicConversionRational.mf4 | 2.1 KB | 0.0001 | 0.0047 | 0.0050 | 47.1× | 50.4× |
| Vector_AlgebraicConversionSinus.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0049 | 48.8× | 48.6× |
| Vector_RationalConversionZeroedParams.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0048 | 54.3× | 53.5× |
| Vector_RationalConversionRealParams.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0049 | 62.2× | 60.6× |
| Vector_RationalConversionIntParams.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0050 | 55.9× | 56.0× |
| Vector_AlgebraicConversionQuadratic.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0048 | 49.8× | 48.4× |
| Vector_ValueRange2TextConversion.mf4 | 2.1 KB | 0.0001 | 0.0051 | 0.0047 | 56.6× | 52.7× |
| Vector_ArrayWithFixedAxes.MF4 | 2.2 KB | 0.0001 | 0.0048 | 0.0050 | 43.3× | 45.6× |
| Vector_Text2ValueConversion.mf4 | 2.3 KB | 0.0001 | 0.0047 | 0.0049 | 59.2× | 61.4× |
| Vector_Value2ValueConversionInterpolation.mf4 | 2.3 KB | 0.0001 | 0.0049 | 0.0050 | 54.1× | 55.1× |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 2.6 KB | 0.0001 | 0.0052 | 0.0049 | 64.7× | 61.6× |
| Vector_Text2TextConversion.mf4 | 2.6 KB | 0.0001 | 0.0050 | 0.0049 | 55.1× | 54.9× |
| Vector_ValueRange2ValueConversion.mf4 | 2.7 KB | 0.0001 | 0.0049 | 0.0049 | 49.3× | 48.8× |
| dSPACE_LinearConversion.mf4 | 2.8 KB | 0.0001 | 0.0051 | 0.0047 | 51.0× | 47.0× |
| dSPACE_AlgebraicConversion.mf4 | 2.8 KB | 0.0001 | 0.0051 | 0.0052 | 50.5× | 52.1× |
| Vector_AttachmentRef.mf4 | 2.9 KB | 0.0001 | 0.0054 | 0.0052 | 59.6× | 57.8× |
| dSPACE_Value2TextConversion.mf4 | 2.9 KB | 0.0001 | 0.0050 | 0.0050 | 55.5× | 55.2× |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 2.9 KB | 0.0001 | 0.0049 | 0.0050 | 49.2× | 49.8× |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 2.9 KB | 0.0001 | 0.0050 | 0.0050 | 49.7× | 50.0× |
| dSPACE_ValueRange2TextConversion.mf4 | 3.0 KB | 0.0001 | 0.0054 | 0.0050 | 54.5× | 50.3× |
| test_batch_cut_0.mf4 | 3.3 KB | 0.0001 | 0.0051 | 0.0049 | 39.1× | 37.4× |
| Vector_DefaultX.mf4 | 3.4 KB | 0.0001 | 0.0052 | 0.0049 | 57.3× | 54.9× |
| test_batch_cut_1.mf4 | 3.4 KB | 0.0001 | 0.0049 | 0.0052 | 37.4× | 39.9× |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 3.9 KB | 0.0001 | 0.0050 | 0.0051 | 50.3× | 51.1× |
| all_datatypes_test.mf4 | 5.6 KB | 0.0002 | 0.0062 | 0.0050 | 36.3× | 29.5× |
| dSPACE_MeasurementArrays.mf4 | 6.3 KB | 0.0001 | 0.0054 | 0.0051 | 54.3× | 51.1× |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 6.9 KB | 0.0002 | 0.0050 | 0.0052 | 31.5× | 32.4× |
| single_lin_bus_1.MF4 | 7.1 KB | 0.0001 | 0.0014 | 0.0015 | 12.5× | 13.5× |
| single_can_bus_1.MF4 | 7.1 KB | 0.0001 | 0.0016 | 0.0015 | 14.4× | 14.0× |
| test_batch.mf4 | 8.6 KB | 0.0002 | 0.0051 | 0.0050 | 31.6× | 31.2× |
| Vector_RealTypes.MF4 | 9.0 KB | 0.0001 | 0.0058 | 0.0050 | 52.7× | 45.8× |
| simple.mf4 | 9.6 KB | 0.0002 | 0.0051 | 0.0049 | 25.6× | 24.7× |
| Vector_PartialConversionValueRange2TextRational.mf4 | 10.3 KB | 0.0001 | 0.0055 | 0.0055 | 55.5× | 54.8× |
| Vector_MeasurementArrays.mf4 | 12.2 KB | 0.0001 | 0.0058 | N/A | 41.5× | N/A |
| dSPACE_Bookmarks.mf4 | 13.3 KB | 0.0001 | 0.0051 | 0.0056 | 42.5× | 46.6× |
| multiple_fin.MF4 | 13.6 KB | 0.0001 | 0.0055 | 0.0059 | 42.6× | 45.6× |
| multiple.MF4 | 13.9 KB | 0.0001 | 0.0015 | 0.0012 | 9.9× | 8.3× |
| asammdf_dimensional_demo.mf4 | 17.9 KB | 0.0002 | 0.0060 | 0.0061 | 35.3× | 35.9× |
| Vector_IntegerTypes.MF4 | 18.9 KB | 0.0001 | 0.0051 | 0.0058 | 42.1× | 47.9× |
| test_metadata.mf4 | 19.6 KB | 0.0003 | 0.0061 | 0.0060 | 22.7× | 22.3× |
| dSPACE_RealTypes.mf4 | 23.2 KB | 0.0001 | 0.0049 | 0.0056 | 40.7× | 46.7× |
| Vector_MinimumFile.MF4 | 24.2 KB | 0.0001 | 0.0051 | 0.0051 | 46.5× | 46.0× |
| dSPACE_CaptureBlocks.mf4 | 24.6 KB | 0.0001 | 0.0051 | 0.0054 | 38.9× | 41.3× |
| Vector_CANape.MF4 | 25.3 KB | 0.0001 | 0.0055 | 0.0050 | 46.0× | 41.7× |
| Vector_External.MF4 | 27.0 KB | 0.0001 | 0.0057 | 0.0058 | 38.1× | 38.8× |
| Vector_CustomExtensions_CNcomment.mf4 | 27.3 KB | 0.0001 | 0.0050 | 0.0057 | 33.5× | 37.8× |
| Vector_EmbeddedCompressed.MF4 | 28.1 KB | 0.0001 | 0.0050 | 0.0061 | 35.5× | 43.4× |
| Vector_Embedded.MF4 | 28.5 KB | 0.0001 | 0.0059 | 0.0050 | 45.5× | 38.8× |
| dSPACE_IntegerTypes.mf4 | 44.1 KB | 0.0002 | 0.0057 | 0.0049 | 33.8× | 29.1× |
| Vector_SingleDZ_TransposeDeflate.mf4 | 61.2 KB | 0.0008 | 0.0067 | 0.0056 | 8.6× | 7.2× |
| Vector_DataList_TransposeDeflate.mf4 | 68.5 KB | 0.0009 | 0.0077 | 0.0057 | 8.1× | 6.0× |
| Vector_SingleDZ_Deflate.mf4 | 119.0 KB | 0.0011 | 0.0071 | 0.0059 | 6.2× | 5.1× |
| Vector_DataList_Deflate.mf4 | 120.7 KB | 0.0011 | 0.0074 | 0.0058 | 6.5× | 5.1× |
| ETAS_SimpleSorted.mf4 | 209.6 KB | 0.0003 | 0.0052 | 0.0052 | 19.2× | 19.2× |
| 00000012-64BB8F50.MF4 | 658.3 KB | 0.0031 | 0.0298 | 0.0263 | 9.5× | 8.4× |
| ETAS_IntegerTypes.mf4 | 1007.4 KB | 0.0015 | 0.0079 | 0.0064 | 5.5× | 4.4× |
| dSPACE_HILAPITimeout.mf4 | 1.0 MB | 0.0006 | 0.0052 | 0.0059 | 9.0× | 10.2× |
| dSPACE_HILAPITrigger.mf4 | 1.0 MB | 0.0006 | 0.0059 | 0.0059 | 10.3× | 10.3× |
| 00000002.MF4 | 1.0 MB | 0.0028 | 0.0152 | 0.0116 | 5.4× | 4.1× |
| ASAP2_Demo_V171.mf4 | 1.2 MB | 0.0041 | 0.0138 | 0.0111 | 3.4× | 2.7× |
| 00000013-64BB9AA0.MF4 | 1.7 MB | 0.0079 | 0.0475 | 0.0386 | 6.0× | 4.9× |
| 00000014-64BBA8AF.MF4 | 2.1 MB | 0.0102 | 0.0584 | 0.0442 | 5.7× | 4.4× |
| 00002081.MF4 | 5.0 MB | 0.0136 | 0.0769 | 0.0582 | 5.7× | 4.3× |
| 00002082.MF4 | 5.0 MB | 0.0139 | 0.0788 | 0.0578 | 5.7× | 4.1× |
| 00002084.MF4 | 5.0 MB | 0.0135 | 0.0773 | 0.0571 | 5.7× | 4.2× |
| 00002083.MF4 | 5.0 MB | 0.0138 | 0.0779 | 0.0577 | 5.7× | 4.2× |
| large_deflate.mf4 | 121.9 MB | 2.2830 | 11.2988 | 2.4918 | 4.9× | 1.1× |
| large_uncompressed.mf4 | 479.7 MB | 0.6526 | 1.9070 | 0.9477 | 2.9× | 1.5× |

### Memory

| File | falcon RSS (MB) | asammdf RSS (MB) | Ratio |
|---|---|---|---|
| Vector_ByteArrayFixedLength.mf4 | 1.8 | 132.4 | 72.4× |
| Vector_CANOpenTime.mf4 | 1.8 | 132.0 | 72.2× |
| Vector_CANOpenDate.mf4 | 1.8 | 130.5 | 71.4× |
| Vector_FixedLengthStringSBC.mf4 | 1.8 | 132.4 | 72.4× |
| Vector_FixedLengthStringUTF8.mf4 | 1.8 | 130.6 | 71.5× |
| Vector_FixedLengthStringUTF16_BE.mf4 | 1.8 | 131.8 | 72.1× |
| Vector_FixedLengthStringUTF16_LE.mf4 | 1.8 | 130.4 | 71.3× |
| video_sync.mf4 | 1.8 | 132.0 | 72.2× |
| Vector_LinearConversion.mf4 | 1.8 | 131.1 | 72.3× |
| Vector_Value2TextConversion.mf4 | 1.8 | 132.5 | 72.5× |
| Vector_AlgebraicConversionRational.mf4 | 1.8 | 131.5 | 71.3× |
| Vector_AlgebraicConversionSinus.mf4 | 1.8 | 130.8 | 70.9× |
| Vector_RationalConversionZeroedParams.mf4 | 1.8 | 132.1 | 72.9× |
| Vector_RationalConversionRealParams.mf4 | 1.8 | 132.6 | 73.2× |
| Vector_RationalConversionIntParams.mf4 | 1.8 | 130.6 | 72.0× |
| Vector_AlgebraicConversionQuadratic.mf4 | 1.8 | 131.1 | 71.1× |
| Vector_ValueRange2TextConversion.mf4 | 1.8 | 131.9 | 72.1× |
| Vector_ArrayWithFixedAxes.MF4 | 1.8 | 131.4 | 71.3× |
| Vector_Text2ValueConversion.mf4 | 1.8 | 132.6 | 72.5× |
| Vector_Value2ValueConversionInterpolation.mf4 | 1.8 | 131.6 | 72.6× |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 1.8 | 130.9 | 72.2× |
| Vector_Text2TextConversion.mf4 | 1.8 | 131.3 | 71.8× |
| Vector_ValueRange2ValueConversion.mf4 | 1.8 | 130.5 | 72.0× |
| dSPACE_LinearConversion.mf4 | 1.8 | 131.6 | 72.0× |
| dSPACE_AlgebraicConversion.mf4 | 1.9 | 131.7 | 70.8× |
| Vector_AttachmentRef.mf4 | 1.8 | 130.7 | 71.5× |
| dSPACE_Value2TextConversion.mf4 | 1.8 | 130.9 | 71.0× |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 1.8 | 132.5 | 72.5× |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 1.8 | 131.0 | 71.7× |
| dSPACE_ValueRange2TextConversion.mf4 | 1.8 | 131.5 | 71.3× |
| test_batch_cut_0.mf4 | 2.0 | 131.2 | 65.6× |
| Vector_DefaultX.mf4 | 1.8 | 132.1 | 71.7× |
| test_batch_cut_1.mf4 | 2.0 | 132.3 | 65.1× |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 1.8 | 130.9 | 71.0× |
| all_datatypes_test.mf4 | 2.1 | 132.7 | 63.4× |
| dSPACE_MeasurementArrays.mf4 | 1.8 | 131.1 | 71.1× |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 1.9 | 131.1 | 67.7× |
| single_lin_bus_1.MF4 | 1.9 | 133.0 | 69.7× |
| single_can_bus_1.MF4 | 1.9 | 131.9 | 69.2× |
| test_batch.mf4 | 2.1 | 131.5 | 61.4× |
| Vector_RealTypes.MF4 | 1.9 | 131.4 | 70.7× |
| simple.mf4 | 2.2 | 130.7 | 60.6× |
| Vector_PartialConversionValueRange2TextRational.mf4 | 1.9 | 131.6 | 70.8× |
| Vector_MeasurementArrays.mf4 | 2.0 | 132.5 | 67.3× |
| dSPACE_Bookmarks.mf4 | 1.9 | 131.2 | 69.4× |
| multiple_fin.MF4 | 2.0 | 131.6 | 66.8× |
| multiple.MF4 | 2.0 | 133.1 | 66.5× |
| asammdf_dimensional_demo.mf4 | 2.1 | 131.6 | 63.3× |
| Vector_IntegerTypes.MF4 | 2.0 | 130.5 | 66.3× |
| test_metadata.mf4 | 2.3 | 130.7 | 56.5× |
| dSPACE_RealTypes.mf4 | 1.9 | 131.5 | 69.0× |
| Vector_MinimumFile.MF4 | 1.9 | 131.1 | 67.6× |
| dSPACE_CaptureBlocks.mf4 | 1.9 | 132.6 | 69.6× |
| Vector_CANape.MF4 | 1.9 | 130.5 | 67.3× |
| Vector_External.MF4 | 2.0 | 132.7 | 67.9× |
| Vector_CustomExtensions_CNcomment.mf4 | 2.0 | 130.9 | 66.5× |
| Vector_EmbeddedCompressed.MF4 | 2.0 | 132.2 | 66.6× |
| Vector_Embedded.MF4 | 2.0 | 130.6 | 65.8× |
| dSPACE_IntegerTypes.mf4 | 2.1 | 131.3 | 63.7× |
| Vector_SingleDZ_TransposeDeflate.mf4 | 2.7 | 132.3 | 48.4× |
| Vector_DataList_TransposeDeflate.mf4 | 2.5 | 131.7 | 53.0× |
| Vector_SingleDZ_Deflate.mf4 | 3.0 | 131.3 | 44.5× |
| Vector_DataList_Deflate.mf4 | 2.5 | 131.2 | 52.8× |
| ETAS_SimpleSorted.mf4 | 2.4 | 131.3 | 54.6× |
| 00000012-64BB8F50.MF4 | 5.5 | 138.3 | 25.1× |
| ETAS_IntegerTypes.mf4 | 4.2 | 132.9 | 31.6× |
| dSPACE_HILAPITimeout.mf4 | 2.9 | 132.9 | 45.7× |
| dSPACE_HILAPITrigger.mf4 | 2.9 | 131.3 | 45.2× |
| 00000002.MF4 | 7.0 | 141.7 | 20.2× |
| ASAP2_Demo_V171.mf4 | 5.1 | 134.5 | 26.6× |
| 00000013-64BB9AA0.MF4 | 9.8 | 153.1 | 15.6× |
| 00000014-64BBA8AF.MF4 | 12.6 | 165.7 | 13.2× |
| 00002081.MF4 | 31.6 | 180.6 | 5.7× |
| 00002082.MF4 | 28.2 | 184.6 | 6.6× |
| 00002084.MF4 | 27.6 | 180.2 | 6.5× |
| 00002083.MF4 | 29.9 | 174.8 | 5.8× |
| large_deflate.mf4 | 1371.4 | 2339.5 | 1.7× |
| large_uncompressed.mf4 | 1672.0 | 2693.7 | 1.6× |

Both columns are peak resident set size of the whole process, measured with `/usr/bin/time`.
A bare interpreter that only does `import asammdf` already peaks at **127.8 MB**; subtract that to compare decoding cost rather than runtime cost.

### Timing Breakdown

| File | falcon open (ms) | falcon decode (ms) | asammdf open (ms) | asammdf decode (ms) |
|---|---|---|---|---|
| Vector_ByteArrayFixedLength.mf4 | 0.07 | 0.01 | 5.08 | 0.05 |
| Vector_CANOpenTime.mf4 | 0.07 | 0.01 | 4.85 | 0.05 |
| Vector_CANOpenDate.mf4 | 0.07 | 0.01 | 4.04 | 0.10 |
| Vector_FixedLengthStringSBC.mf4 | 0.07 | 0.01 | 4.84 | 0.05 |
| Vector_FixedLengthStringUTF8.mf4 | 0.07 | 0.01 | 4.92 | 0.06 |
| Vector_FixedLengthStringUTF16_BE.mf4 | 0.08 | 0.01 | 4.80 | 0.06 |
| Vector_FixedLengthStringUTF16_LE.mf4 | 0.07 | 0.01 | 5.08 | 0.05 |
| video_sync.mf4 | 0.07 | 0.01 | 4.64 | 0.44 |
| Vector_LinearConversion.mf4 | 0.09 | 0.01 | 4.84 | 0.07 |
| Vector_Value2TextConversion.mf4 | 0.08 | 0.01 | 4.84 | 0.10 |
| Vector_AlgebraicConversionRational.mf4 | 0.09 | 0.01 | 4.63 | 0.08 |
| Vector_AlgebraicConversionSinus.mf4 | 0.09 | 0.01 | 4.79 | 0.08 |
| Vector_RationalConversionZeroedParams.mf4 | 0.08 | 0.01 | 4.82 | 0.07 |
| Vector_RationalConversionRealParams.mf4 | 0.07 | 0.01 | 4.87 | 0.11 |
| Vector_RationalConversionIntParams.mf4 | 0.08 | 0.01 | 4.91 | 0.11 |
| Vector_AlgebraicConversionQuadratic.mf4 | 0.09 | 0.01 | 4.89 | 0.10 |
| Vector_ValueRange2TextConversion.mf4 | 0.08 | 0.01 | 4.96 | 0.09 |
| Vector_ArrayWithFixedAxes.MF4 | 0.10 | 0.01 | 4.66 | 0.14 |
| Vector_Text2ValueConversion.mf4 | 0.07 | 0.01 | 4.65 | 0.08 |
| Vector_Value2ValueConversionInterpolation.mf4 | 0.08 | 0.01 | 4.77 | 0.09 |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 0.07 | 0.01 | 5.00 | 0.13 |
| Vector_Text2TextConversion.mf4 | 0.08 | 0.01 | 4.88 | 0.08 |
| Vector_ValueRange2ValueConversion.mf4 | 0.09 | 0.01 | 4.79 | 0.12 |
| dSPACE_LinearConversion.mf4 | 0.09 | 0.01 | 5.02 | 0.07 |
| dSPACE_AlgebraicConversion.mf4 | 0.09 | 0.01 | 4.95 | 0.09 |
| Vector_AttachmentRef.mf4 | 0.08 | 0.01 | 5.08 | 0.24 |
| dSPACE_Value2TextConversion.mf4 | 0.08 | 0.01 | 4.93 | 0.08 |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 0.09 | 0.01 | 4.87 | 0.08 |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 0.09 | 0.01 | 4.84 | 0.12 |
| dSPACE_ValueRange2TextConversion.mf4 | 0.09 | 0.01 | 5.32 | 0.12 |
| test_batch_cut_0.mf4 | 0.10 | 0.03 | 5.00 | 0.09 |
| Vector_DefaultX.mf4 | 0.08 | 0.01 | 5.04 | 0.10 |
| test_batch_cut_1.mf4 | 0.10 | 0.03 | 4.78 | 0.07 |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 0.09 | 0.01 | 4.84 | 0.18 |
| all_datatypes_test.mf4 | 0.09 | 0.08 | 5.87 | 0.31 |
| dSPACE_MeasurementArrays.mf4 | 0.09 | 0.01 | 5.25 | 0.18 |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 0.09 | 0.07 | 4.82 | 0.22 |
| single_lin_bus_1.MF4 | 0.10 | 0.01 | 0.96 | 0.40 |
| single_can_bus_1.MF4 | 0.10 | 0.01 | 1.10 | 0.47 |
| test_batch.mf4 | 0.05 | 0.11 | 4.74 | 0.29 |
| Vector_RealTypes.MF4 | 0.09 | 0.02 | 5.66 | 0.14 |
| simple.mf4 | 0.08 | 0.12 | 4.45 | 0.65 |
| Vector_PartialConversionValueRange2TextRational.mf4 | 0.08 | 0.02 | 5.17 | 0.38 |
| Vector_MeasurementArrays.mf4 | 0.12 | 0.02 | 5.09 | 0.71 |
| dSPACE_Bookmarks.mf4 | 0.11 | 0.01 | 5.05 | 0.06 |
| multiple_fin.MF4 | 0.10 | 0.03 | 5.06 | 0.49 |
| multiple.MF4 | 0.12 | 0.03 | 1.00 | 0.49 |
| asammdf_dimensional_demo.mf4 | 0.11 | 0.06 | 5.38 | 0.55 |
| Vector_IntegerTypes.MF4 | 0.09 | 0.03 | 4.81 | 0.26 |
| test_metadata.mf4 | 0.20 | 0.07 | 5.33 | 0.66 |
| dSPACE_RealTypes.mf4 | 0.10 | 0.02 | 4.79 | 0.11 |
| Vector_MinimumFile.MF4 | 0.08 | 0.03 | 4.92 | 0.19 |
| dSPACE_CaptureBlocks.mf4 | 0.10 | 0.03 | 4.92 | 0.12 |
| Vector_CANape.MF4 | 0.09 | 0.03 | 5.29 | 0.18 |
| Vector_External.MF4 | 0.12 | 0.03 | 5.44 | 0.20 |
| Vector_CustomExtensions_CNcomment.mf4 | 0.12 | 0.03 | 4.84 | 0.18 |
| Vector_EmbeddedCompressed.MF4 | 0.11 | 0.03 | 4.80 | 0.17 |
| Vector_Embedded.MF4 | 0.10 | 0.03 | 5.70 | 0.22 |
| dSPACE_IntegerTypes.mf4 | 0.12 | 0.05 | 5.47 | 0.27 |
| Vector_SingleDZ_TransposeDeflate.mf4 | 0.08 | 0.70 | 5.20 | 1.60 |
| Vector_DataList_TransposeDeflate.mf4 | 0.08 | 0.87 | 5.43 | 2.34 |
| Vector_SingleDZ_Deflate.mf4 | 0.08 | 1.06 | 5.13 | 1.82 |
| Vector_DataList_Deflate.mf4 | 0.09 | 1.04 | 5.44 | 1.97 |
| ETAS_SimpleSorted.mf4 | 0.09 | 0.18 | 4.84 | 0.35 |
| 00000012-64BB8F50.MF4 | 1.08 | 2.05 | 10.45 | 19.33 |
| ETAS_IntegerTypes.mf4 | 0.11 | 1.34 | 5.75 | 2.26 |
| dSPACE_HILAPITimeout.mf4 | 0.12 | 0.46 | 4.77 | 0.43 |
| dSPACE_HILAPITrigger.mf4 | 0.11 | 0.46 | 5.42 | 0.45 |
| 00000002.MF4 | 1.22 | 1.61 | 7.86 | 7.34 |
| ASAP2_Demo_V171.mf4 | 0.35 | 3.74 | 6.56 | 7.18 |
| 00000013-64BB9AA0.MF4 | 2.15 | 5.79 | 18.91 | 28.40 |
| 00000014-64BBA8AF.MF4 | 2.61 | 7.55 | 24.03 | 33.75 |
| 00002081.MF4 | 5.80 | 7.81 | 37.36 | 39.73 |
| 00002082.MF4 | 5.94 | 8.00 | 38.19 | 40.64 |
| 00002084.MF4 | 5.73 | 7.78 | 37.01 | 40.04 |
| 00002083.MF4 | 5.79 | 7.99 | 37.34 | 40.27 |
| large_deflate.mf4 | 0.27 | 2282.68 | 1129.80 | 10167.92 |
| large_uncompressed.mf4 | 0.29 | 652.34 | 378.21 | 1528.82 |
