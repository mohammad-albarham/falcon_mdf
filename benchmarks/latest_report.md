## Performance: falcon_mdf vs asammdf

**Machine**: macOS-26.6.2-arm64-arm-64bit-Mach-O
**Processor**: arm
**Python**: 3.14.7
**asammdf**: 8.7.2
**falcon_mdf**: git b8db8fc
**Files tested**: 76

### Summary

| Metric | Value |
|---|---|
| Geometric mean speedup (vs `get()`) | 30.8× |
| Geometric mean speedup (vs `select()`) | 29.6× |
| Median speedup (vs `get()`) | 47.7× |
| Min speedup | 3.3× |
| Max speedup | 85.4× |
| Files where falcon faster | 76/76 |

### Results by File Size

Fixed overhead (asammdf's `MDF()` construction, ~5 ms) dominates the
smallest files, so the aggregate over the whole corpus overstates the
decoding advantage. Quote the `> 1 MB` row.

`Files` counts only files where both libraries decoded the same
number of samples; see Sample-Count Agreement below for the rest.

| Size bucket | Files | Geo. mean vs `get()` | Geo. mean vs `select()` | Worst vs `select()` |
|---|---|---|---|---|
| < 100 KB | 58 | 45.6× | 45.6× | 5.6× |
| 100 KB – 1 MB | 5 | 7.5× | 6.9× | 4.4× |
| > 1 MB | 8 | 4.6× | 3.5× | 2.6× |

### Sample-Count Agreement

falcon and asammdf decoded identical sample counts on **71/76** files.

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
| Vector_ByteArrayFixedLength.mf4 | 1.6 KB | 0.0001 | 0.0060 | 0.0049 | 85.4× | 69.6× |
| Vector_CANOpenTime.mf4 | 1.6 KB | 0.0001 | 0.0050 | 0.0045 | 71.0× | 63.6× |
| Vector_CANOpenDate.mf4 | 1.6 KB | 0.0001 | 0.0050 | 0.0052 | 62.3× | 65.5× |
| Vector_FixedLengthStringUTF16_BE.mf4 | 1.7 KB | 0.0001 | 0.0056 | 0.0050 | 80.3× | 71.2× |
| Vector_FixedLengthStringSBC.mf4 | 1.7 KB | 0.0001 | 0.0049 | 0.0050 | 70.2× | 71.2× |
| Vector_FixedLengthStringUTF8.mf4 | 1.7 KB | 0.0001 | 0.0050 | 0.0050 | 71.3× | 70.8× |
| Vector_FixedLengthStringUTF16_LE.mf4 | 1.7 KB | 0.0001 | 0.0049 | 0.0050 | 70.3× | 70.9× |
| video_sync.mf4 | 1.8 KB | 0.0001 | 0.0051 | 0.0052 | 72.3× | 75.0× |
| Vector_LinearConversion.mf4 | 2.1 KB | 0.0001 | 0.0048 | 0.0050 | 60.1× | 62.2× |
| Vector_Value2TextConversion.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0050 | 70.3× | 71.3× |
| Vector_AlgebraicConversionSinus.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0049 | 55.8× | 54.0× |
| Vector_AlgebraicConversionRational.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0050 | 55.7× | 55.9× |
| Vector_ValueRange2TextConversion.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0049 | 56.0× | 54.3× |
| Vector_RationalConversionIntParams.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0050 | 63.1× | 62.0× |
| Vector_RationalConversionRealParams.mf4 | 2.1 KB | 0.0001 | 0.0051 | 0.0050 | 56.2× | 55.8× |
| Vector_RationalConversionZeroedParams.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0050 | 61.5× | 62.2× |
| Vector_AlgebraicConversionQuadratic.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0049 | 61.5× | 60.8× |
| Vector_ArrayWithFixedAxes.MF4 | 2.2 KB | 0.0001 | 0.0050 | 0.0050 | 62.6× | 61.9× |
| Vector_Text2ValueConversion.mf4 | 2.3 KB | 0.0001 | 0.0049 | 0.0050 | 61.8× | 61.9× |
| Vector_Value2ValueConversionInterpolation.mf4 | 2.3 KB | 0.0001 | 0.0048 | 0.0050 | 68.2× | 70.9× |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 2.6 KB | 0.0001 | 0.0054 | 0.0055 | 67.2× | 68.3× |
| Vector_Text2TextConversion.mf4 | 2.6 KB | 0.0001 | 0.0050 | 0.0050 | 62.9× | 62.8× |
| Vector_ValueRange2ValueConversion.mf4 | 2.7 KB | 0.0001 | 0.0055 | 0.0050 | 68.5× | 62.4× |
| dSPACE_LinearConversion.mf4 | 2.8 KB | 0.0001 | 0.0049 | 0.0051 | 48.8× | 51.1× |
| dSPACE_AlgebraicConversion.mf4 | 2.8 KB | 0.0001 | 0.0050 | 0.0059 | 45.4× | 53.6× |
| Vector_AttachmentRef.mf4 | 2.9 KB | 0.0001 | 0.0049 | 0.0051 | 70.6× | 72.3× |
| dSPACE_Value2TextConversion.mf4 | 2.9 KB | 0.0001 | 0.0049 | 0.0050 | 61.3× | 62.9× |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 2.9 KB | 0.0001 | 0.0051 | 0.0051 | 63.7× | 63.4× |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 2.9 KB | 0.0001 | 0.0050 | 0.0052 | 55.7× | 57.9× |
| dSPACE_ValueRange2TextConversion.mf4 | 3.0 KB | 0.0001 | 0.0053 | 0.0049 | 58.6× | 54.7× |
| test_batch_cut_0.mf4 | 3.3 KB | 0.0001 | 0.0056 | 0.0051 | 46.4× | 42.3× |
| Vector_DefaultX.mf4 | 3.4 KB | 0.0001 | 0.0050 | 0.0057 | 62.6× | 71.5× |
| test_batch_cut_1.mf4 | 3.4 KB | 0.0001 | 0.0048 | 0.0050 | 48.4× | 49.8× |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 3.9 KB | 0.0001 | 0.0048 | 0.0051 | 59.8× | 64.1× |
| all_datatypes_test.mf4 | 5.6 KB | 0.0002 | 0.0055 | 0.0057 | 36.9× | 37.7× |
| dSPACE_MeasurementArrays.mf4 | 6.3 KB | 0.0001 | 0.0055 | 0.0051 | 60.9× | 56.4× |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 6.9 KB | 0.0001 | 0.0050 | 0.0052 | 36.0× | 37.3× |
| single_lin_bus_1.MF4 | 7.1 KB | 0.0001 | 0.0013 | 0.0014 | 11.8× | 12.3× |
| single_can_bus_1.MF4 | 7.1 KB | 0.0001 | 0.0013 | 0.0012 | 12.6× | 12.4× |
| test_batch.mf4 | 8.6 KB | 0.0002 | 0.0057 | 0.0049 | 35.8× | 30.9× |
| Vector_RealTypes.MF4 | 9.0 KB | 0.0001 | 0.0050 | 0.0048 | 62.9× | 59.9× |
| simple.mf4 | 9.6 KB | 0.0002 | 0.0052 | 0.0052 | 27.3× | 27.3× |
| Vector_PartialConversionValueRange2TextRational.mf4 | 10.3 KB | 0.0001 | 0.0053 | 0.0055 | 58.4× | 61.0× |
| Vector_MeasurementArrays.mf4 | 12.2 KB | 0.0001 | 0.0061 | N/A | 47.2× | N/A |
| dSPACE_Bookmarks.mf4 | 13.3 KB | 0.0001 | 0.0050 | 0.0050 | 45.2× | 45.4× |
| multiple_fin.MF4 | 13.6 KB | 0.0001 | 0.0058 | 0.0059 | 48.2× | 48.9× |
| multiple.MF4 | 13.9 KB | 0.0001 | 0.0014 | 0.0012 | 10.1× | 8.7× |
| asammdf_dimensional_demo.mf4 | 17.9 KB | 0.0002 | 0.0060 | 0.0063 | 37.3× | 39.1× |
| Vector_IntegerTypes.MF4 | 18.9 KB | 0.0001 | 0.0050 | 0.0058 | 35.7× | 41.6× |
| test_metadata.mf4 | 19.6 KB | 0.0003 | 0.0059 | 0.0060 | 22.5× | 23.2× |
| dSPACE_RealTypes.mf4 | 23.2 KB | 0.0001 | 0.0047 | 0.0052 | 42.8× | 46.9× |
| Vector_MinimumFile.MF4 | 24.2 KB | 0.0001 | 0.0051 | 0.0049 | 57.1× | 54.5× |
| dSPACE_CaptureBlocks.mf4 | 24.6 KB | 0.0001 | 0.0050 | 0.0050 | 41.9× | 41.8× |
| Vector_CANape.MF4 | 25.3 KB | 0.0001 | 0.0054 | 0.0050 | 49.0× | 45.0× |
| Vector_External.MF4 | 27.0 KB | 0.0001 | 0.0050 | 0.0054 | 35.9× | 38.3× |
| Vector_CustomExtensions_CNcomment.mf4 | 27.3 KB | 0.0001 | 0.0051 | 0.0057 | 42.8× | 47.8× |
| Vector_EmbeddedCompressed.MF4 | 28.1 KB | 0.0001 | 0.0051 | 0.0061 | 39.0× | 46.9× |
| Vector_Embedded.MF4 | 28.5 KB | 0.0001 | 0.0050 | 0.0053 | 38.7× | 40.5× |
| dSPACE_IntegerTypes.mf4 | 44.1 KB | 0.0002 | 0.0059 | 0.0052 | 39.3× | 34.8× |
| Vector_SingleDZ_TransposeDeflate.mf4 | 61.2 KB | 0.0008 | 0.0061 | 0.0060 | 7.2× | 7.2× |
| Vector_DataList_TransposeDeflate.mf4 | 68.5 KB | 0.0011 | 0.0071 | 0.0059 | 6.6× | 5.6× |
| Vector_SingleDZ_Deflate.mf4 | 119.0 KB | 0.0011 | 0.0070 | 0.0060 | 6.1× | 5.2× |
| Vector_DataList_Deflate.mf4 | 120.7 KB | 0.0011 | 0.0069 | 0.0058 | 6.2× | 5.3× |
| ETAS_SimpleSorted.mf4 | 209.6 KB | 0.0002 | 0.0051 | 0.0058 | 21.1× | 24.2× |
| 00000012-64BB8F50.MF4 | 658.3 KB | 0.0045 | 0.0283 | 0.0244 | 6.3× | 5.4× |
| ETAS_IntegerTypes.mf4 | 1007.4 KB | 0.0015 | 0.0070 | 0.0064 | 4.8× | 4.4× |
| dSPACE_HILAPITimeout.mf4 | 1.0 MB | 0.0005 | 0.0050 | 0.0064 | 9.4× | 12.0× |
| dSPACE_HILAPITrigger.mf4 | 1.0 MB | 0.0005 | 0.0057 | 0.0050 | 10.7× | 9.4× |
| 00000002.MF4 | 1.0 MB | 0.0029 | 0.0138 | 0.0108 | 4.8× | 3.8× |
| ASAP2_Demo_V171.mf4 | 1.2 MB | 0.0040 | 0.0132 | 0.0101 | 3.3× | 2.6× |
| 00000013-64BB9AA0.MF4 | 1.7 MB | 0.0116 | 0.0439 | 0.0377 | 3.8× | 3.3× |
| 00000014-64BBA8AF.MF4 | 2.1 MB | 0.0152 | 0.0535 | 0.0432 | 3.5× | 2.8× |
| 00002081.MF4 | 5.0 MB | 0.0138 | 0.0741 | 0.0547 | 5.4× | 4.0× |
| 00002082.MF4 | 5.0 MB | 0.0137 | 0.0766 | 0.0545 | 5.6× | 4.0× |
| 00002083.MF4 | 5.0 MB | 0.0139 | 0.0745 | 0.0536 | 5.4× | 3.9× |
| 00002084.MF4 | 5.0 MB | 0.0138 | 0.0733 | 0.0546 | 5.3× | 4.0× |

### Memory

| File | falcon RSS (MB) | asammdf RSS (MB) | Ratio |
|---|---|---|---|
| Vector_ByteArrayFixedLength.mf4 | 1.8 | 131.7 | 71.4× |
| Vector_CANOpenTime.mf4 | 1.8 | 130.8 | 70.9× |
| Vector_CANOpenDate.mf4 | 1.8 | 132.2 | 71.7× |
| Vector_FixedLengthStringUTF16_BE.mf4 | 1.8 | 132.2 | 71.7× |
| Vector_FixedLengthStringSBC.mf4 | 1.8 | 130.5 | 70.8× |
| Vector_FixedLengthStringUTF8.mf4 | 1.8 | 131.8 | 71.5× |
| Vector_FixedLengthStringUTF16_LE.mf4 | 1.8 | 130.8 | 71.0× |
| video_sync.mf4 | 1.8 | 130.9 | 71.0× |
| Vector_LinearConversion.mf4 | 1.8 | 130.2 | 70.6× |
| Vector_Value2TextConversion.mf4 | 1.9 | 131.8 | 70.9× |
| Vector_AlgebraicConversionSinus.mf4 | 1.9 | 132.0 | 70.4× |
| Vector_AlgebraicConversionRational.mf4 | 1.9 | 132.2 | 70.5× |
| Vector_ValueRange2TextConversion.mf4 | 1.9 | 130.4 | 70.1× |
| Vector_RationalConversionIntParams.mf4 | 1.8 | 132.0 | 71.6× |
| Vector_RationalConversionRealParams.mf4 | 1.8 | 132.0 | 71.6× |
| Vector_RationalConversionZeroedParams.mf4 | 1.8 | 131.0 | 71.0× |
| Vector_AlgebraicConversionQuadratic.mf4 | 1.9 | 132.8 | 70.8× |
| Vector_ArrayWithFixedAxes.MF4 | 1.9 | 132.0 | 70.4× |
| Vector_Text2ValueConversion.mf4 | 1.9 | 131.8 | 70.9× |
| Vector_Value2ValueConversionInterpolation.mf4 | 1.8 | 130.1 | 70.6× |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 1.8 | 131.8 | 71.5× |
| Vector_Text2TextConversion.mf4 | 1.9 | 131.8 | 70.9× |
| Vector_ValueRange2ValueConversion.mf4 | 1.8 | 131.9 | 71.5× |
| dSPACE_LinearConversion.mf4 | 1.8 | 132.2 | 71.7× |
| dSPACE_AlgebraicConversion.mf4 | 1.9 | 132.0 | 70.4× |
| Vector_AttachmentRef.mf4 | 1.8 | 132.4 | 71.8× |
| dSPACE_Value2TextConversion.mf4 | 1.9 | 132.0 | 71.0× |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 1.8 | 130.1 | 70.6× |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 1.8 | 132.0 | 71.6× |
| dSPACE_ValueRange2TextConversion.mf4 | 1.9 | 131.8 | 70.9× |
| test_batch_cut_0.mf4 | 2.0 | 132.2 | 66.1× |
| Vector_DefaultX.mf4 | 1.9 | 131.6 | 70.8× |
| test_batch_cut_1.mf4 | 2.0 | 132.0 | 66.0× |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 1.9 | 132.1 | 70.4× |
| all_datatypes_test.mf4 | 2.1 | 132.1 | 63.5× |
| dSPACE_MeasurementArrays.mf4 | 1.9 | 131.7 | 70.8× |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 2.0 | 132.0 | 66.0× |
| single_lin_bus_1.MF4 | 1.9 | 132.6 | 69.0× |
| single_can_bus_1.MF4 | 1.9 | 132.3 | 68.8× |
| test_batch.mf4 | 2.1 | 132.0 | 62.1× |
| Vector_RealTypes.MF4 | 1.9 | 132.0 | 69.8× |
| simple.mf4 | 2.2 | 132.0 | 61.2× |
| Vector_PartialConversionValueRange2TextRational.mf4 | 1.9 | 132.6 | 70.1× |
| Vector_MeasurementArrays.mf4 | 2.0 | 132.3 | 65.1× |
| dSPACE_Bookmarks.mf4 | 1.9 | 132.0 | 68.7× |
| multiple_fin.MF4 | 2.0 | 132.6 | 64.8× |
| multiple.MF4 | 2.0 | 132.2 | 64.6× |
| asammdf_dimensional_demo.mf4 | 2.1 | 132.5 | 64.3× |
| Vector_IntegerTypes.MF4 | 2.0 | 131.8 | 67.5× |
| test_metadata.mf4 | 2.3 | 132.3 | 58.0× |
| dSPACE_RealTypes.mf4 | 1.9 | 132.7 | 68.5× |
| Vector_MinimumFile.MF4 | 2.0 | 132.0 | 66.0× |
| dSPACE_CaptureBlocks.mf4 | 1.9 | 132.4 | 68.3× |
| Vector_CANape.MF4 | 2.0 | 132.1 | 66.6× |
| Vector_External.MF4 | 2.0 | 132.5 | 65.2× |
| Vector_CustomExtensions_CNcomment.mf4 | 2.0 | 131.8 | 66.4× |
| Vector_EmbeddedCompressed.MF4 | 2.0 | 132.1 | 64.5× |
| Vector_Embedded.MF4 | 2.0 | 131.8 | 65.9× |
| dSPACE_IntegerTypes.mf4 | 2.1 | 131.6 | 63.3× |
| Vector_SingleDZ_TransposeDeflate.mf4 | 2.7 | 132.4 | 48.4× |
| Vector_DataList_TransposeDeflate.mf4 | 2.5 | 132.3 | 53.9× |
| Vector_SingleDZ_Deflate.mf4 | 2.9 | 132.1 | 46.2× |
| Vector_DataList_Deflate.mf4 | 2.5 | 131.9 | 53.1× |
| ETAS_SimpleSorted.mf4 | 2.4 | 132.0 | 54.5× |
| 00000012-64BB8F50.MF4 | 5.8 | 137.5 | 23.7× |
| ETAS_IntegerTypes.mf4 | 4.2 | 133.0 | 31.9× |
| dSPACE_HILAPITimeout.mf4 | 2.9 | 132.4 | 45.1× |
| dSPACE_HILAPITrigger.mf4 | 2.9 | 132.2 | 45.0× |
| 00000002.MF4 | 8.1 | 141.8 | 17.6× |
| ASAP2_Demo_V171.mf4 | 5.1 | 134.4 | 26.3× |
| 00000013-64BB9AA0.MF4 | 10.7 | 147.2 | 13.7× |
| 00000014-64BBA8AF.MF4 | 13.8 | 156.5 | 11.4× |
| 00002081.MF4 | 36.2 | 170.3 | 4.7× |
| 00002082.MF4 | 36.2 | 169.9 | 4.7× |
| 00002083.MF4 | 36.2 | 170.3 | 4.7× |
| 00002084.MF4 | 36.2 | 174.9 | 4.8× |

Both columns are peak resident set size of the whole process, measured with `/usr/bin/time`.
A bare interpreter that only does `import asammdf` already peaks at **129.5 MB**; subtract that to compare decoding cost rather than runtime cost.

### Timing Breakdown

| File | falcon open (ms) | falcon decode (ms) | asammdf open (ms) | asammdf decode (ms) |
|---|---|---|---|---|
| Vector_ByteArrayFixedLength.mf4 | 0.06 | 0.01 | 5.95 | 0.04 |
| Vector_CANOpenTime.mf4 | 0.06 | 0.01 | 4.94 | 0.04 |
| Vector_CANOpenDate.mf4 | 0.07 | 0.01 | 4.91 | 0.09 |
| Vector_FixedLengthStringUTF16_BE.mf4 | 0.06 | 0.01 | 5.56 | 0.05 |
| Vector_FixedLengthStringSBC.mf4 | 0.06 | 0.01 | 4.86 | 0.06 |
| Vector_FixedLengthStringUTF8.mf4 | 0.06 | 0.01 | 4.94 | 0.04 |
| Vector_FixedLengthStringUTF16_LE.mf4 | 0.06 | 0.01 | 4.88 | 0.04 |
| video_sync.mf4 | 0.06 | 0.01 | 4.72 | 0.34 |
| Vector_LinearConversion.mf4 | 0.07 | 0.01 | 4.74 | 0.06 |
| Vector_Value2TextConversion.mf4 | 0.06 | 0.01 | 4.84 | 0.05 |
| Vector_AlgebraicConversionSinus.mf4 | 0.08 | 0.01 | 4.96 | 0.06 |
| Vector_AlgebraicConversionRational.mf4 | 0.08 | 0.01 | 4.94 | 0.08 |
| Vector_ValueRange2TextConversion.mf4 | 0.08 | 0.01 | 4.95 | 0.07 |
| Vector_RationalConversionIntParams.mf4 | 0.07 | 0.01 | 4.96 | 0.08 |
| Vector_RationalConversionRealParams.mf4 | 0.08 | 0.01 | 4.97 | 0.09 |
| Vector_RationalConversionZeroedParams.mf4 | 0.07 | 0.01 | 4.84 | 0.08 |
| Vector_AlgebraicConversionQuadratic.mf4 | 0.07 | 0.01 | 4.85 | 0.06 |
| Vector_ArrayWithFixedAxes.MF4 | 0.07 | 0.01 | 4.92 | 0.09 |
| Vector_Text2ValueConversion.mf4 | 0.07 | 0.01 | 4.90 | 0.05 |
| Vector_Value2ValueConversionInterpolation.mf4 | 0.06 | 0.01 | 4.72 | 0.06 |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 0.07 | 0.01 | 5.25 | 0.09 |
| Vector_Text2TextConversion.mf4 | 0.07 | 0.01 | 4.98 | 0.06 |
| Vector_ValueRange2ValueConversion.mf4 | 0.07 | 0.01 | 5.38 | 0.10 |
| dSPACE_LinearConversion.mf4 | 0.09 | 0.01 | 4.81 | 0.06 |
| dSPACE_AlgebraicConversion.mf4 | 0.10 | 0.01 | 4.92 | 0.08 |
| Vector_AttachmentRef.mf4 | 0.06 | 0.01 | 4.79 | 0.16 |
| dSPACE_Value2TextConversion.mf4 | 0.07 | 0.01 | 4.85 | 0.06 |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 0.07 | 0.01 | 5.05 | 0.05 |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 0.08 | 0.01 | 4.96 | 0.07 |
| dSPACE_ValueRange2TextConversion.mf4 | 0.08 | 0.01 | 5.18 | 0.08 |
| test_batch_cut_0.mf4 | 0.09 | 0.03 | 5.51 | 0.06 |
| Vector_DefaultX.mf4 | 0.07 | 0.01 | 4.92 | 0.08 |
| test_batch_cut_1.mf4 | 0.08 | 0.02 | 4.77 | 0.08 |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 0.07 | 0.01 | 4.60 | 0.19 |
| all_datatypes_test.mf4 | 0.07 | 0.08 | 5.30 | 0.23 |
| dSPACE_MeasurementArrays.mf4 | 0.08 | 0.01 | 5.30 | 0.16 |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 0.07 | 0.07 | 4.85 | 0.16 |
| single_lin_bus_1.MF4 | 0.10 | 0.01 | 0.90 | 0.40 |
| single_can_bus_1.MF4 | 0.09 | 0.01 | 0.86 | 0.40 |
| test_batch.mf4 | 0.05 | 0.11 | 5.49 | 0.25 |
| Vector_RealTypes.MF4 | 0.07 | 0.01 | 4.91 | 0.11 |
| simple.mf4 | 0.06 | 0.13 | 4.52 | 0.58 |
| Vector_PartialConversionValueRange2TextRational.mf4 | 0.07 | 0.02 | 5.00 | 0.25 |
| Vector_MeasurementArrays.mf4 | 0.11 | 0.02 | 5.52 | 0.62 |
| dSPACE_Bookmarks.mf4 | 0.10 | 0.01 | 4.92 | 0.05 |
| multiple_fin.MF4 | 0.09 | 0.03 | 5.29 | 0.49 |
| multiple.MF4 | 0.11 | 0.03 | 0.92 | 0.48 |
| asammdf_dimensional_demo.mf4 | 0.10 | 0.06 | 5.40 | 0.59 |
| Vector_IntegerTypes.MF4 | 0.10 | 0.04 | 4.76 | 0.24 |
| test_metadata.mf4 | 0.19 | 0.07 | 5.33 | 0.53 |
| dSPACE_RealTypes.mf4 | 0.08 | 0.03 | 4.62 | 0.09 |
| Vector_MinimumFile.MF4 | 0.06 | 0.03 | 4.98 | 0.18 |
| dSPACE_CaptureBlocks.mf4 | 0.09 | 0.03 | 4.90 | 0.13 |
| Vector_CANape.MF4 | 0.08 | 0.03 | 5.23 | 0.16 |
| Vector_External.MF4 | 0.11 | 0.03 | 4.87 | 0.16 |
| Vector_CustomExtensions_CNcomment.mf4 | 0.09 | 0.03 | 4.95 | 0.17 |
| Vector_EmbeddedCompressed.MF4 | 0.10 | 0.03 | 4.91 | 0.16 |
| Vector_Embedded.MF4 | 0.10 | 0.03 | 4.86 | 0.17 |
| dSPACE_IntegerTypes.mf4 | 0.10 | 0.05 | 5.67 | 0.23 |
| Vector_SingleDZ_TransposeDeflate.mf4 | 0.07 | 0.77 | 4.56 | 1.51 |
| Vector_DataList_TransposeDeflate.mf4 | 0.07 | 1.00 | 4.87 | 2.23 |
| Vector_SingleDZ_Deflate.mf4 | 0.06 | 1.08 | 5.28 | 1.69 |
| Vector_DataList_Deflate.mf4 | 0.07 | 1.03 | 5.05 | 1.87 |
| ETAS_SimpleSorted.mf4 | 0.07 | 0.17 | 4.74 | 0.33 |
| 00000012-64BB8F50.MF4 | 1.06 | 3.45 | 10.05 | 18.26 |
| ETAS_IntegerTypes.mf4 | 0.10 | 1.35 | 4.80 | 2.16 |
| dSPACE_HILAPITimeout.mf4 | 0.09 | 0.44 | 4.57 | 0.41 |
| dSPACE_HILAPITrigger.mf4 | 0.09 | 0.44 | 5.23 | 0.43 |
| 00000002.MF4 | 1.10 | 1.77 | 6.75 | 7.01 |
| ASAP2_Demo_V171.mf4 | 0.32 | 3.63 | 6.15 | 7.00 |
| 00000013-64BB9AA0.MF4 | 2.00 | 9.60 | 16.92 | 26.89 |
| 00000014-64BBA8AF.MF4 | 2.48 | 12.72 | 22.09 | 31.62 |
| 00002081.MF4 | 5.17 | 8.59 | 35.66 | 38.44 |
| 00002082.MF4 | 5.21 | 8.47 | 36.61 | 39.19 |
| 00002083.MF4 | 5.30 | 8.57 | 36.04 | 38.48 |
| 00002084.MF4 | 5.19 | 8.56 | 35.64 | 37.86 |
