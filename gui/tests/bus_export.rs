//! What the bus panel writes when someone exports the frame list.
//!
//! A CSV leaves the program and is read by something else — a spreadsheet, a
//! script, a colleague. That makes its header a contract and its quoting a
//! correctness question rather than a cosmetic one: a message name containing
//! a comma silently shifts every column after it, and a column named
//! `message` that carries a value the database never produced is worse than
//! an empty one.
//!
//! These run without a window, against the writers directly.

use falcon_mdf::{CanDatabase, CanFrame, LinFrame, MessageDef};
use falcon_mdf_gui::panels::bus::{csv_field, write_can_csv, write_lin_csv};

const CAN_HEADER_NO_DB: &str = "index,time_ms,id,id_hex,extended,bus_channel,length,data_hex";
const CAN_HEADER_WITH_DB: &str =
    "index,time_ms,id,id_hex,extended,bus_channel,length,data_hex,message";
const LIN_HEADER: &str = "index,time_ms,id,id_hex,extended,bus_channel,length,data_hex";

/// A scratch directory unique to one test: tests run in parallel inside one
/// process, so the name is what keeps them apart.
fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("falcon_mdf_gui_bus_export_{name}"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .unwrap_or_else(|e| panic!("{} should be clearable: {e}", dir.display()));
    }
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|e| panic!("{} should be creatable: {e}", dir.display()));
    dir
}

fn tidy(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn read_lines(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{} should be readable: {e}", path.display()))
        .lines()
        .map(str::to_string)
        .collect()
}

fn can_frame(
    timestamp: f64,
    id: u32,
    extended: Option<bool>,
    data: &'static [u8],
) -> CanFrame<'static> {
    CanFrame {
        timestamp,
        id,
        extended,
        bus_channel: 1,
        data,
    }
}

