//! Native tests for the structure binding: one JSON document carrying the
//! file's internal outline — block count, history, attachments, events,
//! channel hierarchy, data groups → channel groups → channels — the shape
//! the viewer's structure tab draws. The v4 fixture is writer-made; the v3
//! one comes from asammdf like the other format tests.

mod common;

use common::{parse_json, JsonVal};
use falcon_mdf::write::Mf4Writer;
use falcon_mdf_wasm::WasmMf4File;

/// A writer-made file with two groups: a numeric pair and a numeric channel
/// beside a VLSD text channel — the kinds the structure rows must name.
fn structure_file() -> WasmMf4File {
    let t: Vec<f64> = (0..10).map(|i| i as f64 * 0.1).collect();
    let mut writer = Mf4Writer::new();
    {
        let group = writer.add_group(&t).expect("add group");
        group
            .add_channel(
                "Speed",
                "km/h",
                &(0..10).map(|i| i as f64).collect::<Vec<_>>(),
            )
            .expect("speed");
        group
            .add_channel_vlsd_str(
                "State",
                "",
                &[
                    "off", "on", "on", "off", "on", "off", "on", "on", "off", "on",
                ],
            )
            .expect("state");
    }
    {
        let group = writer.add_group(&t).expect("add group 2");
        group
            .add_channel(
                "Counter",
                "",
                &(0..10).map(|i| i as f64 * 2.0).collect::<Vec<_>>(),
            )
            .expect("counter");
    }
    let mut bytes = Vec::new();
    writer.write(&mut bytes).expect("write MF4");
    // Keep a copy where the browser pass can deep-link it (?file=), the same
    // end-to-end path the demo ships to users.
    let _ = std::fs::write("../test_data/generated/gen_structure.mf4", &bytes);
    WasmMf4File::new(bytes).expect("open")
}

fn field_of<'a>(fields: &'a [(String, JsonVal)], key: &str) -> &'a JsonVal {
    fields
        .iter()
        .find_map(|(k, v)| (k == key).then_some(v))
        .unwrap_or_else(|| panic!("no {key} in the object"))
}

fn obj(value: &JsonVal) -> &[(String, JsonVal)] {
    match value {
        JsonVal::Obj(fields) => fields,
        other => panic!("expected an object, got {other:?}"),
    }
}

fn array(value: &JsonVal) -> &[JsonVal] {
    match value {
        JsonVal::Array(items) => items,
        other => panic!("expected an array, got {other:?}"),
    }
}

fn num(value: &JsonVal) -> f64 {
    match value {
        JsonVal::Number(n) => *n,
        other => panic!("expected a number, got {other:?}"),
    }
}

fn str_of(value: &JsonVal) -> &str {
    match value {
        JsonVal::Str(s) => s,
        other => panic!("expected a string, got {other:?}"),
    }
}

fn bool_of(value: &JsonVal) -> bool {
    match value {
        JsonVal::Bool(b) => *b,
        other => panic!("expected a boolean, got {other:?}"),
    }
}

#[test]
fn structure_describes_the_file_outline() {
    let file = structure_file();
    let json = file.structure().expect("structure");
    let parsed = parse_json(&json).expect("the whole document parses as JSON");
    let fields = obj(&parsed);

    assert_eq!(num(field_of(fields, "format")), 4.0);
    assert!(
        str_of(field_of(fields, "version")).starts_with("4."),
        "version names MDF 4"
    );
    // A written file's block walk: at least the ID, HD, the groups' blocks
    // and the data blocks — the exact number is the writer's business.
    let block_count = num(field_of(fields, "block_count"));
    assert!(
        block_count >= 8.0,
        "a written file has a real block walk, got {block_count}"
    );
    assert!(
        str_of(field_of(fields, "id_block")).starts_with("MDF"),
        "the ID block reports its magic"
    );
    assert_eq!(str_of(field_of(fields, "hd_block")), "##HD");

    // Every MF4 file carries at least one history entry (the writer's own).
    assert!(!array(field_of(fields, "history")).is_empty());
    let first = obj(&array(field_of(fields, "history"))[0]);
    assert!(first.iter().any(|(k, _)| k == "time"));
    assert!(first.iter().any(|(k, _)| k == "tool"));

    // The writer writes no attachments, events or hierarchy; the sections
    // exist (so a viewer renders them) and are empty.
    assert!(array(field_of(fields, "attachments")).is_empty());
    assert!(array(field_of(fields, "events")).is_empty());
    assert!(array(field_of(fields, "hierarchy")).is_empty());

    let groups = array(field_of(fields, "data_groups"));
    assert_eq!(groups.len(), 2, "one data group per add_group");
}

