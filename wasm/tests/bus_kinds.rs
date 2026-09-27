//! Ethernet and FlexRay logs through the browser binding's frame endpoints,
//! over files `ihedvall/mdflib` wrote (`scripts/make_mdflib_bus_fixtures.sh`).
//! Values follow the formulas in `scripts/mdflib_bus_fixtures/main.cpp`.

mod common;

use common::{parse_json, JsonVal};
use falcon_mdf_wasm::WasmMf4File;

fn open(name: &str) -> Option<WasmMf4File> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../test_data/generated")
        .join(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("skipping: {name} missing; run scripts/make_mdflib_bus_fixtures.sh");
        return None;
    };
    Some(WasmMf4File::new(bytes).expect("opens"))
}

fn field(v: &JsonVal, k: &str) -> JsonVal {
    match v {
        JsonVal::Obj(fields) => fields
            .iter()
            .find_map(|(key, v)| (key == k).then(|| v.clone()))
            .unwrap_or_else(|| panic!("no {k}")),
        other => panic!("expected an object, got {other:?}"),
    }
}

fn rows(page: &str) -> Vec<JsonVal> {
    let page = parse_json(page).expect("page json");
    match field(&page, "rows") {
        JsonVal::Array(rows) => rows,
        other => panic!("rows is an array, got {other:?}"),
    }
}

#[test]
fn ethernet_frames_page_with_ether_type_and_addresses() {
    let Some(file) = open("mdflib_ethernet.mf4") else {
        return;
    };
    let groups = file.bus_groups().unwrap();
    assert!(
        groups.contains("\"kind\":\"eth\",\"group\":0,\"frames\":24"),
        "{groups}"
    );
    let rows = rows(&file.bus_frames_page("eth", 0, 5, 1).unwrap());
    assert_eq!(field(&rows[0], "id"), JsonVal::Number(0x0805 as f64));
    assert_eq!(field(&rows[0], "bus"), JsonVal::Number(3.0));
    assert_eq!(field(&rows[0], "dlc"), JsonVal::Number(6.0));
    assert_eq!(
        field(&rows[0], "detail"),
        JsonVal::Str("02:00:00:00:00:05 → 02:ff:00:00:00:05".into())
    );
    // Frames are 1 ms apart from the first; t sits on frame 7.
    let t0 = match field(&rows[0], "t") {
        JsonVal::Number(t) => t - 0.005,
        other => panic!("{other:?}"),
    };
    assert_eq!(file.bus_frame_locate("eth", 0, t0 + 0.0071).unwrap(), 7);
}

#[test]
fn flexray_frames_page_with_cycle_and_flags() {
    let Some(file) = open("mdflib_flexray.mf4") else {
        return;
    };
    let groups = file.bus_groups().unwrap();
    assert!(
        groups.contains("\"kind\":\"flexray\",\"group\":0,\"frames\":24"),
        "{groups}"
    );
    let rows = rows(&file.bus_frames_page("flexray", 0, 5, 4).unwrap());
    // Frame 5: ID 105, cycle 15, a null frame; frame 8: sync and startup.
    assert_eq!(field(&rows[0], "id"), JsonVal::Number(105.0));
    assert_eq!(
        field(&rows[0], "detail"),
        JsonVal::Str("cycle 15 null".into())
    );
    assert_eq!(
        field(&rows[3], "detail"),
        JsonVal::Str("cycle 24 sync startup".into())
    );
    assert_eq!(field(&rows[3], "ext"), JsonVal::Null);
}

#[test]
fn an_unknown_bus_kind_is_named() {
    let Some(file) = open("mdflib_ethernet.mf4") else {
        return;
    };
    assert!(file.bus_frames_page("most", 0, 0, 1).is_err());
}
