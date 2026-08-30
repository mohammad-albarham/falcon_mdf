use falcon_mdf::write::Mf4Writer;
use falcon_mdf_wasm::{escape_json_str, WasmMf4File};

mod common;

use common::{parse_json, JsonVal};

#[test]
fn test_json_escaping_helper() {
    let input = "hello \"world\" \\ path\nnewline\rreturn\ttab\x08back\x0Cform\x01ctrl\x1fend";
    let escaped = escape_json_str(input);
    let expected =
        "hello \\\"world\\\" \\\\ path\\nnewline\\rreturn\\ttab\\bback\\fform\\u0001ctrl\\u001fend";

    assert_eq!(escaped, expected);

    // Verify wrapping in quotes produces a valid JSON string that parses back to input
    let json_str = format!("\"{escaped}\"");
    let parsed = parse_json(&json_str).expect("Valid JSON string");
    assert_eq!(parsed, JsonVal::Str(input.to_string()));
}

#[test]
fn test_signal_output_nan_and_inf_emitted_as_null() {
    let mut writer = Mf4Writer::new();
    let group = writer
        .add_group(&[0.0, 1.0, 2.0, 3.0, 4.0])
        .expect("add group");

    // Add a channel with finite numbers, NaN, +infinity, -infinity
    let values = [1.25, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 42.0];
    group
        .add_channel("Speed_Sensor", "km/h", &values)
        .expect("add channel");

    let mut mdf_bytes = Vec::new();
    writer.write(&mut mdf_bytes).expect("write MF4");

    let file = WasmMf4File::new(mdf_bytes).expect("open WasmMf4File");
    let signal_json = file.signal("Speed_Sensor").expect("read signal");

    // Assert that NaN and inf / Infinity NEVER appear anywhere in the output
    assert!(
        !signal_json.contains("NaN"),
        "JSON must not contain NaN: {signal_json}"
    );
    assert!(
        !signal_json.contains("inf"),
        "JSON must not contain 'inf': {signal_json}"
    );
    assert!(
        !signal_json.contains("Inf"),
        "JSON must not contain 'Inf': {signal_json}"
    );
    assert!(
        !signal_json.contains("INFINITY"),
        "JSON must not contain 'INFINITY': {signal_json}"
    );
    assert!(
        !signal_json.contains("Infinity"),
        "JSON must not contain 'Infinity': {signal_json}"
    );

    // Assert that the entire string parses cleanly as JSON
    let parsed = parse_json(&signal_json).expect("Signal output must parse as valid JSON");

    let JsonVal::Obj(fields) = parsed else {
        panic!("Expected JSON object");
    };

    let name = fields
        .iter()
        .find(|(k, _)| k == "name")
        .map(|(_, v)| v)
        .expect("name field");
    assert_eq!(name, &JsonVal::Str("Speed_Sensor".to_string()));

    let unit = fields
        .iter()
        .find(|(k, _)| k == "unit")
        .map(|(_, v)| v)
        .expect("unit field");
    assert_eq!(unit, &JsonVal::Str("km/h".to_string()));

    let timestamps = fields
        .iter()
        .find(|(k, _)| k == "timestamps")
        .map(|(_, v)| v)
        .expect("timestamps field");
    assert_eq!(
        timestamps,
        &JsonVal::Array(vec![
            JsonVal::Number(0.0),
            JsonVal::Number(1.0),
            JsonVal::Number(2.0),
            JsonVal::Number(3.0),
            JsonVal::Number(4.0),
        ])
    );

    let vals = fields
        .iter()
        .find(|(k, _)| k == "values")
        .map(|(_, v)| v)
        .expect("values field");
    assert_eq!(
        vals,
        &JsonVal::Array(vec![
            JsonVal::Number(1.25),
            JsonVal::Null,
            JsonVal::Null,
            JsonVal::Null,
            JsonVal::Number(42.0),
        ])
    );
}