#[test]
fn structure_lists_groups_and_channels_with_flags() {
    let file = structure_file();
    let json = file.structure().expect("structure");
    let parsed = parse_json(&json).expect("valid json");
    let fields = obj(&parsed);

    let groups = array(field_of(fields, "data_groups"));
    let first = obj(&groups[0]);
    assert_eq!(num(field_of(first, "index")), 0.0);
    assert!(
        bool_of(field_of(first, "sorted")),
        "writer output is sorted"
    );

    let cgs = array(field_of(first, "channel_groups"));
    assert_eq!(cgs.len(), 1);
    let cg = obj(&cgs[0]);
    assert_eq!(num(field_of(cg, "samples")), 10.0);
    assert!(!bool_of(field_of(cg, "bus")));
    assert!(!bool_of(field_of(cg, "vlsd")));
    assert!(array(field_of(cg, "reductions")).is_empty());

    let channels = array(field_of(cg, "channels"));
    assert_eq!(channels.len(), 3, "Time master + Speed + State");
    let time = obj(&channels[0]);
    assert_eq!(str_of(field_of(time, "name")), "Time");
    assert!(
        bool_of(field_of(time, "master")),
        "the implicit master is marked"
    );
    assert_eq!(str_of(field_of(time, "unit")), "s");
    assert_eq!(str_of(field_of(time, "kind")), "f64");
    assert!(field_of(time, "unreadable").eq(&JsonVal::Null));

    let state = channels
        .iter()
        .map(obj)
        .find(|ch| str_of(field_of(ch, "name")) == "State")
        .expect("the text channel is listed");
    assert_eq!(
        str_of(field_of(state, "kind")),
        "text",
        "VLSD strings report the text kind"
    );
    assert!(!bool_of(field_of(state, "master")));
}

#[test]
fn structure_channel_names_round_trip_through_json_escaping() {
    // A quote in a channel name must not break the document — the same
    // escaping rule every other binding endpoint follows.
    let t = [0.0, 0.1];
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&t).expect("add group");
    group
        .add_channel("weird \"name\" \\ here", "", &[1.0, 2.0])
        .expect("channel");
    let mut bytes = Vec::new();
    writer.write(&mut bytes).expect("write");
    let file = WasmMf4File::new(bytes).expect("open");

    let json = file.structure().expect("structure");
    let parsed = parse_json(&json).expect("the escaped name still parses");
    let fields = obj(&parsed);
    let channels = array(field_of(
        obj(&array(field_of(fields, "data_groups"))[0]),
        "channel_groups",
    ));
    let names: Vec<&str> = array(field_of(obj(&channels[0]), "channels"))
        .iter()
        .map(|ch| str_of(field_of(obj(ch), "name")))
        .collect();
    assert!(names.contains(&"weird \"name\" \\ here"));
}

