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
    let complex = series(
        "ComplexCh",
        vec![0.0, 1.0],
        SignalValues::Complex {
            re: vec![1.0, 2.0],
            im: vec![0.5, -0.5],
        },
    );
    let err = to_record_batch(&[complex]).expect_err("complex channels must fail");
    let msg = err.to_string();
    assert!(msg.contains("ComplexCh") && msg.contains("complex"));
}