fn lin_frame(timestamp: f64, id: u8, data: &'static [u8]) -> LinFrame<'static> {
    LinFrame {
        timestamp,
        id,
        bus_channel: 2,
        data,
    }
}

fn sample_database() -> CanDatabase {
    CanDatabase::new(vec![
        MessageDef {
            name: "EngineData".into(),
            id: 0x1f4,
            extended: false,
            length: 8,
            signals: Vec::new(),
        },
        MessageDef {
            name: "Engine, rpm".into(),
            id: 0x200,
            extended: false,
            length: 8,
            signals: Vec::new(),
        },
        MessageDef {
            name: "Engine \"A\"".into(),
            id: 0x300,
            extended: false,
            length: 8,
            signals: Vec::new(),
        },
    ])
}

#[test]
fn the_can_header_without_database_has_no_message_column() {
    let dir = scratch_dir("can_no_db");
    let path = dir.join("can.csv");
    let frames = vec![
        can_frame(0.5, 0x1f4, Some(false), &[0x01, 0x0a]),
        can_frame(1.5, 0x18ff_ee00, Some(true), &[]),
    ];

    write_can_csv(&frames, None, None, &path).expect("the export writes");

    let lines = read_lines(&path);
    assert_eq!(lines[0], CAN_HEADER_NO_DB, "the header is the contract");
    assert_eq!(lines.len(), 3, "one header and one row per frame");

    let width = lines[0].split(',').count();
    for row in &lines[1..] {
        assert_eq!(
            row.split(',').count(),
            width,
            "row {row:?} does not have as many fields as the header"
        );
    }
    assert_eq!(lines[1], "0,500.000000,500,0x1f4,false,1,2,01 0A");
    assert_eq!(lines[2], "1,1500.000000,419425792,0x18ffee00,true,1,0,");
    tidy(&dir);
}

#[test]
fn the_can_header_with_database_includes_message_column() {
    let dir = scratch_dir("can_db");
    let path = dir.join("can.csv");
    let db = sample_database();
    let frames = vec![
        can_frame(0.5, 0x1f4, Some(false), &[0x01, 0x0a]),
        can_frame(1.5, 0x999, Some(true), &[]),
    ];

    write_can_csv(&frames, None, Some(&db), &path).expect("the export writes");

    let lines = read_lines(&path);
    assert_eq!(lines[0], CAN_HEADER_WITH_DB, "the header with DBC");
    assert_eq!(lines.len(), 3, "one header and one row per frame");

    let width = lines[0].split(',').count();
    for row in &lines[1..] {
        assert_eq!(
            row.split(',').count(),
            width,
            "row {row:?} does not have as many fields as the header"
        );
    }
    assert_eq!(
        lines[1], "0,500.000000,500,0x1f4,false,1,2,01 0A,EngineData",
        "row with matching message in DBC"
    );
    assert_eq!(
        lines[2], "1,1500.000000,2457,0x999,true,1,0,,",
        "row without matching message leaves message empty"
    );
    tidy(&dir);
}

#[test]
fn a_row_carries_the_frames_own_numbers() {
    let dir = scratch_dir("can_values");
    let path = dir.join("can.csv");
    // 0.001 s is 1 ms: the column is named for the unit it carries, so the
    // conversion has to actually happen.
    let frames = vec![can_frame(0.001, 0x1f4, Some(false), &[0x01, 0x0a])];

    write_can_csv(&frames, None, None, &path).expect("the export writes");

    let lines = read_lines(&path);
    assert_eq!(
        lines[1], "0,1.000000,500,0x1f4,false,1,2,01 0A",
        "index, milliseconds, decimal and hex identifier, flag, channel, length, payload"
    );
    tidy(&dir);
}

#[test]
fn an_extended_frame_says_so_and_lin_leaves_the_flag_empty() {
    let dir = scratch_dir("extended");
    let can_path = dir.join("can.csv");
    let lin_path = dir.join("lin.csv");

    write_can_csv(
        &[can_frame(0.0, 0x18ff_ee00, Some(true), &[0xff])],
        None,
        None,
        &can_path,
    )
    .expect("the export writes");
    write_lin_csv(&[lin_frame(0.0, 0x3b, &[0xff])], None, &lin_path).expect("the export writes");

    let can_fields: Vec<String> = read_lines(&can_path)[1]
        .split(',')
        .map(str::to_string)
        .collect();
    let lin_fields: Vec<String> = read_lines(&lin_path)[1]
        .split(',')
        .map(str::to_string)
        .collect();

    assert_eq!(can_fields[4], "true", "the file calls this frame extended");
    assert_eq!(
        lin_fields[4], "",
        "LIN has no extended flag, so the column is empty rather than false"
    );
    assert_eq!(lin_fields[3], "0x3b", "the LIN identifier in hex");
    tidy(&dir);
}

#[test]
fn the_lin_header_has_no_message_column() {
    // There is no LIN database in this build, so a message column could never
    // be filled. An always-empty column is noise, not information.
    let dir = scratch_dir("lin_header");
    let path = dir.join("lin.csv");

    write_lin_csv(&[lin_frame(0.25, 0x3b, &[0x11, 0x22])], None, &path).expect("the export writes");

    let lines = read_lines(&path);
    assert_eq!(lines[0], LIN_HEADER);
    assert!(
        !lines[0].contains("message"),
        "LIN must not offer a message column it can never fill"
    );
    assert_eq!(lines[1], "0,250.000000,59,0x3b,,2,2,11 22");

    let width = lines[0].split(',').count();
    assert_eq!(lines[1].split(',').count(), width);
    tidy(&dir);
}

#[test]
fn only_the_filtered_frames_are_written() {
    // The button says "export the frames listed"; the list is what the
    // filters left. Exporting everything would quietly hand back rows the
    // user had already excluded.
    let dir = scratch_dir("filtered");
    let path = dir.join("can.csv");
    let frames = vec![
        can_frame(0.0, 0x100, Some(false), &[0x01]),
        can_frame(1.0, 0x200, Some(false), &[0x02]),
        can_frame(2.0, 0x300, Some(false), &[0x03]),
    ];

    write_can_csv(&frames, Some(&[1, 2]), None, &path).expect("the export writes");

    let lines = read_lines(&path);
    assert_eq!(lines.len(), 3, "two frames survived the filter");
    assert!(lines[1].starts_with("1,"), "the row keeps its own index");
    assert!(lines[2].starts_with("2,"));
    tidy(&dir);
}

#[test]
fn an_empty_selection_writes_the_header_alone() {
    let dir = scratch_dir("empty");
    let can_path_no_db = dir.join("can_no_db.csv");
    let can_path_db = dir.join("can_db.csv");
    let lin_path = dir.join("lin.csv");
    let db = sample_database();

    write_can_csv(&[], None, None, &can_path_no_db).expect("the export writes");
    write_can_csv(&[], None, Some(&db), &can_path_db).expect("the export writes");
    write_lin_csv(&[], None, &lin_path).expect("the export writes");

    assert_eq!(
        read_lines(&can_path_no_db),
        vec![CAN_HEADER_NO_DB.to_string()]
    );
    assert_eq!(
        read_lines(&can_path_db),
        vec![CAN_HEADER_WITH_DB.to_string()]
    );
    assert_eq!(read_lines(&lin_path), vec![LIN_HEADER.to_string()]);
    tidy(&dir);
}

#[test]
fn payload_bytes_keep_their_leading_zero() {
    // `A` instead of `0A` would make a payload unreadable by position, which
    // is how anyone reading hex reads it.
    let dir = scratch_dir("payload");
    let path = dir.join("can.csv");

    write_can_csv(
        &[can_frame(0.0, 1, Some(false), &[0x0a, 0x00, 0xff])],
        None,
        None,
        &path,
    )
    .expect("the export writes");

    assert!(
        read_lines(&path)[1].contains("0A 00 FF"),
        "two-digit uppercase hex, single-spaced"
    );
    tidy(&dir);
}

#[test]
fn a_message_name_with_a_comma_is_quoted() {
    // Unquoted, it would shift every column after it by one.
    assert_eq!(csv_field("Engine, rpm"), "\"Engine, rpm\"");

    let dir = scratch_dir("comma_export");
    let path = dir.join("comma.csv");
    let db = sample_database();
    let frames = vec![can_frame(0.0, 0x200, Some(false), &[0x01])];
    write_can_csv(&frames, None, Some(&db), &path).expect("the export writes");
    let lines = read_lines(&path);
    assert_eq!(
        lines[1],
        "0,0.000000,512,0x200,false,1,1,01,\"Engine, rpm\""
    );
    tidy(&dir);
}

#[test]
fn a_message_name_with_a_quote_has_it_doubled() {
    assert_eq!(csv_field("Engine \"A\""), "\"Engine \"\"A\"\"\"");

    let dir = scratch_dir("quote_export");
    let path = dir.join("quote.csv");
    let db = sample_database();
    let frames = vec![can_frame(0.0, 0x300, Some(false), &[0x01])];
    write_can_csv(&frames, None, Some(&db), &path).expect("the export writes");
    let lines = read_lines(&path);
    assert_eq!(
        lines[1],
        "0,0.000000,768,0x300,false,1,1,01,\"Engine \"\"A\"\"\""
    );
    tidy(&dir);
}

#[test]
fn a_message_name_with_a_newline_is_quoted() {
    assert_eq!(csv_field("Engine\nrpm"), "\"Engine\nrpm\"");
}

#[test]
fn an_ordinary_message_name_is_left_alone() {
    // Quoting everything would be safe but unreadable; quoting only what
    // needs it is the RFC 4180 bargain.
    assert_eq!(csv_field("EngineData"), "EngineData");
    assert_eq!(csv_field(""), "");
}

#[test]
fn a_path_that_cannot_be_created_is_reported_not_panicked() {
    let path = std::path::Path::new("/this/directory/does/not/exist/frames.csv");
    assert!(
        write_can_csv(&[], None, None, path).is_err(),
        "a write that cannot happen must come back as an error"
    );
    assert!(
        write_lin_csv(&[], None, path).is_err(),
        "the LIN writer reports the same way"
    );
}
