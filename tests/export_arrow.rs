//! Tests for Apache Arrow and Arrow IPC export.

#![cfg(feature = "arrow")]

use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, Int32Type, UInt8Type};
use arrow_array::Array;
use arrow_ipc::reader::FileReader;
use arrow_schema::DataType;
use falcon_mdf::export::{to_record_batch, write_arrow_ipc};
use falcon_mdf::{SignalSeries, SignalValues};
use std::io::Cursor;

fn series(name: &str, timestamps: Vec<f64>, values: SignalValues) -> SignalSeries {
    SignalSeries::from_samples(name, "", timestamps, values, None).unwrap()
}

#[test]
fn to_record_batch_in_memory_series() {
    let times = vec![0.0, 0.5, 1.0];
    let speed = vec![0.0, 12.5, 25.0];
    let rpm = vec![1000i32, 2000i32, 3000i32];
    let gear = vec![1u8, 2u8, 3u8];

    let s1 = series("Speed", times.clone(), SignalValues::F64(speed.clone()));
    let s2 = series("RPM", times.clone(), SignalValues::I32(rpm.clone()));
    let s3 = series("Gear", times.clone(), SignalValues::U8(gear.clone()));

    let batch = to_record_batch(&[s1, s2, s3]).expect("conversion to record batch should succeed");

    assert_eq!(batch.num_rows(), 3);
    assert_eq!(batch.num_columns(), 4);

    let schema = batch.schema();
    assert_eq!(schema.fields().len(), 4);

    // time column
    assert_eq!(schema.field(0).name(), "time");
    assert_eq!(schema.field(0).data_type(), &DataType::Float64);
    assert!(!schema.field(0).is_nullable());

    // Speed column
    assert_eq!(schema.field(1).name(), "Speed");
    assert_eq!(schema.field(1).data_type(), &DataType::Float64);
    assert!(!schema.field(1).is_nullable());

    // RPM column
    assert_eq!(schema.field(2).name(), "RPM");
    assert_eq!(schema.field(2).data_type(), &DataType::Int32);
    assert!(!schema.field(2).is_nullable());

    // Gear column
    assert_eq!(schema.field(3).name(), "Gear");
    assert_eq!(schema.field(3).data_type(), &DataType::UInt8);
    assert!(!schema.field(3).is_nullable());

    // Assert column values
    let time_col = batch.column(0).as_primitive::<Float64Type>();
    assert_eq!(time_col.values(), &[0.0, 0.5, 1.0]);

    let speed_col = batch.column(1).as_primitive::<Float64Type>();
    assert_eq!(speed_col.values(), &[0.0, 12.5, 25.0]);

    let rpm_col = batch.column(2).as_primitive::<Int32Type>();
    assert_eq!(rpm_col.values(), &[1000, 2000, 3000]);

    let gear_col = batch.column(3).as_primitive::<UInt8Type>();
    assert_eq!(gear_col.values(), &[1, 2, 3]);
}

#[test]
fn series_with_validity_bits_produces_nullable_column_and_nulls() {
    let times = vec![0.0, 1.0, 2.0, 3.0];
    let values = vec![10.0, 20.0, 30.0, 40.0];
    let validity = vec![true, false, true, false];

    let s = SignalSeries::from_samples(
        "Sensor",
        "bar",
        times.clone(),
        SignalValues::F64(values),
        Some(validity),
    )
    .unwrap();

    let batch = to_record_batch(&[s]).expect("validity-masked series should convert");

    let schema = batch.schema();
    assert_eq!(schema.field(1).name(), "Sensor");
    assert!(
        schema.field(1).is_nullable(),
        "series with validity mask must be nullable in schema"
    );

    let sensor_col = batch.column(1).as_primitive::<Float64Type>();
    assert_eq!(sensor_col.null_count(), 2);
    assert!(sensor_col.is_valid(0));
    assert!(sensor_col.is_null(1));
    assert!(sensor_col.is_valid(2));
    assert!(sensor_col.is_null(3));
    assert_eq!(sensor_col.value(0), 10.0);
    assert_eq!(sensor_col.value(2), 30.0);
}

#[test]
fn write_arrow_ipc_roundtrip_with_file_reader() {
    let times = vec![0.0, 0.25, 0.5, 0.75];
    let voltage = vec![3.3, 3.31, 3.29, 3.30];
    let current = vec![1.2, 1.5, 1.1, 1.0];

    let s1 = series("Voltage", times.clone(), SignalValues::F64(voltage));
    let s2 = series("Current", times.clone(), SignalValues::F64(current));

    let expected_batch = to_record_batch(&[s1.clone(), s2.clone()]).unwrap();

    let mut buf = Vec::new();
    write_arrow_ipc(&[s1, s2], &mut buf).expect("writing arrow IPC should succeed");
    assert!(!buf.is_empty());

    let cursor = Cursor::new(buf);
    let mut reader = FileReader::try_new(cursor, None).expect("reading arrow IPC file should succeed");
    let read_batch = reader
        .next()
        .expect("should yield one batch")
        .expect("batch read should succeed");

    assert_eq!(read_batch, expected_batch);
    assert!(reader.next().is_none());
}

