//! Tests for LDF (LIN Description File) parsing and LIN signal decoding.

use falcon_mdf::candb::CanDatabase;
use falcon_mdf::Mf4File;
use std::path::PathBuf;

const SAMPLE_LDF: &str = r#"
LIN_description_file ;
LIN_protocol_version = "2.1" ;
LIN_language_version = "2.1" ;
LIN_speed = 19.2 kbps ;

Nodes {
    Master: BCM, 5.0 ms, 0.1 ms ;
    Slaves: DoorL, DoorR, Wipers ;
}

Signals {
    DoorLockState: 2, 0, DoorL, BCM ;
    WindowPosition: 10, 0, DoorL, BCM ;
    CabinTemp: 8, 40, DoorL, BCM ;
    WiperSpeed: 3, 0, Wipers, BCM ;
    RainIntensity: 8, 0, Wipers, BCM ;
}

Frames {
    LeftDoorStatus: 0x20, DoorL, 4 {
        DoorLockState, 0 ;
        WindowPosition, 2 ;
        CabinTemp, 16 ;
    }
    WiperStatus: 33, Wipers, 2 {
        WiperSpeed, 0 ;
        RainIntensity, 3 ;
    }
}

Diagnostic_signals {
    MasterReqB0: 8, 0 ;
}

Diagnostic_frames {
    MasterReq: 60 {
        MasterReqB0, 0 ;
    }
}

Signal_encoding_types {
    EncDoorLock {
        logical_value, 0, "Unlocked" ;
        logical_value, 1, "Locked" ;
        logical_value, 2, "Deadlocked" ;
        logical_value, 3, "Fault" ;
    }
    EncWindowPos {
        physical_value, 0, 1000, 0.1, 0.0, "%" ;
    }
    EncCabinTemp {
        physical_value, 0, 255, 0.5, -40.0, "degC" ;
    }
    EncWiperSpeed {
        logical_value, 0, "Off" ;
        logical_value, 1, "Intermittent" ;
        logical_value, 2, "Low" ;
        logical_value, 3, "High" ;
    }
    EncRainIntensity {
        physical_value, 0, 255, 1.0, 0.0, "mm/h" ;
    }
}

Signal_representation {
    EncDoorLock: DoorLockState ;
    EncWindowPos: WindowPosition ;
    EncCabinTemp: CabinTemp ;
    EncWiperSpeed: WiperSpeed ;
    EncRainIntensity: RainIntensity ;
}
"#;

fn resolve_path(rel: &str) -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(rel),
        PathBuf::from("../../falcon_mdf").join(rel),
    ];
    candidates.into_iter().find(|p| p.exists())
}

#[test]
fn ldf_parses_all_frames_and_signals() {
    let db = CanDatabase::from_ldf_str(SAMPLE_LDF).expect("LDF must parse");
    assert_eq!(db.messages().len(), 3); // LeftDoorStatus (0x20), WiperStatus (33), MasterReq (60)

    let door_frame = db.message(0x20).expect("LeftDoorStatus frame");
    assert_eq!(door_frame.name, "LeftDoorStatus");
    assert_eq!(door_frame.length, 4);
    assert_eq!(door_frame.signals.len(), 3);

    let wiper_frame = db.message(33).expect("WiperStatus frame");
    assert_eq!(wiper_frame.name, "WiperStatus");
    assert_eq!(wiper_frame.length, 2);
    assert_eq!(wiper_frame.signals.len(), 2);

    let diag_frame = db.message(60).expect("MasterReq frame");
    assert_eq!(diag_frame.name, "MasterReq");
}

