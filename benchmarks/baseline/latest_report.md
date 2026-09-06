## Performance: falcon_mdf vs asammdf

**Machine**: macOS-26.6.2-arm64-arm-64bit-Mach-O
**Processor**: arm
**Generated**: 2026-09-06T10:43:27+02:00
**Python**: 3.14.7
**asammdf**: 8.7.2
**falcon_mdf**: git 6d5df00
**Files tested**: 81

### Summary

| Metric | Value |
|---|---|
| Geometric mean speedup (vs `get()`) | 35.1× |
| Geometric mean speedup (vs `select()`) | 32.1× |
| Median speedup (vs `get()`) | 56.3× |
| Min speedup | 3.0× |
| Max speedup | 123.8× |
| Files where falcon faster | 81/81 |

### Results by File Size

Fixed overhead (asammdf's `MDF()` construction, ~5 ms) dominates the
smallest files, so the aggregate over the whole corpus overstates the
decoding advantage. Quote the `> 1 MB` row.

`Files` counts only files where both libraries decoded the same
number of samples; see Sample-Count Agreement below for the rest.

| Size bucket | Files | Geo. mean vs `get()` | Geo. mean vs `select()` | Worst vs `select()` |
|---|---|---|---|---|
| < 100 KB | 59 | 55.9× | 54.9× | 7.4× |
| 100 KB – 1 MB | 7 | 11.6× | 9.8× | 5.0× |
| > 1 MB | 10 | 5.1× | 3.3× | 1.1× |

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
| Vector_ByteArrayFixedLength.mf4 | 1.6 KB | 0.0001 | 0.0065 | 0.0050 | 59.4× | 45.5× |
| Vector_CANOpenTime.mf4 | 1.6 KB | 0.0000 | 0.0050 | 0.0049 | 123.8× | 123.6× |
| Vector_CANOpenDate.mf4 | 1.6 KB | 0.0001 | 0.0049 | 0.0049 | 97.6× | 98.3× |
| Vector_FixedLengthStringUTF16_BE.mf4 | 1.7 KB | 0.0001 | 0.0049 | 0.0050 | 81.7× | 83.5× |
| Vector_FixedLengthStringSBC.mf4 | 1.7 KB | 0.0001 | 0.0051 | 0.0052 | 84.3× | 87.1× |
| Vector_FixedLengthStringUTF8.mf4 | 1.7 KB | 0.0001 | 0.0049 | 0.0050 | 81.7× | 82.5× |
| Vector_FixedLengthStringUTF16_LE.mf4 | 1.7 KB | 0.0001 | 0.0050 | 0.0050 | 83.9× | 83.5× |
| video_sync.mf4 | 1.8 KB | 0.0001 | 0.0050 | 0.0049 | 71.9× | 70.1× |
| Vector_LinearConversion.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0049 | 83.1× | 81.9× |
| gen_structure.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0049 | 98.7× | 98.3× |
| Vector_Value2TextConversion.mf4 | 2.1 KB | 0.0001 | 0.0052 | 0.0050 | 73.6× | 70.8× |
| Vector_AlgebraicConversionRational.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0049 | 71.2× | 69.9× |
| Vector_AlgebraicConversionSinus.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0050 | 70.4× | 71.2× |
| Vector_RationalConversionZeroedParams.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0050 | 82.9× | 83.6× |
| Vector_RationalConversionIntParams.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0049 | 82.1× | 82.2× |
| Vector_ValueRange2TextConversion.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0052 | 82.3× | 85.9× |
| Vector_RationalConversionRealParams.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0050 | 70.1× | 72.1× |
| Vector_AlgebraicConversionQuadratic.mf4 | 2.1 KB | 0.0001 | 0.0051 | 0.0051 | 72.4× | 72.9× |
| Vector_ArrayWithFixedAxes.MF4 | 2.2 KB | 0.0001 | 0.0048 | 0.0049 | 59.8× | 61.6× |
| Vector_Text2ValueConversion.mf4 | 2.3 KB | 0.0001 | 0.0049 | 0.0051 | 70.3× | 72.8× |
| Vector_Value2ValueConversionInterpolation.mf4 | 2.3 KB | 0.0001 | 0.0048 | 0.0049 | 80.3× | 81.8× |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 2.6 KB | 0.0001 | 0.0050 | 0.0051 | 82.9× | 84.7× |
| Vector_Text2TextConversion.mf4 | 2.6 KB | 0.0001 | 0.0049 | 0.0051 | 70.1× | 72.7× |
| Vector_ValueRange2ValueConversion.mf4 | 2.7 KB | 0.0001 | 0.0049 | 0.0049 | 70.1× | 70.5× |
| dSPACE_LinearConversion.mf4 | 2.8 KB | 0.0001 | 0.0052 | 0.0050 | 86.6× | 84.1× |
| dSPACE_AlgebraicConversion.mf4 | 2.8 KB | 0.0001 | 0.0050 | 0.0049 | 63.0× | 61.5× |
| Vector_AttachmentRef.mf4 | 2.9 KB | 0.0001 | 0.0049 | 0.0049 | 70.4× | 69.4× |
| dSPACE_Value2TextConversion.mf4 | 2.9 KB | 0.0001 | 0.0048 | 0.0047 | 68.8× | 67.6× |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 2.9 KB | 0.0001 | 0.0050 | 0.0050 | 62.7× | 62.4× |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 2.9 KB | 0.0001 | 0.0048 | 0.0049 | 68.4× | 70.4× |
| dSPACE_ValueRange2TextConversion.mf4 | 3.0 KB | 0.0001 | 0.0050 | 0.0050 | 70.8× | 71.0× |
| test_batch_cut_0.mf4 | 3.3 KB | 0.0001 | 0.0050 | 0.0050 | 55.2× | 55.9× |
| Vector_DefaultX.mf4 | 3.4 KB | 0.0001 | 0.0050 | 0.0051 | 71.1× | 72.5× |
| test_batch_cut_1.mf4 | 3.4 KB | 0.0001 | 0.0051 | 0.0050 | 56.3× | 55.8× |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 3.9 KB | 0.0001 | 0.0049 | 0.0049 | 70.7× | 69.3× |
| all_datatypes_test.mf4 | 5.6 KB | 0.0001 | 0.0050 | 0.0060 | 41.7× | 50.0× |
| dSPACE_MeasurementArrays.mf4 | 6.3 KB | 0.0001 | 0.0050 | 0.0050 | 62.3× | 61.9× |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 6.9 KB | 0.0001 | 0.0048 | 0.0049 | 43.7× | 44.3× |
| single_lin_bus_1.MF4 | 7.1 KB | 0.0001 | 0.0010 | 0.0009 | 12.4× | 11.2× |
| single_can_bus_1.MF4 | 7.1 KB | 0.0001 | 0.0010 | 0.0009 | 11.5× | 10.0× |
| test_batch.mf4 | 8.6 KB | 0.0001 | 0.0050 | 0.0049 | 45.3× | 44.7× |
| Vector_RealTypes.MF4 | 9.0 KB | 0.0001 | 0.0049 | 0.0050 | 70.7× | 70.8× |
| simple.mf4 | 9.6 KB | 0.0001 | 0.0050 | 0.0049 | 35.8× | 35.3× |
| Vector_PartialConversionValueRange2TextRational.mf4 | 10.3 KB | 0.0001 | 0.0060 | 0.0048 | 85.0× | 68.7× |
| Vector_MeasurementArrays.mf4 | 12.2 KB | 0.0001 | 0.0080 | N/A | 73.1× | N/A |
| dSPACE_Bookmarks.mf4 | 13.3 KB | 0.0001 | 0.0049 | 0.0050 | 61.2× | 62.0× |
| multiple_fin.MF4 | 13.6 KB | 0.0001 | 0.0052 | 0.0049 | 51.8× | 49.1× |
| multiple.MF4 | 13.9 KB | 0.0001 | 0.0012 | 0.0010 | 12.1× | 9.5× |
| asammdf_dimensional_demo.mf4 | 17.9 KB | 0.0001 | 0.0049 | 0.0052 | 40.8× | 43.1× |
| Vector_IntegerTypes.MF4 | 18.9 KB | 0.0001 | 0.0049 | 0.0050 | 54.7× | 56.1× |
| test_metadata.mf4 | 19.6 KB | 0.0002 | 0.0053 | 0.0057 | 28.1× | 29.9× |
| dSPACE_RealTypes.mf4 | 23.2 KB | 0.0001 | 0.0048 | 0.0049 | 53.0× | 54.1× |
| Vector_MinimumFile.MF4 | 24.2 KB | 0.0001 | 0.0050 | 0.0049 | 70.8× | 70.4× |
| dSPACE_CaptureBlocks.mf4 | 24.6 KB | 0.0001 | 0.0049 | 0.0048 | 54.7× | 53.8× |
| Vector_CANape.MF4 | 25.3 KB | 0.0001 | 0.0049 | 0.0053 | 61.4× | 66.2× |
| Vector_External.MF4 | 27.0 KB | 0.0001 | 0.0049 | 0.0050 | 48.7× | 49.8× |
| Vector_CustomExtensions_CNcomment.mf4 | 27.3 KB | 0.0001 | 0.0049 | 0.0048 | 54.7× | 53.2× |
| Vector_EmbeddedCompressed.MF4 | 28.1 KB | 0.0001 | 0.0050 | 0.0049 | 45.3× | 44.5× |
| Vector_Embedded.MF4 | 28.5 KB | 0.0001 | 0.0049 | 0.0049 | 49.2× | 48.8× |
| dSPACE_IntegerTypes.mf4 | 44.1 KB | 0.0001 | 0.0051 | 0.0046 | 45.9× | 41.5× |
| Vector_SingleDZ_TransposeDeflate.mf4 | 61.2 KB | 0.0006 | 0.0059 | 0.0054 | 9.9× | 9.0× |
| Vector_DataList_TransposeDeflate.mf4 | 68.5 KB | 0.0007 | 0.0067 | 0.0051 | 9.7× | 7.4× |
| gen_many.mf4 | 101.8 KB | 0.0007 | 0.0099 | 0.0079 | 13.3× | 10.6× |
| Vector_SingleDZ_Deflate.mf4 | 119.0 KB | 0.0008 | 0.0067 | 0.0048 | 8.2× | 6.0× |
| Vector_DataList_Deflate.mf4 | 120.7 KB | 0.0008 | 0.0059 | 0.0051 | 7.0× | 6.1× |
| ETAS_SimpleSorted.mf4 | 209.6 KB | 0.0002 | 0.0049 | 0.0049 | 26.0× | 25.6× |
| gen_xy.mf4 | 235.4 KB | 0.0002 | 0.0048 | 0.0050 | 23.0× | 23.8× |
| 00000012-64BB8F50.MF4 | 658.3 KB | 0.0024 | 0.0234 | 0.0185 | 9.6× | 7.5× |
| ETAS_IntegerTypes.mf4 | 1007.4 KB | 0.0011 | 0.0070 | 0.0055 | 6.3× | 5.0× |
| dSPACE_HILAPITimeout.mf4 | 1.0 MB | 0.0004 | 0.0056 | 0.0049 | 13.0× | 11.4× |
| dSPACE_HILAPITrigger.mf4 | 1.0 MB | 0.0004 | 0.0049 | 0.0050 | 11.9× | 12.2× |
| 00000002.MF4 | 1.0 MB | 0.0021 | 0.0113 | 0.0084 | 5.3× | 3.9× |
| ASAP2_Demo_V171.mf4 | 1.2 MB | 0.0030 | 0.0119 | 0.0093 | 4.0× | 3.1× |
| 00000013-64BB9AA0.MF4 | 1.7 MB | 0.0059 | 0.0346 | 0.0275 | 5.9× | 4.7× |
| 00000014-64BBA8AF.MF4 | 2.1 MB | 0.0074 | 0.0413 | 0.0325 | 5.6× | 4.4× |
| 00002081.MF4 | 5.0 MB | 0.0101 | 0.0569 | 0.0418 | 5.7× | 4.2× |
| 00002082.MF4 | 5.0 MB | 0.0101 | 0.0590 | 0.0428 | 5.9× | 4.3× |
| 00002083.MF4 | 5.0 MB | 0.0095 | 0.0565 | 0.0419 | 5.9× | 4.4× |
| 00002084.MF4 | 5.0 MB | 0.0096 | 0.0566 | 0.0440 | 5.9× | 4.6× |
| large_deflate.mf4 | 121.9 MB | 1.7209 | 8.4360 | 1.8753 | 4.9× | 1.1× |
| large_uncompressed.mf4 | 479.7 MB | 0.4883 | 1.4534 | 0.7272 | 3.0× | 1.5× |

### Memory

| File | falcon RSS (MB) | asammdf RSS (MB) | Ratio |
|---|---|---|---|
| Vector_ByteArrayFixedLength.mf4 | 1.9 | 132.3 | 71.2× |
| Vector_CANOpenTime.mf4 | 1.9 | 132.0 | 71.0× |
| Vector_CANOpenDate.mf4 | 1.9 | 132.0 | 71.0× |
| Vector_FixedLengthStringUTF16_BE.mf4 | 1.9 | 131.9 | 70.9× |
| Vector_FixedLengthStringSBC.mf4 | 1.9 | 131.8 | 70.9× |
| Vector_FixedLengthStringUTF8.mf4 | 1.9 | 131.9 | 70.9× |
| Vector_FixedLengthStringUTF16_LE.mf4 | 1.9 | 132.2 | 71.1× |
| video_sync.mf4 | 1.9 | 132.0 | 71.0× |
| Vector_LinearConversion.mf4 | 1.9 | 131.5 | 70.7× |
| gen_structure.mf4 | 1.8 | 130.8 | 70.9× |
| Vector_Value2TextConversion.mf4 | 1.9 | 132.4 | 70.6× |
| Vector_AlgebraicConversionRational.mf4 | 1.9 | 132.1 | 69.9× |
| Vector_AlgebraicConversionSinus.mf4 | 1.9 | 131.9 | 69.8× |
| Vector_RationalConversionZeroedParams.mf4 | 1.9 | 132.5 | 71.3× |
| Vector_RationalConversionIntParams.mf4 | 1.9 | 132.0 | 71.0× |
| Vector_ValueRange2TextConversion.mf4 | 1.9 | 131.8 | 70.3× |
| Vector_RationalConversionRealParams.mf4 | 1.9 | 132.0 | 71.0× |
| Vector_AlgebraicConversionQuadratic.mf4 | 1.9 | 131.8 | 69.7× |
| Vector_ArrayWithFixedAxes.MF4 | 1.9 | 130.6 | 69.1× |
| Vector_Text2ValueConversion.mf4 | 1.9 | 131.9 | 70.3× |
| Vector_Value2ValueConversionInterpolation.mf4 | 1.9 | 132.4 | 71.2× |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 1.9 | 132.0 | 71.0× |
| Vector_Text2TextConversion.mf4 | 1.9 | 131.8 | 69.7× |
| Vector_ValueRange2ValueConversion.mf4 | 1.9 | 132.0 | 71.0× |
| dSPACE_LinearConversion.mf4 | 1.9 | 132.0 | 71.0× |
| dSPACE_AlgebraicConversion.mf4 | 1.9 | 131.2 | 69.4× |
| Vector_AttachmentRef.mf4 | 1.9 | 132.5 | 71.3× |
| dSPACE_Value2TextConversion.mf4 | 1.9 | 132.3 | 70.6× |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 1.9 | 132.3 | 71.2× |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 1.9 | 132.1 | 71.0× |
| dSPACE_ValueRange2TextConversion.mf4 | 1.9 | 131.9 | 70.3× |
| test_batch_cut_0.mf4 | 2.0 | 132.0 | 65.5× |
| Vector_DefaultX.mf4 | 1.9 | 131.8 | 70.3× |
| test_batch_cut_1.mf4 | 2.0 | 131.8 | 65.4× |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 1.9 | 132.6 | 70.1× |
| all_datatypes_test.mf4 | 2.1 | 132.4 | 62.8× |
| dSPACE_MeasurementArrays.mf4 | 1.9 | 132.0 | 70.4× |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 2.0 | 132.1 | 67.1× |
| single_lin_bus_1.MF4 | 1.9 | 132.0 | 68.7× |
| single_can_bus_1.MF4 | 1.9 | 132.4 | 68.9× |
| test_batch.mf4 | 2.1 | 131.9 | 63.0× |
| Vector_RealTypes.MF4 | 1.9 | 131.9 | 69.8× |
| simple.mf4 | 2.2 | 132.0 | 61.2× |
| Vector_PartialConversionValueRange2TextRational.mf4 | 1.9 | 132.2 | 69.4× |
| Vector_MeasurementArrays.mf4 | 2.1 | 132.5 | 63.7× |
| dSPACE_Bookmarks.mf4 | 1.9 | 131.8 | 68.6× |
| multiple_fin.MF4 | 2.0 | 132.2 | 66.6× |
| multiple.MF4 | 2.0 | 132.4 | 65.7× |
| asammdf_dimensional_demo.mf4 | 2.1 | 132.4 | 63.7× |
| Vector_IntegerTypes.MF4 | 1.9 | 131.9 | 68.1× |
| test_metadata.mf4 | 2.3 | 132.2 | 56.8× |
| dSPACE_RealTypes.mf4 | 1.9 | 132.5 | 68.4× |
| Vector_MinimumFile.MF4 | 2.0 | 132.0 | 67.0× |
| dSPACE_CaptureBlocks.mf4 | 1.9 | 131.8 | 68.6× |
| Vector_CANape.MF4 | 2.0 | 132.0 | 67.6× |
| Vector_External.MF4 | 2.0 | 131.9 | 64.9× |
| Vector_CustomExtensions_CNcomment.mf4 | 2.0 | 131.9 | 67.0× |
| Vector_EmbeddedCompressed.MF4 | 2.0 | 132.0 | 65.5× |
| Vector_Embedded.MF4 | 2.0 | 132.3 | 65.1× |
| dSPACE_IntegerTypes.mf4 | 2.1 | 132.0 | 63.5× |
| Vector_SingleDZ_TransposeDeflate.mf4 | 2.8 | 131.8 | 47.9× |
| Vector_DataList_TransposeDeflate.mf4 | 2.5 | 131.4 | 52.5× |
| gen_many.mf4 | 3.5 | 132.6 | 38.4× |
| Vector_SingleDZ_Deflate.mf4 | 2.9 | 132.2 | 46.0× |
| Vector_DataList_Deflate.mf4 | 2.5 | 132.4 | 53.0× |
| ETAS_SimpleSorted.mf4 | 2.4 | 132.3 | 54.3× |
| gen_xy.mf4 | 2.4 | 131.7 | 55.1× |
| 00000012-64BB8F50.MF4 | 5.5 | 137.9 | 25.1× |
| ETAS_IntegerTypes.mf4 | 4.1 | 133.0 | 32.6× |
| dSPACE_HILAPITimeout.mf4 | 2.9 | 132.2 | 45.0× |
| dSPACE_HILAPITrigger.mf4 | 2.9 | 132.2 | 45.0× |
| 00000002.MF4 | 7.0 | 138.6 | 19.8× |
| ASAP2_Demo_V171.mf4 | 5.1 | 134.2 | 26.3× |
| 00000013-64BB9AA0.MF4 | 9.8 | 148.7 | 15.2× |
| 00000014-64BBA8AF.MF4 | 11.7 | 155.8 | 13.3× |
| 00002081.MF4 | 27.6 | 170.3 | 6.2× |
| 00002082.MF4 | 27.6 | 169.9 | 6.2× |
| 00002083.MF4 | 27.6 | 170.0 | 6.2× |
| 00002084.MF4 | 27.6 | 169.9 | 6.2× |
| large_deflate.mf4 | 1371.4 | 2340.3 | 1.7× |
| large_uncompressed.mf4 | 1672.0 | 2693.2 | 1.6× |

Both columns are peak resident set size of the whole process, measured with `/usr/bin/time`.
A bare interpreter that only does `import asammdf` already peaks at **129.1 MB**; subtract that to compare decoding cost rather than runtime cost.

### Timing Breakdown

| File | falcon open (ms) | falcon decode (ms) | asammdf open (ms) | asammdf decode (ms) |
|---|---|---|---|---|
| Vector_ByteArrayFixedLength.mf4 | 0.10 | 0.01 | 6.43 | 0.10 |
| Vector_CANOpenTime.mf4 | 0.04 | 0.00 | 4.91 | 0.04 |
| Vector_CANOpenDate.mf4 | 0.05 | 0.00 | 4.80 | 0.09 |
| Vector_FixedLengthStringUTF16_BE.mf4 | 0.05 | 0.01 | 4.85 | 0.04 |
| Vector_FixedLengthStringSBC.mf4 | 0.05 | 0.01 | 5.02 | 0.04 |
| Vector_FixedLengthStringUTF8.mf4 | 0.05 | 0.01 | 4.85 | 0.05 |
| Vector_FixedLengthStringUTF16_LE.mf4 | 0.05 | 0.01 | 4.98 | 0.04 |
| video_sync.mf4 | 0.06 | 0.01 | 4.77 | 0.28 |
| Vector_LinearConversion.mf4 | 0.06 | 0.00 | 4.93 | 0.06 |
| gen_structure.mf4 | 0.04 | 0.01 | 4.86 | 0.08 |
| Vector_Value2TextConversion.mf4 | 0.06 | 0.01 | 5.08 | 0.08 |
| Vector_AlgebraicConversionRational.mf4 | 0.06 | 0.01 | 4.91 | 0.07 |
| Vector_AlgebraicConversionSinus.mf4 | 0.06 | 0.01 | 4.85 | 0.07 |
| Vector_RationalConversionZeroedParams.mf4 | 0.06 | 0.00 | 4.92 | 0.08 |
| Vector_RationalConversionIntParams.mf4 | 0.06 | 0.00 | 4.84 | 0.08 |
| Vector_ValueRange2TextConversion.mf4 | 0.05 | 0.01 | 4.88 | 0.07 |
| Vector_RationalConversionRealParams.mf4 | 0.06 | 0.01 | 4.84 | 0.08 |
| Vector_AlgebraicConversionQuadratic.mf4 | 0.06 | 0.01 | 5.00 | 0.07 |
| Vector_ArrayWithFixedAxes.MF4 | 0.07 | 0.01 | 4.72 | 0.09 |
| Vector_Text2ValueConversion.mf4 | 0.06 | 0.01 | 4.87 | 0.06 |
| Vector_Value2ValueConversionInterpolation.mf4 | 0.05 | 0.01 | 4.74 | 0.06 |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 0.06 | 0.00 | 4.91 | 0.10 |
| Vector_Text2TextConversion.mf4 | 0.06 | 0.01 | 4.85 | 0.06 |
| Vector_ValueRange2ValueConversion.mf4 | 0.06 | 0.01 | 4.84 | 0.07 |
| dSPACE_LinearConversion.mf4 | 0.06 | 0.00 | 5.14 | 0.06 |
| dSPACE_AlgebraicConversion.mf4 | 0.07 | 0.01 | 4.97 | 0.07 |
| Vector_AttachmentRef.mf4 | 0.06 | 0.01 | 4.73 | 0.20 |
| dSPACE_Value2TextConversion.mf4 | 0.06 | 0.01 | 4.75 | 0.06 |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 0.07 | 0.01 | 4.97 | 0.06 |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 0.06 | 0.01 | 4.72 | 0.07 |
| dSPACE_ValueRange2TextConversion.mf4 | 0.06 | 0.01 | 4.91 | 0.06 |
| test_batch_cut_0.mf4 | 0.07 | 0.02 | 4.90 | 0.05 |
| Vector_DefaultX.mf4 | 0.06 | 0.01 | 4.90 | 0.08 |
| test_batch_cut_1.mf4 | 0.07 | 0.02 | 5.00 | 0.06 |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 0.06 | 0.01 | 4.79 | 0.17 |
| all_datatypes_test.mf4 | 0.06 | 0.06 | 4.77 | 0.21 |
| dSPACE_MeasurementArrays.mf4 | 0.07 | 0.01 | 4.86 | 0.14 |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 0.06 | 0.05 | 4.65 | 0.16 |
| single_lin_bus_1.MF4 | 0.07 | 0.01 | 0.71 | 0.31 |
| single_can_bus_1.MF4 | 0.08 | 0.01 | 0.72 | 0.31 |
| test_batch.mf4 | 0.04 | 0.07 | 4.75 | 0.22 |
| Vector_RealTypes.MF4 | 0.06 | 0.01 | 4.82 | 0.11 |
| simple.mf4 | 0.05 | 0.09 | 4.55 | 0.48 |
| Vector_PartialConversionValueRange2TextRational.mf4 | 0.06 | 0.01 | 5.54 | 0.35 |
| Vector_MeasurementArrays.mf4 | 0.09 | 0.02 | 7.53 | 0.52 |
| dSPACE_Bookmarks.mf4 | 0.07 | 0.01 | 4.85 | 0.05 |
| multiple_fin.MF4 | 0.08 | 0.02 | 4.83 | 0.34 |
| multiple.MF4 | 0.08 | 0.02 | 0.77 | 0.37 |
| asammdf_dimensional_demo.mf4 | 0.08 | 0.04 | 4.49 | 0.40 |
| Vector_IntegerTypes.MF4 | 0.07 | 0.02 | 4.72 | 0.21 |
| test_metadata.mf4 | 0.14 | 0.05 | 4.86 | 0.40 |
| dSPACE_RealTypes.mf4 | 0.07 | 0.02 | 4.68 | 0.10 |
| Vector_MinimumFile.MF4 | 0.05 | 0.02 | 4.81 | 0.15 |
| dSPACE_CaptureBlocks.mf4 | 0.07 | 0.02 | 4.81 | 0.11 |
| Vector_CANape.MF4 | 0.06 | 0.02 | 4.78 | 0.13 |
| Vector_External.MF4 | 0.08 | 0.02 | 4.72 | 0.15 |
| Vector_CustomExtensions_CNcomment.mf4 | 0.07 | 0.02 | 4.79 | 0.15 |
| Vector_EmbeddedCompressed.MF4 | 0.09 | 0.02 | 4.83 | 0.15 |
| Vector_Embedded.MF4 | 0.08 | 0.02 | 4.76 | 0.12 |
| dSPACE_IntegerTypes.mf4 | 0.08 | 0.03 | 4.86 | 0.20 |
| Vector_SingleDZ_TransposeDeflate.mf4 | 0.05 | 0.55 | 4.70 | 1.19 |
| Vector_DataList_TransposeDeflate.mf4 | 0.05 | 0.64 | 5.03 | 1.74 |
| gen_many.mf4 | 0.67 | 0.07 | 6.63 | 3.25 |
| Vector_SingleDZ_Deflate.mf4 | 0.05 | 0.76 | 5.35 | 1.40 |
| Vector_DataList_Deflate.mf4 | 0.06 | 0.78 | 4.36 | 1.46 |
| ETAS_SimpleSorted.mf4 | 0.06 | 0.13 | 4.67 | 0.28 |
| gen_xy.mf4 | 0.05 | 0.16 | 4.53 | 0.28 |
| 00000012-64BB8F50.MF4 | 0.89 | 1.56 | 8.48 | 14.06 |
| ETAS_IntegerTypes.mf4 | 0.08 | 1.03 | 5.31 | 1.70 |
| dSPACE_HILAPITimeout.mf4 | 0.08 | 0.35 | 5.24 | 0.35 |
| dSPACE_HILAPITrigger.mf4 | 0.08 | 0.33 | 4.56 | 0.34 |
| 00000002.MF4 | 0.90 | 1.23 | 5.68 | 5.63 |
| ASAP2_Demo_V171.mf4 | 0.24 | 2.75 | 6.62 | 5.54 |
| 00000013-64BB9AA0.MF4 | 1.54 | 4.36 | 13.89 | 20.58 |
| 00000014-64BBA8AF.MF4 | 1.84 | 5.54 | 16.65 | 24.66 |
| 00002081.MF4 | 4.34 | 5.73 | 27.75 | 29.38 |
| 00002082.MF4 | 4.32 | 5.76 | 28.50 | 30.47 |
| 00002083.MF4 | 3.90 | 5.62 | 27.24 | 29.19 |
| 00002084.MF4 | 4.00 | 5.63 | 27.40 | 29.23 |
| large_deflate.mf4 | 0.18 | 1720.68 | 851.88 | 7593.43 |
| large_uncompressed.mf4 | 0.21 | 488.04 | 290.32 | 1159.60 |