#[test]
fn structure_carries_every_channel_of_a_wide_group() {
    // 450 channels in one group: the document is complete even where a
    // viewer will cap its drawing (the desktop tree stops at 400 rows and
    // points at the channel list). Also leaves a fixture the browser pass
    // can use to check that cap end to end.
    let t: Vec<f64> = (0..5).map(|i| i as f64 * 0.5).collect();
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&t).expect("add group");
    for c in 0..450 {
        group
            .add_channel(&format!("ch{c:03}"), "", &[0.0; 5])
            .expect("channel");
    }
    let mut bytes = Vec::new();
    writer.write(&mut bytes).expect("write");
    let _ = std::fs::write("../test_data/generated/gen_many.mf4", &bytes);
    let file = WasmMf4File::new(bytes).expect("open");

    let json = file.structure().expect("structure");
    let parsed = parse_json(&json).expect("valid json");
    let fields = obj(&parsed);
    let channels = array(field_of(
        obj(&array(field_of(
            obj(&array(field_of(fields, "data_groups"))[0]),
            "channel_groups",
        ))[0]),
        "channels",
    ));
    assert_eq!(channels.len(), 451, "master + 450 named channels");
    let last = obj(&channels[450]);
    assert_eq!(str_of(field_of(last, "name")), "ch449");
}

/// Writes a small MDF 3.30 file with asammdf, exactly like the mdf3 tests;
/// returns its bytes, or `None` without a usable `.venv`.
fn v3_fixture() -> Option<Vec<u8>> {
    let python = ["../.venv/bin/python", ".venv/bin/python"]
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())?;
    let dir = std::env::temp_dir().join(format!("falcon_wasm_structure_{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("gen_v330_structure.mdf");
    let script = format!(
        r#"
import numpy as np
from asammdf import MDF, Signal

t = np.arange(0.0, 1.0, 0.1)
sigs = [
    Signal(samples=(t * 3.0).astype(np.float64), timestamps=t, name="Speed", unit="km/h"),
    Signal(samples=np.array([b"off"] * 5 + [b"on"] * 5), timestamps=t, name="State", unit=""),
]
m = MDF(version="3.30")
m.append(sigs)
m.save(r"{path}", overwrite=True)
m.close()
"#,
        path = path.display()
    );
    let out = std::process::Command::new(python)
        .arg("-c")
        .arg(&script)
        .output()
        .expect("running asammdf should succeed");
    assert!(
        out.status.success(),
        "asammdf failed to write the v3 fixture: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(std::fs::read(&path).expect("read the written fixture"))
}

#[test]
fn v3_structure_has_groups_but_no_v4_sections() {
    let Some(bytes) = v3_fixture() else {
        panic!("a .venv with asammdf is required — a skipped test is not a passed test")
    };
    let file = WasmMf4File::new(bytes).expect("the sniffed v3 file should open");
    let json = file.structure().expect("structure");
    let parsed = parse_json(&json).expect("valid json");
    let fields = obj(&parsed);

    assert_eq!(num(field_of(fields, "format")), 3.0);
    assert!(str_of(field_of(fields, "version")).starts_with("3."));
    assert!(
        fields.iter().all(|(k, _)| k != "block_count"),
        "v3 has no block walk, so the key is absent"
    );
    assert!(array(field_of(fields, "history")).is_empty());
    assert!(array(field_of(fields, "attachments")).is_empty());
    assert!(array(field_of(fields, "events")).is_empty());
    assert!(array(field_of(fields, "hierarchy")).is_empty());

    let groups = array(field_of(fields, "data_groups"));
    assert_eq!(groups.len(), 1, "asammdf's one append lands in one group");
    let channels = array(field_of(
        obj(&array(field_of(obj(&groups[0]), "channel_groups"))[0]),
        "channels",
    ));
    let names: Vec<&str> = channels
        .iter()
        .map(|ch| str_of(field_of(obj(ch), "name")))
        .collect();
    assert!(names.contains(&"Speed"));
    assert!(names.contains(&"State"));
    let state = channels
        .iter()
        .map(obj)
        .find(|ch| str_of(field_of(ch, "name")) == "State")
        .expect("the text channel is listed");
    assert_eq!(
        str_of(field_of(state, "kind")),
        "text",
        "v3 data type 7 is text"
    );

    // Exactly one master per group: v3's time channel.
    let masters = channels
        .iter()
        .map(obj)
        .filter(|ch| bool_of(field_of(ch, "master")))
        .count();
    assert_eq!(masters, 1, "exactly one time channel, got {masters}");
}