#[test]
fn test_channel_names_and_info_with_escaping() {
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&[0.0, 1.0]).expect("add group");

    let special_name = "Channel \"quoted\" \\ \n \t \x01";
    let special_unit = "Unit \"°C\" \\ \x02";
    group
        .add_channel(special_name, special_unit, &[10.0, 20.0])
        .expect("add channel");

    let mut mdf_bytes = Vec::new();
    writer.write(&mut mdf_bytes).expect("write MF4");

    let file = WasmMf4File::new(mdf_bytes).expect("open WasmMf4File");

    // channel_count
    assert_eq!(file.channel_count(), 2); // 1 data channel + 1 time master

    // channel_names
    let names_json = file.channel_names().expect("channel_names JSON");
    let names_parsed = parse_json(&names_json).expect("valid JSON array for channel_names");
    let JsonVal::Array(names_arr) = names_parsed else {
        panic!("Expected array");
    };
    assert!(names_arr
        .iter()
        .any(|item| item == &JsonVal::Str(special_name.to_string())));

    // info
    let info_json = file.info().expect("info JSON");
    let info_parsed = parse_json(&info_json).expect("valid JSON object for info");
    let JsonVal::Obj(info_fields) = info_parsed else {
        panic!("Expected object");
    };
    assert!(info_fields.iter().any(|(k, _)| k == "version"));
    assert!(info_fields.iter().any(|(k, _)| k == "start_time"));
    assert!(info_fields.iter().any(|(k, _)| k == "channel_group_count"));
    assert!(info_fields.iter().any(|(k, _)| k == "channel_count"));

    // signal with special name
    let signal_json = file.signal(special_name).expect("read special signal");
    let signal_parsed = parse_json(&signal_json).expect("valid JSON for special signal");
    let JsonVal::Obj(sig_fields) = signal_parsed else {
        panic!("Expected object");
    };
    let name_val = sig_fields
        .iter()
        .find(|(k, _)| k == "name")
        .map(|(_, v)| v)
        .unwrap();
    assert_eq!(name_val, &JsonVal::Str(special_name.to_string()));
    let unit_val = sig_fields
        .iter()
        .find(|(k, _)| k == "unit")
        .map(|(_, v)| v)
        .unwrap();
    assert_eq!(unit_val, &JsonVal::Str(special_unit.to_string()));
}

#[test]
fn test_missing_channel_returns_err() {
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&[0.0, 1.0]).expect("add group");
    group
        .add_channel("Speed", "m/s", &[1.0, 2.0])
        .expect("add channel");

    let mut mdf_bytes = Vec::new();
    writer.write(&mut mdf_bytes).expect("write MF4");

    let file = WasmMf4File::new(mdf_bytes).expect("open WasmMf4File");
    let res = file.signal("NonExistent");
    assert!(res.is_err(), "Non-existent channel must return Err");
}

#[test]
fn test_invalid_and_truncated_bytes_returns_err() {
    let empty_res = WasmMf4File::new(vec![]);
    assert!(empty_res.is_err(), "Empty bytes must return Err");

    let garbage_res = WasmMf4File::new(vec![0x42; 50]);
    assert!(garbage_res.is_err(), "Garbage bytes must return Err");
}

#[test]
fn test_real_reference_file_parsing() {
    let ref_path = std::path::Path::new("test_data/reference");
    if !ref_path.exists() {
        return;
    }
    let entries = std::fs::read_dir(ref_path).expect("read ref dir");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("mf4")
            || path.extension().and_then(|s| s.to_str()) == Some("MF4")
        {
            let bytes = std::fs::read(&path).expect("read reference file");
            if let Ok(file) = WasmMf4File::new(bytes) {
                let info_json = file.info().expect("info JSON");
                parse_json(&info_json).expect("valid info JSON for reference file");

                let names_json = file.channel_names().expect("channel_names JSON");
                let parsed_names = parse_json(&names_json).expect("valid names JSON");
                let JsonVal::Array(names) = parsed_names else {
                    panic!("Expected array");
                };

                for name_val in names.iter().take(5) {
                    if let JsonVal::Str(ch_name) = name_val {
                        if let Ok(sig_json) = file.signal(ch_name) {
                            assert!(!sig_json.contains("NaN"));
                            assert!(!sig_json.contains("inf"));
                            assert!(!sig_json.contains("Infinity"));
                            parse_json(&sig_json).expect("valid signal JSON");
                        }
                    }
                }
            }
        }
    }
}
