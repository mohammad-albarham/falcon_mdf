## Performance: falcon_mdf vs asammdf

**Machine**: macOS-26.6.2-arm64-arm-64bit-Mach-O
**Processor**: arm
**Generated**: 2026-09-06T11:01:11+02:00
**Python**: 3.14.7
**asammdf**: 8.7.2
**falcon_mdf**: git 6d5df00 + uncommitted reader optimizations
**Files tested**: 81

### Summary

| Metric | Value |
|---|---|
| Geometric mean speedup (vs `get()`) | 36.8× |
| Geometric mean speedup (vs `select()`) | 33.8× |
| Median speedup (vs `get()`) | 57.5× |
| Min speedup | 3.4× |
| Max speedup | 105.5× |
| Files where falcon faster | 81/81 |

### Results by File Size

Fixed overhead (asammdf's `MDF()` construction, ~5 ms) dominates the
smallest files, so the aggregate over the whole corpus overstates the
decoding advantage. Quote the `> 1 MB` row.

`Files` counts only files where both libraries decoded the same
number of samples; see Sample-Count Agreement below for the rest.

| Size bucket | Files | Geo. mean vs `get()` | Geo. mean vs `select()` | Worst vs `select()` |
|---|---|---|---|---|
| < 100 KB | 59 | 57.4× | 56.2× | 9.8× |
| 100 KB – 1 MB | 7 | 14.2× | 12.6× | 5.8× |
| > 1 MB | 10 | 5.5× | 3.6× | 1.7× |

### Sample-Count Agreement

falcon and asammdf decoded identical sample counts on **76/81** files.

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
| Vector_ByteArrayFixedLength.mf4 | 1.6 KB | 0.0001 | 0.0050 | 0.0049 | 71.2× | 70.6× |
| Vector_CANOpenTime.mf4 | 1.6 KB | 0.0001 | 0.0051 | 0.0050 | 101.6× | 99.3× |
| Vector_CANOpenDate.mf4 | 1.6 KB | 0.0001 | 0.0051 | 0.0050 | 101.6× | 100.2× |
| Vector_FixedLengthStringUTF16_LE.mf4 | 1.7 KB | 0.0001 | 0.0049 | 0.0050 | 70.6× | 71.4× |
| Vector_FixedLengthStringUTF16_BE.mf4 | 1.7 KB | 0.0001 | 0.0049 | 0.0052 | 82.4× | 86.5× |
| Vector_FixedLengthStringUTF8.mf4 | 1.7 KB | 0.0001 | 0.0049 | 0.0050 | 82.3× | 83.7× |
| Vector_FixedLengthStringSBC.mf4 | 1.7 KB | 0.0001 | 0.0049 | 0.0050 | 81.4× | 82.8× |
| video_sync.mf4 | 1.8 KB | 0.0001 | 0.0049 | 0.0049 | 81.9× | 81.6× |
| Vector_LinearConversion.mf4 | 2.1 KB | 0.0001 | 0.0048 | 0.0048 | 80.5× | 80.4× |
| gen_structure.mf4 | 2.1 KB | 0.0001 | 0.0053 | 0.0050 | 105.5× | 99.5× |
| Vector_Value2TextConversion.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0048 | 69.5× | 69.0× |
| Vector_AlgebraicConversionRational.mf4 | 2.1 KB | 0.0001 | 0.0048 | 0.0049 | 68.8× | 70.0× |
| Vector_AlgebraicConversionSinus.mf4 | 2.1 KB | 0.0001 | 0.0048 | 0.0047 | 68.1× | 66.8× |
| Vector_RationalConversionIntParams.mf4 | 2.1 KB | 0.0001 | 0.0048 | 0.0047 | 79.4× | 77.6× |
| Vector_AlgebraicConversionQuadratic.mf4 | 2.1 KB | 0.0001 | 0.0048 | 0.0051 | 60.0× | 64.1× |
| Vector_RationalConversionZeroedParams.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0041 | 83.7× | 68.3× |
| Vector_RationalConversionRealParams.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0049 | 80.9× | 81.3× |
| Vector_ValueRange2TextConversion.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0051 | 70.9× | 73.2× |
| Vector_ArrayWithFixedAxes.MF4 | 2.2 KB | 0.0001 | 0.0049 | 0.0048 | 69.8× | 69.1× |
| Vector_Text2ValueConversion.mf4 | 2.3 KB | 0.0001 | 0.0046 | 0.0052 | 66.1× | 74.7× |
| Vector_Value2ValueConversionInterpolation.mf4 | 2.3 KB | 0.0001 | 0.0050 | 0.0050 | 83.1× | 83.2× |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 2.6 KB | 0.0001 | 0.0051 | 0.0050 | 72.6× | 71.9× |
| Vector_Text2TextConversion.mf4 | 2.6 KB | 0.0001 | 0.0051 | 0.0049 | 72.6× | 69.4× |
| Vector_ValueRange2ValueConversion.mf4 | 2.7 KB | 0.0001 | 0.0051 | 0.0048 | 101.4× | 95.4× |
| dSPACE_LinearConversion.mf4 | 2.8 KB | 0.0001 | 0.0063 | 0.0049 | 105.2× | 81.9× |
| dSPACE_AlgebraicConversion.mf4 | 2.8 KB | 0.0001 | 0.0048 | 0.0048 | 68.6× | 68.0× |
| Vector_AttachmentRef.mf4 | 2.9 KB | 0.0001 | 0.0053 | 0.0047 | 75.4× | 67.6× |
| dSPACE_Value2TextConversion.mf4 | 2.9 KB | 0.0001 | 0.0051 | 0.0050 | 63.3× | 62.2× |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 2.9 KB | 0.0001 | 0.0051 | 0.0050 | 72.6× | 70.7× |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 2.9 KB | 0.0001 | 0.0049 | 0.0049 | 70.0× | 69.5× |
| dSPACE_ValueRange2TextConversion.mf4 | 3.0 KB | 0.0001 | 0.0051 | 0.0050 | 64.1× | 63.0× |
| test_batch_cut_0.mf4 | 3.3 KB | 0.0001 | 0.0047 | 0.0046 | 59.1× | 57.9× |
| Vector_DefaultX.mf4 | 3.4 KB | 0.0001 | 0.0050 | 0.0050 | 83.6× | 83.0× |
| test_batch_cut_1.mf4 | 3.4 KB | 0.0001 | 0.0046 | 0.0047 | 50.7× | 52.4× |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 3.9 KB | 0.0001 | 0.0050 | 0.0048 | 71.4× | 68.3× |
| all_datatypes_test.mf4 | 5.6 KB | 0.0001 | 0.0049 | 0.0058 | 61.7× | 72.9× |
| dSPACE_MeasurementArrays.mf4 | 6.3 KB | 0.0001 | 0.0048 | 0.0051 | 59.9× | 63.6× |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 6.9 KB | 0.0001 | 0.0049 | 0.0049 | 37.5× | 37.6× |
| single_lin_bus_1.MF4 | 7.1 KB | 0.0001 | 0.0010 | 0.0009 | 11.4× | 10.4× |
| single_can_bus_1.MF4 | 7.1 KB | 0.0001 | 0.0013 | 0.0011 | 15.7× | 13.7× |
| test_batch.mf4 | 8.6 KB | 0.0001 | 0.0048 | 0.0051 | 48.1× | 51.4× |
| Vector_RealTypes.MF4 | 9.0 KB | 0.0001 | 0.0050 | 0.0049 | 71.9× | 69.5× |
| simple.mf4 | 9.6 KB | 0.0001 | 0.0052 | 0.0049 | 39.8× | 37.4× |
| Vector_PartialConversionValueRange2TextRational.mf4 | 10.3 KB | 0.0001 | 0.0049 | 0.0051 | 61.4× | 64.3× |
| Vector_MeasurementArrays.mf4 | 12.2 KB | 0.0001 | 0.0059 | N/A | 49.2× | N/A |
| dSPACE_Bookmarks.mf4 | 13.3 KB | 0.0001 | 0.0049 | 0.0049 | 54.7× | 54.9× |
| multiple_fin.MF4 | 13.6 KB | 0.0001 | 0.0059 | 0.0050 | 59.2× | 50.5× |
| multiple.MF4 | 13.9 KB | 0.0001 | 0.0013 | 0.0010 | 12.6× | 9.8× |
| asammdf_dimensional_demo.mf4 | 17.9 KB | 0.0001 | 0.0060 | 0.0052 | 50.1× | 43.7× |
| Vector_IntegerTypes.MF4 | 18.9 KB | 0.0001 | 0.0048 | 0.0057 | 48.2× | 57.3× |
| test_metadata.mf4 | 19.6 KB | 0.0002 | 0.0063 | 0.0057 | 33.0× | 29.7× |
| dSPACE_RealTypes.mf4 | 23.2 KB | 0.0001 | 0.0049 | 0.0052 | 53.9× | 58.1× |
| Vector_MinimumFile.MF4 | 24.2 KB | 0.0001 | 0.0049 | 0.0052 | 70.1× | 74.0× |
| dSPACE_CaptureBlocks.mf4 | 24.6 KB | 0.0001 | 0.0048 | 0.0049 | 53.6× | 54.1× |
| Vector_CANape.MF4 | 25.3 KB | 0.0001 | 0.0046 | 0.0052 | 57.5× | 65.0× |
| Vector_External.MF4 | 27.0 KB | 0.0001 | 0.0050 | 0.0049 | 45.8× | 44.9× |
| Vector_CustomExtensions_CNcomment.mf4 | 27.3 KB | 0.0001 | 0.0055 | 0.0054 | 55.1× | 53.9× |
| Vector_EmbeddedCompressed.MF4 | 28.1 KB | 0.0001 | 0.0049 | 0.0050 | 44.4× | 45.4× |
| Vector_Embedded.MF4 | 28.5 KB | 0.0001 | 0.0050 | 0.0048 | 50.1× | 47.6× |
| dSPACE_IntegerTypes.mf4 | 44.1 KB | 0.0001 | 0.0049 | 0.0051 | 44.4× | 46.1× |
| Vector_SingleDZ_TransposeDeflate.mf4 | 61.2 KB | 0.0004 | 0.0061 | 0.0050 | 16.5× | 13.4× |
| Vector_DataList_TransposeDeflate.mf4 | 68.5 KB | 0.0005 | 0.0066 | 0.0059 | 12.1× | 10.9× |
| gen_many.mf4 | 101.8 KB | 0.0006 | 0.0097 | 0.0090 | 14.9× | 13.8× |
| Vector_SingleDZ_Deflate.mf4 | 119.0 KB | 0.0006 | 0.0060 | 0.0050 | 9.3× | 7.9× |
| Vector_DataList_Deflate.mf4 | 120.7 KB | 0.0006 | 0.0068 | 0.0054 | 11.3× | 8.9× |
| ETAS_SimpleSorted.mf4 | 209.6 KB | 0.0001 | 0.0051 | 0.0051 | 34.3× | 34.0× |
| gen_xy.mf4 | 235.4 KB | 0.0002 | 0.0049 | 0.0049 | 33.0× | 32.4× |
| 00000012-64BB8F50.MF4 | 658.3 KB | 0.0023 | 0.0216 | 0.0182 | 9.5× | 8.1× |
| ETAS_IntegerTypes.mf4 | 1007.4 KB | 0.0010 | 0.0069 | 0.0059 | 6.8× | 5.8× |
| dSPACE_HILAPITimeout.mf4 | 1.0 MB | 0.0004 | 0.0054 | 0.0051 | 13.4× | 12.7× |
| dSPACE_HILAPITrigger.mf4 | 1.0 MB | 0.0004 | 0.0050 | 0.0049 | 12.4× | 12.4× |
| 00000002.MF4 | 1.0 MB | 0.0021 | 0.0110 | 0.0085 | 5.4× | 4.1× |
| ASAP2_Demo_V171.mf4 | 1.2 MB | 0.0029 | 0.0125 | 0.0102 | 4.4× | 3.6× |
| 00000013-64BB9AA0.MF4 | 1.7 MB | 0.0061 | 0.0350 | 0.0279 | 5.7× | 4.6× |
| 00000014-64BBA8AF.MF4 | 2.1 MB | 0.0076 | 0.0414 | 0.0326 | 5.4× | 4.3× |
| 00002081.MF4 | 5.0 MB | 0.0094 | 0.0576 | 0.0420 | 6.1× | 4.5× |
| 00002082.MF4 | 5.0 MB | 0.0093 | 0.0589 | 0.0422 | 6.3× | 4.5× |
| 00002084.MF4 | 5.0 MB | 0.0094 | 0.0572 | 0.0433 | 6.1× | 4.6× |
| 00002083.MF4 | 5.0 MB | 0.0096 | 0.0570 | 0.0427 | 6.0× | 4.5× |
| large_deflate.mf4 | 121.9 MB | 1.0672 | 8.5043 | 1.8830 | 8.0× | 1.8× |
| large_uncompressed.mf4 | 479.7 MB | 0.4298 | 1.4541 | 0.7374 | 3.4× | 1.7× |

### Memory

| File | falcon RSS (MB) | asammdf RSS (MB) | Ratio |
|---|---|---|---|
| Vector_ByteArrayFixedLength.mf4 | 1.9 | 132.0 | 70.4× |
| Vector_CANOpenTime.mf4 | 1.9 | 131.9 | 70.3× |
| Vector_CANOpenDate.mf4 | 1.9 | 132.0 | 70.4× |
| Vector_FixedLengthStringUTF16_LE.mf4 | 1.9 | 131.8 | 70.3× |
| Vector_FixedLengthStringUTF16_BE.mf4 | 1.9 | 131.9 | 70.4× |
| Vector_FixedLengthStringUTF8.mf4 | 1.9 | 131.2 | 70.0× |
| Vector_FixedLengthStringSBC.mf4 | 1.9 | 131.8 | 70.3× |
| video_sync.mf4 | 1.9 | 131.8 | 70.3× |
| Vector_LinearConversion.mf4 | 1.9 | 132.1 | 70.5× |
| gen_structure.mf4 | 1.9 | 131.9 | 70.9× |
| Vector_Value2TextConversion.mf4 | 1.9 | 132.0 | 69.8× |
| Vector_AlgebraicConversionRational.mf4 | 1.9 | 131.9 | 69.2× |
| Vector_AlgebraicConversionSinus.mf4 | 1.9 | 132.0 | 69.2× |
| Vector_RationalConversionIntParams.mf4 | 1.9 | 132.1 | 70.4× |
| Vector_AlgebraicConversionQuadratic.mf4 | 1.9 | 130.7 | 68.5× |
| Vector_RationalConversionZeroedParams.mf4 | 1.9 | 130.7 | 69.7× |
| Vector_RationalConversionRealParams.mf4 | 1.9 | 132.0 | 70.4× |
| Vector_ValueRange2TextConversion.mf4 | 1.9 | 131.9 | 69.7× |
| Vector_ArrayWithFixedAxes.MF4 | 1.9 | 131.8 | 69.7× |
| Vector_Text2ValueConversion.mf4 | 1.9 | 131.8 | 69.7× |
| Vector_Value2ValueConversionInterpolation.mf4 | 1.9 | 131.9 | 70.3× |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 1.9 | 132.0 | 70.4× |
| Vector_Text2TextConversion.mf4 | 1.9 | 131.9 | 69.8× |
| Vector_ValueRange2ValueConversion.mf4 | 1.9 | 130.9 | 69.8× |
| dSPACE_LinearConversion.mf4 | 1.9 | 132.4 | 70.6× |
| dSPACE_AlgebraicConversion.mf4 | 1.9 | 132.0 | 69.3× |
| Vector_AttachmentRef.mf4 | 1.9 | 132.0 | 70.4× |
| dSPACE_Value2TextConversion.mf4 | 1.9 | 132.1 | 69.9× |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 1.9 | 132.4 | 70.6× |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 1.9 | 132.0 | 70.4× |
| dSPACE_ValueRange2TextConversion.mf4 | 1.9 | 132.4 | 70.0× |
| test_batch_cut_0.mf4 | 2.0 | 131.9 | 67.0× |
| Vector_DefaultX.mf4 | 1.9 | 132.0 | 69.2× |
| test_batch_cut_1.mf4 | 1.9 | 132.0 | 68.7× |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 1.9 | 132.2 | 69.4× |
| all_datatypes_test.mf4 | 2.0 | 131.9 | 64.9× |
| dSPACE_MeasurementArrays.mf4 | 1.9 | 131.9 | 69.8× |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 2.0 | 132.2 | 65.1× |
| single_lin_bus_1.MF4 | 2.0 | 132.1 | 67.7× |
| single_can_bus_1.MF4 | 1.9 | 132.1 | 68.2× |
| test_batch.mf4 | 2.1 | 131.9 | 63.0× |
| Vector_RealTypes.MF4 | 1.9 | 131.9 | 69.2× |
| simple.mf4 | 2.1 | 131.9 | 61.6× |
| Vector_PartialConversionValueRange2TextRational.mf4 | 1.9 | 132.3 | 68.8× |
| Vector_MeasurementArrays.mf4 | 2.1 | 132.0 | 64.0× |
| dSPACE_Bookmarks.mf4 | 1.9 | 132.2 | 68.8× |
| multiple_fin.MF4 | 2.0 | 132.3 | 64.7× |
| multiple.MF4 | 2.0 | 132.4 | 64.7× |
| asammdf_dimensional_demo.mf4 | 2.1 | 132.3 | 62.7× |
| Vector_IntegerTypes.MF4 | 2.0 | 131.9 | 66.0× |
| test_metadata.mf4 | 2.3 | 132.3 | 58.4× |
| dSPACE_RealTypes.mf4 | 2.0 | 132.0 | 67.0× |
| Vector_MinimumFile.MF4 | 2.0 | 131.1 | 65.1× |
| dSPACE_CaptureBlocks.mf4 | 2.0 | 131.8 | 67.5× |
| Vector_CANape.MF4 | 2.0 | 132.0 | 65.5× |
| Vector_External.MF4 | 2.0 | 132.0 | 65.0× |
| Vector_CustomExtensions_CNcomment.mf4 | 2.0 | 131.9 | 64.9× |
| Vector_EmbeddedCompressed.MF4 | 2.0 | 131.9 | 64.5× |
| Vector_Embedded.MF4 | 2.0 | 132.0 | 65.5× |
| dSPACE_IntegerTypes.mf4 | 2.1 | 132.0 | 63.0× |
| Vector_SingleDZ_TransposeDeflate.mf4 | 2.7 | 132.1 | 48.3× |
| Vector_DataList_TransposeDeflate.mf4 | 2.5 | 132.3 | 53.6× |
| gen_many.mf4 | 3.4 | 132.6 | 38.7× |
| Vector_SingleDZ_Deflate.mf4 | 2.8 | 132.5 | 46.8× |
| Vector_DataList_Deflate.mf4 | 2.5 | 132.0 | 53.5× |
| ETAS_SimpleSorted.mf4 | 2.5 | 132.1 | 53.9× |
| gen_xy.mf4 | 2.4 | 131.9 | 54.8× |
| 00000012-64BB8F50.MF4 | 5.5 | 137.7 | 24.9× |
| ETAS_IntegerTypes.mf4 | 4.1 | 133.3 | 32.3× |
| dSPACE_HILAPITimeout.mf4 | 3.0 | 132.1 | 44.5× |
| dSPACE_HILAPITrigger.mf4 | 3.0 | 132.1 | 44.7× |
| 00000002.MF4 | 7.0 | 138.4 | 19.6× |
| ASAP2_Demo_V171.mf4 | 5.2 | 134.1 | 25.9× |
| 00000013-64BB9AA0.MF4 | 9.8 | 151.1 | 15.5× |
| 00000014-64BBA8AF.MF4 | 11.7 | 155.8 | 13.3× |
| 00002081.MF4 | 27.6 | 176.6 | 6.4× |
| 00002082.MF4 | 27.6 | 170.2 | 6.2× |
| 00002084.MF4 | 27.6 | 170.1 | 6.2× |
| 00002083.MF4 | 27.6 | 170.0 | 6.2× |
| large_deflate.mf4 | 1371.4 | 2340.3 | 1.7× |
| large_uncompressed.mf4 | 1672.0 | 2693.0 | 1.6× |

Both columns are peak resident set size of the whole process, measured with `/usr/bin/time`.
A bare interpreter that only does `import asammdf` already peaks at **129.6 MB**; subtract that to compare decoding cost rather than runtime cost.

### Timing Breakdown

| File | falcon open (ms) | falcon decode (ms) | asammdf open (ms) | asammdf decode (ms) |
|---|---|---|---|---|
| Vector_ByteArrayFixedLength.mf4 | 0.06 | 0.01 | 4.95 | 0.04 |
| Vector_CANOpenTime.mf4 | 0.05 | 0.00 | 5.04 | 0.04 |
| Vector_CANOpenDate.mf4 | 0.05 | 0.00 | 4.98 | 0.08 |
| Vector_FixedLengthStringUTF16_LE.mf4 | 0.06 | 0.01 | 4.90 | 0.04 |
| Vector_FixedLengthStringUTF16_BE.mf4 | 0.05 | 0.01 | 4.91 | 0.03 |
| Vector_FixedLengthStringUTF8.mf4 | 0.05 | 0.01 | 4.91 | 0.03 |
| Vector_FixedLengthStringSBC.mf4 | 0.05 | 0.01 | 4.85 | 0.03 |
| video_sync.mf4 | 0.05 | 0.01 | 4.67 | 0.26 |
| Vector_LinearConversion.mf4 | 0.06 | 0.00 | 4.78 | 0.06 |
| gen_structure.mf4 | 0.04 | 0.01 | 5.16 | 0.11 |
| Vector_Value2TextConversion.mf4 | 0.06 | 0.01 | 4.83 | 0.05 |
| Vector_AlgebraicConversionRational.mf4 | 0.06 | 0.01 | 4.76 | 0.06 |
| Vector_AlgebraicConversionSinus.mf4 | 0.06 | 0.01 | 4.72 | 0.06 |
| Vector_RationalConversionIntParams.mf4 | 0.06 | 0.00 | 4.70 | 0.07 |
| Vector_AlgebraicConversionQuadratic.mf4 | 0.07 | 0.01 | 4.76 | 0.06 |
| Vector_RationalConversionZeroedParams.mf4 | 0.06 | 0.00 | 4.93 | 0.13 |
| Vector_RationalConversionRealParams.mf4 | 0.06 | 0.00 | 4.78 | 0.07 |
| Vector_ValueRange2TextConversion.mf4 | 0.06 | 0.01 | 4.90 | 0.05 |
| Vector_ArrayWithFixedAxes.MF4 | 0.06 | 0.01 | 4.79 | 0.10 |
| Vector_Text2ValueConversion.mf4 | 0.06 | 0.01 | 4.58 | 0.07 |
| Vector_Value2ValueConversionInterpolation.mf4 | 0.06 | 0.00 | 4.93 | 0.06 |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 0.06 | 0.01 | 5.03 | 0.08 |
| Vector_Text2TextConversion.mf4 | 0.06 | 0.01 | 5.03 | 0.06 |
| Vector_ValueRange2ValueConversion.mf4 | 0.05 | 0.00 | 5.01 | 0.07 |
| dSPACE_LinearConversion.mf4 | 0.06 | 0.00 | 6.26 | 0.05 |
| dSPACE_AlgebraicConversion.mf4 | 0.06 | 0.01 | 4.74 | 0.06 |
| Vector_AttachmentRef.mf4 | 0.06 | 0.01 | 5.15 | 0.14 |
| dSPACE_Value2TextConversion.mf4 | 0.07 | 0.01 | 5.01 | 0.05 |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 0.06 | 0.01 | 5.04 | 0.05 |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 0.06 | 0.01 | 4.77 | 0.06 |
| dSPACE_ValueRange2TextConversion.mf4 | 0.07 | 0.01 | 5.08 | 0.05 |
| test_batch_cut_0.mf4 | 0.07 | 0.01 | 4.67 | 0.04 |
| Vector_DefaultX.mf4 | 0.05 | 0.01 | 4.95 | 0.07 |
| test_batch_cut_1.mf4 | 0.08 | 0.01 | 4.50 | 0.06 |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 0.06 | 0.01 | 4.87 | 0.13 |
| all_datatypes_test.mf4 | 0.06 | 0.02 | 4.76 | 0.19 |
| dSPACE_MeasurementArrays.mf4 | 0.07 | 0.01 | 4.69 | 0.12 |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 0.07 | 0.06 | 4.75 | 0.14 |
| single_lin_bus_1.MF4 | 0.08 | 0.01 | 0.72 | 0.30 |
| single_can_bus_1.MF4 | 0.07 | 0.01 | 0.85 | 0.33 |
| test_batch.mf4 | 0.04 | 0.06 | 4.62 | 0.19 |
| Vector_RealTypes.MF4 | 0.06 | 0.01 | 4.93 | 0.10 |
| simple.mf4 | 0.06 | 0.07 | 4.73 | 0.46 |
| Vector_PartialConversionValueRange2TextRational.mf4 | 0.06 | 0.02 | 4.69 | 0.24 |
| Vector_MeasurementArrays.mf4 | 0.10 | 0.02 | 5.43 | 0.48 |
| dSPACE_Bookmarks.mf4 | 0.08 | 0.01 | 4.89 | 0.05 |
| multiple_fin.MF4 | 0.08 | 0.02 | 5.56 | 0.35 |
| multiple.MF4 | 0.08 | 0.02 | 0.88 | 0.38 |
| asammdf_dimensional_demo.mf4 | 0.08 | 0.04 | 5.62 | 0.40 |
| Vector_IntegerTypes.MF4 | 0.08 | 0.02 | 4.64 | 0.19 |
| test_metadata.mf4 | 0.16 | 0.03 | 5.82 | 0.45 |
| dSPACE_RealTypes.mf4 | 0.07 | 0.02 | 4.76 | 0.08 |
| Vector_MinimumFile.MF4 | 0.05 | 0.02 | 4.76 | 0.12 |
| dSPACE_CaptureBlocks.mf4 | 0.07 | 0.02 | 4.73 | 0.09 |
| Vector_CANape.MF4 | 0.06 | 0.02 | 4.46 | 0.14 |
| Vector_External.MF4 | 0.09 | 0.02 | 4.90 | 0.15 |
| Vector_CustomExtensions_CNcomment.mf4 | 0.08 | 0.02 | 5.37 | 0.14 |
| Vector_EmbeddedCompressed.MF4 | 0.09 | 0.02 | 4.75 | 0.14 |
| Vector_Embedded.MF4 | 0.08 | 0.02 | 4.88 | 0.13 |
| dSPACE_IntegerTypes.mf4 | 0.08 | 0.03 | 4.70 | 0.19 |
| Vector_SingleDZ_TransposeDeflate.mf4 | 0.05 | 0.32 | 4.93 | 1.18 |
| Vector_DataList_TransposeDeflate.mf4 | 0.06 | 0.48 | 4.90 | 1.73 |
| gen_many.mf4 | 0.59 | 0.06 | 6.26 | 3.31 |
| Vector_SingleDZ_Deflate.mf4 | 0.06 | 0.58 | 4.74 | 1.30 |
| Vector_DataList_Deflate.mf4 | 0.06 | 0.54 | 5.35 | 1.45 |
| ETAS_SimpleSorted.mf4 | 0.06 | 0.09 | 4.88 | 0.26 |
| gen_xy.mf4 | 0.04 | 0.11 | 4.66 | 0.29 |
| 00000012-64BB8F50.MF4 | 0.77 | 1.49 | 7.85 | 13.74 |
| ETAS_IntegerTypes.mf4 | 0.08 | 0.93 | 5.21 | 1.69 |
| dSPACE_HILAPITimeout.mf4 | 0.08 | 0.32 | 5.04 | 0.33 |
| dSPACE_HILAPITrigger.mf4 | 0.08 | 0.32 | 4.64 | 0.32 |
| 00000002.MF4 | 0.88 | 1.17 | 5.41 | 5.53 |
| ASAP2_Demo_V171.mf4 | 0.25 | 2.61 | 7.30 | 5.46 |
| 00000013-64BB9AA0.MF4 | 1.86 | 4.26 | 13.73 | 21.19 |
| 00000014-64BBA8AF.MF4 | 1.94 | 5.66 | 17.28 | 24.19 |
| 00002081.MF4 | 4.23 | 5.18 | 28.80 | 28.85 |
| 00002082.MF4 | 4.17 | 5.12 | 29.26 | 29.62 |
| 00002084.MF4 | 4.10 | 5.32 | 27.92 | 29.02 |
| 00002083.MF4 | 4.13 | 5.45 | 27.76 | 29.28 |
| large_deflate.mf4 | 0.19 | 1066.99 | 852.30 | 7649.23 |
| large_uncompressed.mf4 | 0.22 | 429.62 | 290.72 | 1163.39 |
