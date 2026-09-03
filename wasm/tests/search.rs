//! Native tests for the channel-name search binding (plan 2.3): the mode
//! strings, their matching semantics, and the error contract. The reader's
//! own matcher is tested upstream; what is pinned here is that the binding
//! maps `"contains"` / `"wildcard"` / `"exact"` onto it honestly and refuses
//! everything else (regex is a JS-side mode in the demo and never reaches
//! this function).

mod common;

use common::parse_json;
use falcon_mdf::write::Mf4Writer;
use falcon_mdf_wasm::WasmMf4File;

/// A file whose channel names exercise the three modes' differences: case,
/// wildcards, and word boundaries inside one name set.
fn search_file() -> WasmMf4File {
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&[0.0, 1.0, 2.0]).expect("add group");
    for name in [
        "Engine.RPM",
        "engine.rpm_raw",
        "WheelSpeedFL",
        "WHEELSPEEDFR",
        "X",
    ] {
        group
            .add_channel(name, "", &[1.0, 2.0, 3.0])
            .expect("add channel");
    }
    let mut bytes = Vec::new();
    writer.write(&mut bytes).expect("write MF4");
    WasmMf4File::new(bytes).expect("open")
}

fn names_of(file: &WasmMf4File, pattern: &str, mode: &str) -> Vec<String> {
    let json = file
        .search_channels(pattern, mode)
        .unwrap_or_else(|e| panic!("search({pattern}, {mode}) failed: {e:?}"));
    match parse_json(&json).expect("valid json") {
        common::JsonVal::Array(items) => items
            .into_iter()
            .map(|v| match v {
                common::JsonVal::Str(s) => s,
                other => panic!("names are strings, got {other:?}"),
            })
            .collect(),
        other => panic!("search result is an array, got {other:?}"),
    }
}

#[test]
fn contains_is_a_case_insensitive_substring() {
    let file = search_file();
    // Both case spellings hit both channels; the reader sorts the results.
    assert_eq!(
        names_of(&file, "wheelspeed", "contains"),
        vec!["WHEELSPEEDFR".to_string(), "WheelSpeedFL".to_string()]
    );
    assert_eq!(
        names_of(&file, "RPM", "contains"),
        vec!["Engine.RPM".to_string(), "engine.rpm_raw".to_string()]
    );
    // The empty pattern contains-matches everything: the five data channels
    // plus the group's auto-created master "t".
    assert_eq!(names_of(&file, "", "contains").len(), 6);
}

#[test]
fn wildcard_is_whole_name_with_stars_and_question_marks() {
    let file = search_file();
    // Unanchored substring semantics do not apply: "RPM" alone matches
    // nothing whole-name; the star does.
    assert!(names_of(&file, "RPM", "wildcard").is_empty());
    assert_eq!(
        names_of(&file, "*RPM", "wildcard"),
        vec!["Engine.RPM".to_string()]
    );
    assert_eq!(
        names_of(&file, "WheelSpeedF?", "wildcard"),
        vec!["WheelSpeedFL".to_string()]
    );
    assert_eq!(
        names_of(&file, "?ngine.RPM", "wildcard"),
        vec!["Engine.RPM".to_string()]
    );
}

#[test]
fn exact_matches_one_name_and_nothing_else() {
    let file = search_file();
    assert_eq!(names_of(&file, "X", "exact"), vec!["X".to_string()]);
    // Case matters in exact mode: the lower-case twin stays out.
    assert!(names_of(&file, "x", "exact").is_empty());
    assert_eq!(
        names_of(&file, "engine.rpm_raw", "exact"),
        vec!["engine.rpm_raw".to_string()]
    );
}

#[test]
fn an_unknown_mode_is_an_error_not_a_silent_contains() {
    let file = search_file();
    assert!(file.search_channels("RPM", "regex").is_err());
    assert!(file.search_channels("RPM", "").is_err());
    assert!(file.search_channels("RPM", "Contains").is_err());
}
