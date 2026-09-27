## Performance: falcon_mdf vs asammdf

**Machine**: macOS-27.0-arm64-arm-64bit-Mach-O
**Processor**: arm
**Generated**: 2026-09-27T03:00:41+02:00
**Python**: 3.14.7
**asammdf**: 8.7.2
**falcon_mdf**: git 4902572
**Files tested**: 87

### Summary

| Metric | Value |
|---|---|
| Geometric mean speedup (vs `get()`) | 37.9× |
| Geometric mean speedup (vs `select()`) | 35.3× |
| Median speedup (vs `get()`) | 58.3× |
| Min speedup | 3.3× |
| Max speedup | 101.2× |
| Files where falcon faster | 87/87 |

### Results by File Size

Fixed overhead (asammdf's `MDF()` construction, ~5 ms) dominates the
smallest files, so the aggregate over the whole corpus overstates the
decoding advantage. Quote the `> 1 MB` row.

`Files` counts only files where both libraries decoded the same
number of samples; see Sample-Count Agreement below for the rest.

| Size bucket | Files | Geo. mean vs `get()` | Geo. mean vs `select()` | Worst vs `select()` |
|---|---|---|---|---|
| < 100 KB | 65 | 60.7× | 60.4× | 9.6× |
| 100 KB – 1 MB | 7 | 14.4× | 12.8× | 5.3× |
| > 1 MB | 12 | 6.5× | 4.5× | 1.8× |

### Sample-Count Agreement

falcon and asammdf decoded identical sample counts on **84/87** files.

These files are excluded from the equal-work aggregates above, because a ratio between different amounts of work is not a speedup:

| File | Size | falcon samples | asammdf samples |
|---|---|---|---|
| Vector_MeasurementArrays.mf4 | 12.2 KB | 82 | 78 |
| mdflib_mixed_compressed.mf4 | 119.4 KB | 35,000 | 34,285 |
| mdflib_mixed.mf4 | 353.1 KB | 35,000 | 34,285 |

### Per-File Results

| File | Size | falcon (s) | asammdf get (s) | asammdf select (s) | Speedup (get) | Speedup (select) |
|---|---|---|---|---|---|---|
| Vector_ByteArrayFixedLength.mf4 | 1.6 KB | 0.0001 | 0.0048 | 0.0050 | 79.3× | 82.7× |
| Vector_CANOpenTime.mf4 | 1.6 KB | 0.0001 | 0.0049 | 0.0049 | 98.5× | 97.5× |
| Vector_CANOpenDate.mf4 | 1.6 KB | 0.0001 | 0.0041 | 0.0049 | 81.2× | 97.9× |
| Vector_FixedLengthStringUTF16_BE.mf4 | 1.7 KB | 0.0001 | 0.0040 | 0.0049 | 65.9× | 81.4× |
| Vector_FixedLengthStringUTF16_LE.mf4 | 1.7 KB | 0.0001 | 0.0050 | 0.0051 | 99.5× | 102.4× |
| Vector_FixedLengthStringSBC.mf4 | 1.7 KB | 0.0001 | 0.0050 | 0.0050 | 100.5× | 100.2× |
| Vector_FixedLengthStringUTF8.mf4 | 1.7 KB | 0.0001 | 0.0050 | 0.0049 | 99.6× | 98.9× |
| video_sync.mf4 | 1.8 KB | 0.0001 | 0.0047 | 0.0049 | 94.4× | 98.0× |
| mdflib_array.mf4 | 1.8 KB | 0.0001 | 0.0050 | 0.0050 | 100.3× | 99.6× |
| Vector_LinearConversion.mf4 | 2.1 KB | 0.0001 | 0.0051 | 0.0049 | 101.1× | 98.9× |
| gen_structure.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0050 | 99.2× | 99.7× |
| Vector_Value2TextConversion.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0050 | 82.0× | 82.9× |
| Vector_AlgebraicConversionSinus.mf4 | 2.1 KB | 0.0001 | 0.0049 | 0.0050 | 98.9× | 100.1× |
| Vector_AlgebraicConversionRational.mf4 | 2.1 KB | 0.0001 | 0.0052 | 0.0046 | 86.4× | 76.3× |
| Vector_RationalConversionRealParams.mf4 | 2.1 KB | 0.0001 | 0.0044 | 0.0048 | 88.5× | 96.7× |
| Vector_AlgebraicConversionQuadratic.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0048 | 83.4× | 79.4× |
| Vector_ValueRange2TextConversion.mf4 | 2.1 KB | 0.0001 | 0.0048 | 0.0050 | 68.3× | 70.8× |
| Vector_RationalConversionIntParams.mf4 | 2.1 KB | 0.0001 | 0.0041 | 0.0049 | 82.6× | 98.8× |
| Vector_RationalConversionZeroedParams.mf4 | 2.1 KB | 0.0001 | 0.0050 | 0.0048 | 83.0× | 79.3× |
| Vector_ArrayWithFixedAxes.MF4 | 2.2 KB | 0.0001 | 0.0050 | 0.0051 | 82.7× | 84.5× |
| Vector_Text2ValueConversion.mf4 | 2.3 KB | 0.0001 | 0.0050 | 0.0040 | 70.9× | 57.3× |
| Vector_Value2ValueConversionInterpolation.mf4 | 2.3 KB | 0.0001 | 0.0040 | 0.0051 | 80.9× | 102.3× |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 2.6 KB | 0.0001 | 0.0049 | 0.0048 | 98.1× | 95.4× |
| Vector_Text2TextConversion.mf4 | 2.6 KB | 0.0001 | 0.0041 | 0.0049 | 68.5× | 81.6× |
| Vector_ValueRange2ValueConversion.mf4 | 2.7 KB | 0.0001 | 0.0051 | 0.0047 | 101.2× | 93.8× |
| dSPACE_LinearConversion.mf4 | 2.8 KB | 0.0001 | 0.0040 | 0.0048 | 67.2× | 79.3× |
| dSPACE_AlgebraicConversion.mf4 | 2.8 KB | 0.0001 | 0.0048 | 0.0047 | 80.7× | 78.1× |
| Vector_AttachmentRef.mf4 | 2.9 KB | 0.0001 | 0.0049 | 0.0046 | 98.0× | 91.5× |
| dSPACE_Value2TextConversion.mf4 | 2.9 KB | 0.0001 | 0.0049 | 0.0040 | 81.6× | 67.0× |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 2.9 KB | 0.0001 | 0.0047 | 0.0048 | 77.7× | 79.8× |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 2.9 KB | 0.0001 | 0.0045 | 0.0041 | 75.5× | 68.7× |
| dSPACE_ValueRange2TextConversion.mf4 | 3.0 KB | 0.0001 | 0.0041 | 0.0049 | 68.3× | 80.9× |
| test_batch_cut_0.mf4 | 3.3 KB | 0.0001 | 0.0049 | 0.0040 | 60.6× | 50.4× |
| Vector_DefaultX.mf4 | 3.4 KB | 0.0001 | 0.0042 | 0.0050 | 69.2× | 82.8× |
| test_batch_cut_1.mf4 | 3.4 KB | 0.0001 | 0.0047 | 0.0041 | 58.3× | 50.8× |
| mdflib_big_endian.mf4 | 3.8 KB | 0.0001 | 0.0040 | 0.0047 | 67.5× | 78.3× |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 3.9 KB | 0.0001 | 0.0048 | 0.0046 | 79.4× | 77.0× |
| all_datatypes_test.mf4 | 5.6 KB | 0.0001 | 0.0047 | 0.0047 | 58.3× | 59.2× |
| dSPACE_MeasurementArrays.mf4 | 6.3 KB | 0.0001 | 0.0041 | 0.0049 | 58.2× | 70.0× |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 6.9 KB | 0.0001 | 0.0048 | 0.0041 | 43.9× | 37.7× |
| single_lin_bus_1.MF4 | 7.1 KB | 0.0001 | 0.0009 | 0.0009 | 11.6× | 11.0× |
| single_can_bus_1.MF4 | 7.1 KB | 0.0001 | 0.0009 | 0.0009 | 11.6× | 10.6× |
| test_batch.mf4 | 8.6 KB | 0.0001 | 0.0047 | 0.0040 | 52.0× | 44.9× |
| Vector_RealTypes.MF4 | 9.0 KB | 0.0001 | 0.0047 | 0.0041 | 67.0× | 58.1× |
| simple.mf4 | 9.6 KB | 0.0001 | 0.0049 | 0.0042 | 44.7× | 38.3× |
| Vector_PartialConversionValueRange2TextRational.mf4 | 10.3 KB | 0.0001 | 0.0042 | 0.0048 | 70.0× | 80.6× |
| Vector_MeasurementArrays.mf4 | 12.2 KB | 0.0001 | 0.0049 | N/A | 44.3× | N/A |
| dSPACE_Bookmarks.mf4 | 13.3 KB | 0.0001 | 0.0040 | 0.0048 | 44.4× | 53.1× |
| multiple_fin.MF4 | 13.6 KB | 0.0001 | 0.0052 | 0.0049 | 57.8× | 54.5× |
| multiple.MF4 | 13.9 KB | 0.0001 | 0.0011 | 0.0010 | 10.7× | 9.6× |
| mdflib_ethernet.mf4 | 14.8 KB | 0.0001 | 0.0059 | 0.0053 | 53.4× | 48.4× |
| asammdf_dimensional_demo.mf4 | 17.9 KB | 0.0001 | 0.0050 | 0.0051 | 45.7× | 46.5× |
| Vector_IntegerTypes.MF4 | 18.9 KB | 0.0001 | 0.0049 | 0.0041 | 61.8× | 50.8× |
| test_metadata.mf4 | 19.6 KB | 0.0002 | 0.0052 | 0.0049 | 32.4× | 30.4× |
| dSPACE_RealTypes.mf4 | 23.2 KB | 0.0001 | 0.0046 | 0.0048 | 65.8× | 68.0× |
| Vector_MinimumFile.MF4 | 24.2 KB | 0.0001 | 0.0044 | 0.0047 | 74.1× | 78.3× |
| dSPACE_CaptureBlocks.mf4 | 24.6 KB | 0.0001 | 0.0046 | 0.0050 | 57.2× | 63.0× |
| Vector_CANape.MF4 | 25.3 KB | 0.0001 | 0.0049 | 0.0050 | 61.6× | 61.9× |
| mdflib_flexray.mf4 | 25.6 KB | 0.0002 | 0.0061 | 0.0058 | 38.0× | 36.2× |
| Vector_External.MF4 | 27.0 KB | 0.0001 | 0.0048 | 0.0041 | 53.4× | 45.7× |
| Vector_CustomExtensions_CNcomment.mf4 | 27.3 KB | 0.0001 | 0.0041 | 0.0040 | 51.4× | 49.4× |
| Vector_EmbeddedCompressed.MF4 | 28.1 KB | 0.0001 | 0.0041 | 0.0048 | 45.4× | 53.7× |
| Vector_Embedded.MF4 | 28.5 KB | 0.0001 | 0.0049 | 0.0050 | 54.4× | 55.9× |
| dSPACE_IntegerTypes.mf4 | 44.1 KB | 0.0001 | 0.0050 | 0.0049 | 49.8× | 48.8× |
| Vector_SingleDZ_TransposeDeflate.mf4 | 61.2 KB | 0.0004 | 0.0058 | 0.0049 | 15.6× | 13.4× |
| Vector_DataList_TransposeDeflate.mf4 | 68.5 KB | 0.0005 | 0.0060 | 0.0049 | 11.7× | 9.7× |
| gen_many.mf4 | 101.8 KB | 0.0006 | 0.0094 | 0.0078 | 15.4× | 12.8× |
| Vector_SingleDZ_Deflate.mf4 | 119.0 KB | 0.0005 | 0.0059 | 0.0050 | 11.1× | 9.5× |
| mdflib_mixed_compressed.mf4 | 119.4 KB | 0.0010 | 0.0080 | 0.0063 | 7.9× | 6.2× |
| Vector_DataList_Deflate.mf4 | 120.7 KB | 0.0006 | 0.0062 | 0.0049 | 10.3× | 8.2× |
| ETAS_SimpleSorted.mf4 | 209.6 KB | 0.0001 | 0.0048 | 0.0050 | 34.2× | 35.6× |
| gen_xy.mf4 | 235.4 KB | 0.0001 | 0.0049 | 0.0049 | 37.6× | 37.5× |
| mdflib_mixed.mf4 | 353.1 KB | 0.0006 | 0.0059 | 0.0059 | 10.1× | 10.1× |
| 00000012-64BB8F50.MF4 | 658.3 KB | 0.0022 | 0.0208 | 0.0173 | 9.4× | 7.8× |
| ETAS_IntegerTypes.mf4 | 1007.4 KB | 0.0010 | 0.0059 | 0.0051 | 6.2× | 5.3× |
| dSPACE_HILAPITimeout.mf4 | 1.0 MB | 0.0004 | 0.0051 | 0.0049 | 13.1× | 12.6× |
| dSPACE_HILAPITrigger.mf4 | 1.0 MB | 0.0004 | 0.0049 | 0.0050 | 13.7× | 13.9× |
| 00000002.MF4 | 1.0 MB | 0.0019 | 0.0105 | 0.0082 | 5.4× | 4.2× |
| ASAP2_Demo_V171.mf4 | 1.2 MB | 0.0025 | 0.0111 | 0.0095 | 4.4× | 3.7× |
| 00000013-64BB9AA0.MF4 | 1.7 MB | 0.0056 | 0.0326 | 0.0259 | 5.8× | 4.6× |
| 00000014-64BBA8AF.MF4 | 2.1 MB | 0.0073 | 0.0388 | 0.0306 | 5.3× | 4.2× |
| 00002081.MF4 | 5.0 MB | 0.0090 | 0.0567 | 0.0405 | 6.3× | 4.5× |
| 00002082.MF4 | 5.0 MB | 0.0091 | 0.0578 | 0.0407 | 6.3× | 4.5× |
| 00002083.MF4 | 5.0 MB | 0.0092 | 0.0568 | 0.0402 | 6.2× | 4.4× |
| 00002084.MF4 | 5.0 MB | 0.0090 | 0.0561 | 0.0408 | 6.2× | 4.5× |
| large_deflate.mf4 | 121.9 MB | 1.0316 | 8.3396 | 1.8467 | 8.1× | 1.8× |
| large_uncompressed.mf4 | 479.7 MB | 0.4223 | 1.4074 | 0.7532 | 3.3× | 1.8× |

### Memory

| File | falcon RSS (MB) | asammdf RSS (MB) | Ratio |
|---|---|---|---|
| Vector_ByteArrayFixedLength.mf4 | 2.2 | 156.8 | 70.2× |
| Vector_CANOpenTime.mf4 | 2.2 | 156.8 | 70.2× |
| Vector_CANOpenDate.mf4 | 2.2 | 156.8 | 70.2× |
| Vector_FixedLengthStringUTF16_BE.mf4 | 2.2 | 156.7 | 70.1× |
| Vector_FixedLengthStringUTF16_LE.mf4 | 2.2 | 156.8 | 70.2× |
| Vector_FixedLengthStringSBC.mf4 | 2.2 | 156.8 | 70.2× |
| Vector_FixedLengthStringUTF8.mf4 | 2.2 | 156.9 | 70.2× |
| video_sync.mf4 | 2.2 | 156.7 | 70.1× |
| mdflib_array.mf4 | 2.2 | 156.7 | 71.1× |
| Vector_LinearConversion.mf4 | 2.2 | 156.7 | 70.1× |
| gen_structure.mf4 | 2.2 | 155.7 | 70.7× |
| Vector_Value2TextConversion.mf4 | 2.2 | 157.0 | 69.8× |
| Vector_AlgebraicConversionSinus.mf4 | 2.3 | 156.6 | 69.1× |
| Vector_AlgebraicConversionRational.mf4 | 2.3 | 156.7 | 69.2× |
| Vector_RationalConversionRealParams.mf4 | 2.2 | 156.7 | 70.1× |
| Vector_AlgebraicConversionQuadratic.mf4 | 2.3 | 157.3 | 69.4× |
| Vector_ValueRange2TextConversion.mf4 | 2.2 | 156.7 | 69.6× |
| Vector_RationalConversionIntParams.mf4 | 2.2 | 157.0 | 70.3× |
| Vector_RationalConversionZeroedParams.mf4 | 2.2 | 156.9 | 70.2× |
| Vector_ArrayWithFixedAxes.MF4 | 2.3 | 156.7 | 69.2× |
| Vector_Text2ValueConversion.mf4 | 2.2 | 157.0 | 70.2× |
| Vector_Value2ValueConversionInterpolation.mf4 | 2.2 | 156.8 | 70.2× |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 2.2 | 156.8 | 70.2× |
| Vector_Text2TextConversion.mf4 | 2.2 | 156.8 | 69.7× |
| Vector_ValueRange2ValueConversion.mf4 | 2.2 | 156.7 | 70.1× |
| dSPACE_LinearConversion.mf4 | 2.2 | 156.8 | 70.2× |
| dSPACE_AlgebraicConversion.mf4 | 2.3 | 156.8 | 69.2× |
| Vector_AttachmentRef.mf4 | 2.2 | 156.9 | 70.2× |
| dSPACE_Value2TextConversion.mf4 | 2.2 | 157.2 | 69.9× |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 2.2 | 156.7 | 70.1× |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 2.2 | 156.9 | 70.2× |
| dSPACE_ValueRange2TextConversion.mf4 | 2.2 | 156.8 | 69.7× |
| test_batch_cut_0.mf4 | 2.3 | 156.8 | 68.7× |
| Vector_DefaultX.mf4 | 2.2 | 156.7 | 69.6× |
| test_batch_cut_1.mf4 | 2.3 | 156.7 | 68.7× |
| mdflib_big_endian.mf4 | 2.2 | 156.6 | 70.1× |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 2.3 | 157.0 | 69.3× |
| all_datatypes_test.mf4 | 2.4 | 156.9 | 65.6× |
| dSPACE_MeasurementArrays.mf4 | 2.2 | 156.8 | 69.7× |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 2.4 | 156.9 | 65.6× |
| single_lin_bus_1.MF4 | 2.3 | 157.0 | 68.4× |
| single_can_bus_1.MF4 | 2.3 | 157.1 | 67.9× |
| test_batch.mf4 | 2.4 | 156.7 | 66.0× |
| Vector_RealTypes.MF4 | 2.3 | 156.7 | 69.2× |
| simple.mf4 | 2.5 | 156.8 | 63.9× |
| Vector_PartialConversionValueRange2TextRational.mf4 | 2.3 | 157.0 | 68.8× |
| Vector_MeasurementArrays.mf4 | 2.5 | 156.8 | 63.9× |
| dSPACE_Bookmarks.mf4 | 2.3 | 156.7 | 68.7× |
| multiple_fin.MF4 | 2.4 | 157.3 | 65.4× |
| multiple.MF4 | 2.4 | 157.2 | 64.9× |
| mdflib_ethernet.mf4 | 2.5 | 157.0 | 62.4× |
| asammdf_dimensional_demo.mf4 | 2.5 | 157.2 | 64.1× |
| Vector_IntegerTypes.MF4 | 2.4 | 156.6 | 65.5× |
| test_metadata.mf4 | 2.6 | 157.0 | 60.2× |
| dSPACE_RealTypes.mf4 | 2.3 | 156.7 | 67.8× |
| Vector_MinimumFile.MF4 | 2.3 | 156.7 | 67.3× |
| dSPACE_CaptureBlocks.mf4 | 2.3 | 156.8 | 67.8× |
| Vector_CANape.MF4 | 2.3 | 156.6 | 67.3× |
| mdflib_flexray.mf4 | 2.7 | 157.2 | 58.5× |
| Vector_External.MF4 | 2.4 | 156.7 | 66.0× |
| Vector_CustomExtensions_CNcomment.mf4 | 2.4 | 156.7 | 66.4× |
| Vector_EmbeddedCompressed.MF4 | 2.4 | 156.7 | 65.1× |
| Vector_Embedded.MF4 | 2.4 | 157.0 | 65.2× |
| dSPACE_IntegerTypes.mf4 | 2.4 | 156.8 | 64.3× |
| Vector_SingleDZ_TransposeDeflate.mf4 | 3.1 | 156.7 | 50.6× |
| Vector_DataList_TransposeDeflate.mf4 | 2.8 | 156.7 | 55.7× |
| gen_many.mf4 | 3.7 | 157.2 | 42.3× |
| Vector_SingleDZ_Deflate.mf4 | 3.2 | 156.6 | 49.1× |
| mdflib_mixed_compressed.mf4 | 3.7 | 157.1 | 42.1× |
| Vector_DataList_Deflate.mf4 | 2.8 | 156.7 | 55.7× |
| ETAS_SimpleSorted.mf4 | 2.8 | 156.8 | 55.8× |
| gen_xy.mf4 | 2.8 | 156.0 | 56.7× |
| mdflib_mixed.mf4 | 3.9 | 157.4 | 40.3× |
| 00000012-64BB8F50.MF4 | 5.9 | 162.1 | 27.4× |
| ETAS_IntegerTypes.mf4 | 4.5 | 158.1 | 35.5× |
| dSPACE_HILAPITimeout.mf4 | 3.3 | 156.8 | 47.1× |
| dSPACE_HILAPITrigger.mf4 | 3.3 | 157.1 | 47.2× |
| 00000002.MF4 | 7.4 | 163.2 | 22.0× |
| ASAP2_Demo_V171.mf4 | 5.5 | 160.3 | 28.9× |
| 00000013-64BB9AA0.MF4 | 10.2 | 172.1 | 16.9× |
| 00000014-64BBA8AF.MF4 | 12.1 | 182.2 | 15.1× |
| 00002081.MF4 | 28.0 | 194.3 | 7.0× |
| 00002082.MF4 | 28.0 | 194.5 | 7.0× |
| 00002083.MF4 | 27.9 | 194.3 | 7.0× |
| 00002084.MF4 | 27.9 | 194.4 | 7.0× |
| large_deflate.mf4 | 1802.4 | 2365.0 | 1.3× |
| large_uncompressed.mf4 | 1672.4 | 2717.6 | 1.6× |

Both columns are peak resident set size of the whole process, measured with `/usr/bin/time`.
A bare interpreter that only does `import asammdf` already peaks at **154.2 MB**; subtract that to compare decoding cost rather than runtime cost.

### Timing Breakdown

| File | falcon open (ms) | falcon decode (ms) | asammdf open (ms) | asammdf decode (ms) |
|---|---|---|---|---|
| Vector_ByteArrayFixedLength.mf4 | 0.06 | 0.00 | 4.72 | 0.04 |
| Vector_CANOpenTime.mf4 | 0.05 | 0.00 | 4.89 | 0.03 |
| Vector_CANOpenDate.mf4 | 0.05 | 0.00 | 3.99 | 0.07 |
| Vector_FixedLengthStringUTF16_BE.mf4 | 0.05 | 0.01 | 3.92 | 0.03 |
| Vector_FixedLengthStringUTF16_LE.mf4 | 0.05 | 0.00 | 4.95 | 0.03 |
| Vector_FixedLengthStringSBC.mf4 | 0.05 | 0.00 | 4.99 | 0.03 |
| Vector_FixedLengthStringUTF8.mf4 | 0.05 | 0.00 | 4.95 | 0.03 |
| video_sync.mf4 | 0.05 | 0.00 | 4.48 | 0.23 |
| mdflib_array.mf4 | 0.04 | 0.01 | 4.96 | 0.06 |
| Vector_LinearConversion.mf4 | 0.05 | 0.00 | 5.01 | 0.04 |
| gen_structure.mf4 | 0.04 | 0.01 | 4.89 | 0.07 |
| Vector_Value2TextConversion.mf4 | 0.05 | 0.01 | 4.88 | 0.04 |
| Vector_AlgebraicConversionSinus.mf4 | 0.05 | 0.00 | 4.90 | 0.04 |
| Vector_AlgebraicConversionRational.mf4 | 0.06 | 0.00 | 4.67 | 0.06 |
| Vector_RationalConversionRealParams.mf4 | 0.05 | 0.00 | 4.37 | 0.05 |
| Vector_AlgebraicConversionQuadratic.mf4 | 0.06 | 0.00 | 4.95 | 0.05 |
| Vector_ValueRange2TextConversion.mf4 | 0.06 | 0.01 | 4.72 | 0.06 |
| Vector_RationalConversionIntParams.mf4 | 0.05 | 0.00 | 4.08 | 0.05 |
| Vector_RationalConversionZeroedParams.mf4 | 0.06 | 0.00 | 4.93 | 0.05 |
| Vector_ArrayWithFixedAxes.MF4 | 0.06 | 0.00 | 4.90 | 0.10 |
| Vector_Text2ValueConversion.mf4 | 0.06 | 0.01 | 4.92 | 0.04 |
| Vector_Value2ValueConversionInterpolation.mf4 | 0.05 | 0.00 | 4.01 | 0.04 |
| Vector_Value2ValueConversionNoInterpolation.mf4 | 0.05 | 0.00 | 4.85 | 0.06 |
| Vector_Text2TextConversion.mf4 | 0.05 | 0.01 | 4.08 | 0.04 |
| Vector_ValueRange2ValueConversion.mf4 | 0.05 | 0.00 | 5.01 | 0.05 |
| dSPACE_LinearConversion.mf4 | 0.06 | 0.00 | 4.00 | 0.03 |
| dSPACE_AlgebraicConversion.mf4 | 0.06 | 0.00 | 4.80 | 0.04 |
| Vector_AttachmentRef.mf4 | 0.05 | 0.00 | 4.70 | 0.19 |
| dSPACE_Value2TextConversion.mf4 | 0.06 | 0.00 | 4.86 | 0.05 |
| dSPACE_Value2ValueConversionInterpolation.mf4 | 0.06 | 0.00 | 4.61 | 0.05 |
| dSPACE_Value2ValueConversionNoInterpolation.mf4 | 0.06 | 0.00 | 4.47 | 0.06 |
| dSPACE_ValueRange2TextConversion.mf4 | 0.06 | 0.00 | 4.06 | 0.04 |
| test_batch_cut_0.mf4 | 0.07 | 0.01 | 4.77 | 0.08 |
| Vector_DefaultX.mf4 | 0.05 | 0.01 | 4.09 | 0.07 |
| test_batch_cut_1.mf4 | 0.07 | 0.01 | 4.61 | 0.05 |
| mdflib_big_endian.mf4 | 0.05 | 0.01 | 3.99 | 0.06 |
| Vector_PartialConversionLinearIdentityAlgebraic.mf4 | 0.05 | 0.01 | 4.67 | 0.12 |
| all_datatypes_test.mf4 | 0.06 | 0.02 | 4.49 | 0.18 |
| dSPACE_MeasurementArrays.mf4 | 0.06 | 0.01 | 3.98 | 0.10 |
| Vector_StatusStringTableConversionAlgebraic.mf4 | 0.06 | 0.05 | 4.72 | 0.12 |
| single_lin_bus_1.MF4 | 0.07 | 0.01 | 0.63 | 0.30 |
| single_can_bus_1.MF4 | 0.07 | 0.01 | 0.62 | 0.30 |
| test_batch.mf4 | 0.04 | 0.05 | 4.51 | 0.18 |
| Vector_RealTypes.MF4 | 0.06 | 0.01 | 4.59 | 0.10 |
| simple.mf4 | 0.05 | 0.06 | 4.47 | 0.44 |
| Vector_PartialConversionValueRange2TextRational.mf4 | 0.05 | 0.01 | 4.00 | 0.20 |
| Vector_MeasurementArrays.mf4 | 0.09 | 0.02 | 4.42 | 0.47 |
| dSPACE_Bookmarks.mf4 | 0.08 | 0.01 | 3.96 | 0.04 |
| multiple_fin.MF4 | 0.07 | 0.02 | 4.85 | 0.32 |
| multiple.MF4 | 0.08 | 0.02 | 0.72 | 0.36 |
| mdflib_ethernet.mf4 | 0.10 | 0.01 | 5.33 | 0.57 |
| asammdf_dimensional_demo.mf4 | 0.07 | 0.04 | 4.65 | 0.38 |
| Vector_IntegerTypes.MF4 | 0.06 | 0.02 | 4.74 | 0.19 |
| test_metadata.mf4 | 0.14 | 0.02 | 4.80 | 0.39 |
| dSPACE_RealTypes.mf4 | 0.06 | 0.01 | 4.54 | 0.07 |
| Vector_MinimumFile.MF4 | 0.05 | 0.01 | 4.32 | 0.13 |
| dSPACE_CaptureBlocks.mf4 | 0.07 | 0.01 | 4.49 | 0.08 |
| Vector_CANape.MF4 | 0.06 | 0.02 | 4.78 | 0.13 |
| mdflib_flexray.mf4 | 0.14 | 0.02 | 4.94 | 1.13 |
| Vector_External.MF4 | 0.08 | 0.01 | 4.68 | 0.13 |
| Vector_CustomExtensions_CNcomment.mf4 | 0.07 | 0.01 | 3.99 | 0.12 |
| Vector_EmbeddedCompressed.MF4 | 0.08 | 0.01 | 3.96 | 0.12 |
| Vector_Embedded.MF4 | 0.08 | 0.01 | 4.77 | 0.12 |
| dSPACE_IntegerTypes.mf4 | 0.07 | 0.03 | 4.80 | 0.17 |
| Vector_SingleDZ_TransposeDeflate.mf4 | 0.05 | 0.32 | 4.68 | 1.11 |
| Vector_DataList_TransposeDeflate.mf4 | 0.05 | 0.46 | 4.37 | 1.61 |
| gen_many.mf4 | 0.55 | 0.06 | 6.32 | 3.09 |
| Vector_SingleDZ_Deflate.mf4 | 0.05 | 0.48 | 4.62 | 1.23 |
| mdflib_mixed_compressed.mf4 | 0.05 | 0.96 | 4.40 | 3.54 |
| Vector_DataList_Deflate.mf4 | 0.06 | 0.54 | 4.78 | 1.38 |
| ETAS_SimpleSorted.mf4 | 0.06 | 0.08 | 4.56 | 0.24 |
| gen_xy.mf4 | 0.03 | 0.10 | 4.59 | 0.26 |
| mdflib_mixed.mf4 | 0.04 | 0.55 | 4.54 | 1.39 |
| 00000012-64BB8F50.MF4 | 0.75 | 1.47 | 7.30 | 13.48 |
| ETAS_IntegerTypes.mf4 | 0.07 | 0.89 | 4.39 | 1.55 |
| dSPACE_HILAPITimeout.mf4 | 0.08 | 0.31 | 4.79 | 0.34 |
| dSPACE_HILAPITrigger.mf4 | 0.07 | 0.29 | 4.61 | 0.30 |
| 00000002.MF4 | 0.86 | 1.09 | 5.40 | 5.14 |
| ASAP2_Demo_V171.mf4 | 0.23 | 2.31 | 5.93 | 5.17 |
| 00000013-64BB9AA0.MF4 | 1.45 | 4.19 | 12.79 | 19.79 |
| 00000014-64BBA8AF.MF4 | 1.84 | 5.43 | 15.73 | 23.06 |
| 00002081.MF4 | 3.79 | 5.23 | 28.22 | 28.39 |
| 00002082.MF4 | 3.88 | 5.26 | 29.24 | 28.52 |
| 00002083.MF4 | 3.85 | 5.33 | 28.30 | 28.51 |
| 00002084.MF4 | 3.78 | 5.22 | 27.43 | 28.66 |
| large_deflate.mf4 | 0.18 | 1031.45 | 830.27 | 7509.36 |
| large_uncompressed.mf4 | 0.20 | 422.08 | 279.95 | 1127.25 |