#[test]
fn ldf_decodes_physical_and_logical_signals() {
    let db = CanDatabase::from_ldf_str(SAMPLE_LDF).expect("LDF must parse");

    // DoorLockState = 1 (Locked) -> bits 0..2 = 1
    // WindowPosition = 750 (75.0%) -> bits 2..12 = 750 (750 = 0x02EE -> 0xEE in bits 2..10, 0x02 in bits 10..12)
    // Low 8 bits of payload: 1 | (750 << 2) = 1 | (0x02EE << 2) = 1 | 0x0BB8 -> byte 0: 0xB9, byte 1: 0x0B
    // CabinTemp = 130 -> 130 * 0.5 - 40 = 25.0 degC -> byte 2: 130
    let payload = [0xB9, 0x0B, 130, 0x00];
    let decoded = db.decode(0x20, &payload);

    let lock = decoded.iter().find(|s| s.name == "DoorLockState").unwrap();
    assert_eq!(lock.value, 1.0);
    assert_eq!(lock.text, Some("Locked"));

    let win = decoded.iter().find(|s| s.name == "WindowPosition").unwrap();
    assert_eq!(win.value, 75.0);
    assert_eq!(win.unit, "%");

    let temp = decoded.iter().find(|s| s.name == "CabinTemp").unwrap();
    assert_eq!(temp.value, 25.0);
    assert_eq!(temp.unit, "degC");
}

#[test]
fn ldf_decodes_wiper_frame() {
    let db = CanDatabase::from_ldf_str(SAMPLE_LDF).expect("LDF must parse");

    // WiperSpeed = 2 (Low) -> bits 0..3 = 2
    // RainIntensity = 50 -> bits 3..11 = 50 -> 50 << 3 = 400 = 0x0190
    // Byte 0: 2 | (0x90) = 0x92
    // Byte 1: 0x01
    let payload = [0x92, 0x01];
    let decoded = db.decode(33, &payload);

    let speed = decoded.iter().find(|s| s.name == "WiperSpeed").unwrap();
    assert_eq!(speed.value, 2.0);
    assert_eq!(speed.text, Some("Low"));

    let rain = decoded.iter().find(|s| s.name == "RainIntensity").unwrap();
    assert_eq!(rain.value, 50.0);
    assert_eq!(rain.unit, "mm/h");
}

#[test]
fn decode_lin_from_mf4_reference_file() {
    let Some(path) = resolve_path("test_data/reference/single_lin_bus_1.MF4") else {
        eprintln!("SKIP: single_lin_bus_1.MF4 not found");
        return;
    };

    let file = Mf4File::open(&path).expect("open single_lin_bus_1.MF4");
    let groups = file.lin_frame_groups();
    assert!(!groups.is_empty(), "must find LIN frame groups");

    // Inspect the logged frame IDs in the file
    let frames = file.lin_frames(groups[0]).expect("read LIN frames");
    assert!(!frames.is_empty());

    let mut distinct_ids = std::collections::HashSet::new();
    for f in frames.iter() {
        distinct_ids.insert(f.id);
    }

    // Synthesize an LDF matching the IDs present in the log
    let mut ldf_text = String::from(
        r#"
LIN_description_file ;
LIN_protocol_version = "2.1" ;
LIN_language_version = "2.1" ;
LIN_speed = 19.2 kbps ;

Nodes {
    Master: MasterNode, 5.0 ms, 0.1 ms ;
    Slaves: SlaveNode ;
}

Signals {
"#,
    );

    for &id in &distinct_ids {
        ldf_text.push_str(&format!("    RawByte0_{id}: 8, 0, SlaveNode, MasterNode ;\n"));
    }
    ldf_text.push_str("}\nFrames {\n");
    for &id in &distinct_ids {
        ldf_text.push_str(&format!(
            "    Frame_{id}: {id}, SlaveNode, 8 {{\n        RawByte0_{id}, 0 ;\n    }}\n"
        ));
    }
    ldf_text.push_str("}\n");

    let db = CanDatabase::from_ldf_str(&ldf_text).expect("synthesized LDF must parse");
    let bus_signals = file.decode_lin(&db).expect("decode_lin must succeed");
    assert!(!bus_signals.is_empty());

    for signal in bus_signals.iter() {
        assert_eq!(signal.timestamps.len(), signal.values.len());
        assert!(!signal.values.is_empty());
    }
}