#[test]
fn exporting_empty_slice_writes_table_with_only_time_column_and_no_rows() {
    let batch = to_record_batch(&[]).expect("empty series slice should succeed");
    assert_eq!(batch.num_rows(), 0);
    assert_eq!(batch.num_columns(), 1);
    assert_eq!(batch.schema().field(0).name(), "time");
    assert_eq!(batch.schema().field(0).data_type(), &DataType::Float64);
    assert!(!batch.schema().field(0).is_nullable());

    let mut buf = Vec::new();
    write_arrow_ipc(&[], &mut buf).expect("writing empty arrow IPC table should succeed");
    let cursor = Cursor::new(buf);
    let mut reader = FileReader::try_new(cursor, None).unwrap();
    let read_batch = reader.next().unwrap().unwrap();
    assert_eq!(read_batch.num_rows(), 0);
    assert_eq!(read_batch.num_columns(), 1);
    assert_eq!(read_batch.schema().field(0).name(), "time");
}

#[test]
fn mismatched_time_axes_are_refused() {
    let s1 = series("Ch1", vec![0.0, 1.0, 2.0], SignalValues::F64(vec![1.0, 2.0, 3.0]));
    let s2 = series("Ch2", vec![0.0, 0.5, 1.0], SignalValues::F64(vec![4.0, 5.0, 6.0]));

    let err = to_record_batch(&[s1, s2]).expect_err("mismatched timestamps must fail");
    let msg = err.to_string();
    assert!(msg.contains("Ch1") && msg.contains("Ch2") && msg.contains("resample"));
}

#[test]
fn unsupported_channel_kind_is_refused() {
    let var_array = series(
        "DynArr",
        vec![0.0, 1.0],
        SignalValues::ArrayVarLen {
            values: vec![1.0, 2.0, 3.0],
            starts: vec![0, 2, 3],
        },
    );
    let err = to_record_batch(&[var_array]).expect_err("variable-length array channels must fail");
    let msg = err.to_string();
    assert!(msg.contains("DynArr") && (msg.contains("variable-length array") || msg.contains("array")));
}

#[test]
fn complex_and_canopen_and_arrays_are_flattened() {
    use arrow_array::types::Int64Type;
    use falcon_mdf::model::values::{CanopenDate, CanopenTime};

    let times = vec![0.0, 1.0];
    let complex = series(
        "Impedance",
        times.clone(),
        SignalValues::Complex {
            re: vec![1.0, 2.0],
            im: vec![0.5, -0.5],
        },
    );
    let date = series(
        "StartDate",
        times.clone(),
        SignalValues::CanopenDate(vec![
            CanopenDate {
                year: 2026,
                month: 8,
                day: 27,
                hour: 10,
                minute: 30,
                ms: 0,
                day_of_week: 4,
                summer_time: true,
            },
            CanopenDate {
                year: 2026,
                month: 8,
                day: 27,
                hour: 10,
                minute: 31,
                ms: 0,
                day_of_week: 4,
                summer_time: true,
            },
        ]),
    );
    let time = series(
        "StartTime",
        times.clone(),
        SignalValues::CanopenTime(vec![
            CanopenTime {
                ms_since_midnight: 3600000,
                days_since_1984: 100,
            },
            CanopenTime {
                ms_since_midnight: 3601000,
                days_since_1984: 100,
            },
        ]),
    );

    let mut array_series = series(
        "Matrix",
        times.clone(),
        SignalValues::Array {
            values: vec![
                10.0, 20.0, 30.0, 40.0, // sample 0: 2x2
                50.0, 60.0, 70.0, 80.0, // sample 1: 2x2
            ],
            elements_per_sample: 4,
        },
    );
    array_series.channel.array_shape = Some(vec![2, 2]);

    let batch = to_record_batch(&[complex, date, time, array_series]).expect("batch with composites");
    assert_eq!(batch.num_rows(), 2);
    // time + Impedance.re + Impedance.im + StartDate + StartTime + Matrix[0][0] + Matrix[0][1] + Matrix[1][0] + Matrix[1][1]
    assert_eq!(batch.num_columns(), 9);

    let schema = batch.schema();
    assert_eq!(schema.field(0).name(), "time");
    assert_eq!(schema.field(1).name(), "Impedance.re");
    assert_eq!(schema.field(2).name(), "Impedance.im");
    assert_eq!(schema.field(3).name(), "StartDate");
    assert_eq!(schema.field(4).name(), "StartTime");
    assert_eq!(schema.field(5).name(), "Matrix[0][0]");
    assert_eq!(schema.field(6).name(), "Matrix[0][1]");
    assert_eq!(schema.field(7).name(), "Matrix[1][0]");
    assert_eq!(schema.field(8).name(), "Matrix[1][1]");

    let re_col = batch.column(1).as_primitive::<Float64Type>();
    assert_eq!(re_col.values(), &[1.0, 2.0]);
    let im_col = batch.column(2).as_primitive::<Float64Type>();
    assert_eq!(im_col.values(), &[0.5, -0.5]);

    let date_col = batch.column(3).as_primitive::<Int64Type>();
    assert_eq!(date_col.values().len(), 2);
    assert_eq!(date_col.values()[1] - date_col.values()[0], 60_000_000_000); // 1 minute in nanos

    let m00 = batch.column(5).as_primitive::<Float64Type>();
    assert_eq!(m00.values(), &[10.0, 50.0]);
    let m01 = batch.column(6).as_primitive::<Float64Type>();
    assert_eq!(m01.values(), &[20.0, 60.0]);
    let m10 = batch.column(7).as_primitive::<Float64Type>();
    assert_eq!(m10.values(), &[30.0, 70.0]);
    let m11 = batch.column(8).as_primitive::<Float64Type>();
    assert_eq!(m11.values(), &[40.0, 80.0]);
}
