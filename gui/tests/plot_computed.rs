//! Tests for computed channels: expression parsing, operator precedence, evaluation over signals,
//! timebase resampling, division by zero, unknown channel error handling, and session roundtripping.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use falcon_mdf::{Mf4File, Mf4Writer};
use falcon_mdf_gui::computed::{
    eval_expr, evaluate_visible_defs, parse_expr, ComputedDef,
};
use falcon_mdf_gui::model::ChannelLoc;
use falcon_mdf_gui::panels::plot::cursor_measurement;
use falcon_mdf_gui::session::{format_line, parse_line, Session};
use falcon_mdf_gui::signal_loader::ChannelSignal;

fn make_signal(name: &str, unit: &str, times: Vec<f64>, values: Vec<f64>, valid: Option<Vec<bool>>) -> ChannelSignal {
    ChannelSignal {
        loc: ChannelLoc {
            data_group_index: 0,
            channel_group_index: 0,
            channel_index: 0,
        },
        name: name.to_string(),
        unit: unit.to_string(),
        time_name: "Time".to_string(),
        time_unit: "s".to_string(),
        times,
        values,
        valid,
    }
}

#[test]
fn eval_scalar_arithmetic_precedence() {
    let empty_signals = HashMap::new();

    // 2 + 3 * 4 = 14 (multiplication before addition)
    let e1 = parse_expr("2 + 3 * 4").unwrap();
    let res1 = eval_expr("calc", "", &e1, &empty_signals).unwrap();
    assert_eq!(res1.values[0], 14.0);

    // (2 + 3) * 4 = 20 (parentheses override precedence)
    let e2 = parse_expr("(2 + 3) * 4").unwrap();
    let res2 = eval_expr("calc", "", &e2, &empty_signals).unwrap();
    assert_eq!(res2.values[0], 20.0);

    // 10 - 4 - 2 = 4 (left-associative subtraction: (10 - 4) - 2)
    let e3 = parse_expr("10 - 4 - 2").unwrap();
    let res3 = eval_expr("calc", "", &e3, &empty_signals).unwrap();
    assert_eq!(res3.values[0], 4.0);

    // 100 / 10 / 2 = 5 (left-associative division: (100 / 10) / 2)
    let e4 = parse_expr("100 / 10 / 2").unwrap();
    let res4 = eval_expr("calc", "", &e4, &empty_signals).unwrap();
    assert_eq!(res4.values[0], 5.0);

    // Unary negation and binary arithmetic: -5 + 3 = -2, -(3 + 4) * 2 = -14
    let e5 = parse_expr("-5 + 3").unwrap();
    let res5 = eval_expr("calc", "", &e5, &empty_signals).unwrap();
    assert_eq!(res5.values[0], -2.0);

    let e6 = parse_expr("-(3 + 4) * 2").unwrap();
    let res6 = eval_expr("calc", "", &e6, &empty_signals).unwrap();
    assert_eq!(res6.values[0], -14.0);
}

#[test]
fn eval_pointwise_channel_math() {
    let voltage = make_signal("Voltage", "V", vec![0.0, 1.0, 2.0], vec![10.0, 12.0, 14.0], None);
    let current = make_signal("Current", "A", vec![0.0, 1.0, 2.0], vec![2.0, 3.0, 4.0], None);

    let mut signals = HashMap::new();
    signals.insert("Voltage".to_string(), &voltage);
    signals.insert("Current".to_string(), &current);

    let expr = parse_expr("Voltage * Current").unwrap();
    let power = eval_expr("Power", "W", &expr, &signals).unwrap();

    assert_eq!(power.name, "Power");
    assert_eq!(power.unit, "W");
    assert_eq!(power.times, vec![0.0, 1.0, 2.0]);
    assert_eq!(power.values, vec![20.0, 36.0, 56.0]);
    assert_eq!(power.valid, None);
}

#[test]
fn eval_complex_expression_with_constants_and_brackets() {
    let fl = make_signal("Wheel Speed FL", "km/h", vec![0.0, 1.0], vec![100.0, 105.0], None);
    let fr = make_signal("Wheel Speed FR", "km/h", vec![0.0, 1.0], vec![96.0, 99.0], None);

    let mut signals = HashMap::new();
    signals.insert("Wheel Speed FL".to_string(), &fl);
    signals.insert("Wheel Speed FR".to_string(), &fr);

    let expr = parse_expr("([Wheel Speed FL] - [Wheel Speed FR]) / 2.0").unwrap();
    let diff = eval_expr("SpeedDelta", "km/h", &expr, &signals).unwrap();

    assert_eq!(diff.name, "SpeedDelta");
    assert_eq!(diff.times, vec![0.0, 1.0]);
    assert_eq!(diff.values, vec![2.0, 3.0]);
}

#[test]
fn eval_unknown_channel_error_path() {
    let sig = make_signal("EngineSpeed", "rpm", vec![0.0, 1.0], vec![1000.0, 2000.0], None);
    let mut signals = HashMap::new();
    signals.insert("EngineSpeed".to_string(), &sig);

    // Expression refers to "NonExistentChannel" which is not loaded
    let expr = parse_expr("EngineSpeed + NonExistentChannel").unwrap();
    let result = eval_expr("test", "", &expr, &signals);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("NonExistentChannel"),
        "error message must name the missing channel: {err}"
    );
}

#[test]
fn eval_syntax_error_malformed_expression() {
    assert!(parse_expr("").is_err());
    assert!(parse_expr("   ").is_err());
    assert!(parse_expr("1 +").is_err());
    assert!(parse_expr("(1 + 2").is_err());
    assert!(parse_expr("1 + 2)").is_err());
    assert!(parse_expr("1 @ 2").is_err());
    assert!(parse_expr("1 2").is_err());
    assert!(parse_expr("[unclosed").is_err());
    assert!(parse_expr("[]").is_err());
}

#[test]
fn eval_division_by_zero_safe_handling() {
    let sig = make_signal("Val", "", vec![0.0, 1.0, 2.0], vec![10.0, 20.0, 30.0], None);
    let mut signals = HashMap::new();
    signals.insert("Val".to_string(), &sig);

    let expr = parse_expr("Val / 0.0").unwrap();
    let res = eval_expr("DivZero", "", &expr, &signals).unwrap();

    assert_eq!(res.valid, Some(vec![false, false, false]));
    assert!(res.values.iter().all(|v| v.is_nan()));
}

#[test]
fn eval_resample_across_different_timebases() {
    let slow = make_signal("Slow", "m", vec![0.0, 1.0, 2.0], vec![0.0, 10.0, 20.0], None);
    let fast = make_signal("Fast", "m", vec![0.0, 0.5, 1.0, 1.5, 2.0], vec![1.0, 2.0, 3.0, 4.0, 5.0], None);

    let mut signals = HashMap::new();
    signals.insert("Slow".to_string(), &slow);
    signals.insert("Fast".to_string(), &fast);

    let expr = parse_expr("Slow + Fast").unwrap();
    let sum = eval_expr("Sum", "m", &expr, &signals).unwrap();

    assert_eq!(sum.times, vec![0.0, 0.5, 1.0, 1.5, 2.0]);
    // Slow linearly interpolated onto Fast's timestamps:
    // at t=0.0 -> 0.0, t=0.5 -> 5.0, t=1.0 -> 10.0, t=1.5 -> 15.0, t=2.0 -> 20.0
    // Sum = Slow + Fast = [1.0, 7.0, 13.0, 19.0, 25.0]
    assert_eq!(sum.values, vec![1.0, 7.0, 13.0, 19.0, 25.0]);
}

#[test]
fn computed_channel_survives_session_roundtrip() {
    let path = PathBuf::from("/data/vehicle_log.mf4");
    let original = Session {
        plotted: vec![ChannelLoc {
            data_group_index: 0,
            channel_group_index: 0,
            channel_index: 1,
        }],
        nav: "Channels".to_string(),
        tab: "Plot".to_string(),
        cursor_a: Some(1.5),
        cursor_b: Some(3.5),
        computed: vec![
            ComputedDef::new("Power_kW", "Torque * EngineSpeed / 9549.0", "kW"),
            ComputedDef::new("Slip", "[Wheel Speed FL] - [Wheel Speed RL]", "km/h"),
        ],
    };

    let formatted = format_line(&path, &original);
    let (read_path, read_session) = parse_line(&formatted).expect("session line must parse");

    assert_eq!(read_path, path);
    assert_eq!(read_session, original);
    assert_eq!(read_session.computed.len(), 2);
    assert_eq!(read_session.computed[0].name, "Power_kW");
    assert_eq!(read_session.computed[0].expression, "Torque * EngineSpeed / 9549.0");
    assert_eq!(read_session.computed[0].unit, "kW");
    assert_eq!(read_session.computed[1].name, "Slip");
    assert_eq!(read_session.computed[1].expression, "[Wheel Speed FL] - [Wheel Speed RL]");
    assert_eq!(read_session.computed[1].unit, "km/h");
}

#[test]
fn cursors_measure_computed_signal_accurately() {
    let voltage = make_signal("Voltage", "V", vec![0.0, 1.0, 2.0, 3.0], vec![10.0, 12.0, 14.0, 16.0], None);
    let current = make_signal("Current", "A", vec![0.0, 1.0, 2.0, 3.0], vec![2.0, 3.0, 4.0, 5.0], None);

    let mut signals = HashMap::new();
    signals.insert("Voltage".to_string(), &voltage);
    signals.insert("Current".to_string(), &current);

    let expr = parse_expr("Voltage * Current").unwrap();
    let power = eval_expr("Power", "W", &expr, &signals).unwrap();

    // Power values: [20.0, 36.0, 56.0, 80.0] at [0.0, 1.0, 2.0, 3.0]
    // Cursor A at t=1.0, Cursor B at t=3.0
    let m = cursor_measurement(&power.times, &power.values, power.valid.as_deref(), Some(1.0), Some(3.0));

    assert_eq!(m.value_a, Some(36.0));
    assert!(m.valid_a);
    assert_eq!(m.value_b, Some(80.0));
    assert!(m.valid_b);
    assert_eq!(m.delta_t, Some(2.0));
    assert_eq!(m.delta_y, Some(44.0)); // 80.0 - 36.0 = 44.0
}

/// Writes a small two-channel file and opens it: the fixture the evaluation
/// cache tests run against.
fn two_channel_file(name: &str) -> Arc<Mf4File> {
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&[0.0, 1.0, 2.0]).unwrap();
    group.add_channel("A", "V", &[1.0, 2.0, 3.0]).unwrap();
    group.add_channel("B", "V", &[10.0, 20.0, 30.0]).unwrap();

    let path = std::env::temp_dir().join(format!("falcon_gui_computed_{name}.mf4"));
    writer.write_to_file(&path).unwrap();
    let file = Mf4File::open(&path).expect("written file should open");
    let _ = std::fs::remove_file(&path);
    Arc::new(file)
}

#[test]
fn a_definition_that_is_not_plotted_is_not_evaluated() {
    // Hiding a definition is how a user keeps a formula around without paying
    // for it: it must cost nothing per frame — no parse that matters, no
    // operand decodes, no cache entry.
    let file = two_channel_file("hidden");
    let hidden = ComputedDef {
        visible: false,
        ..ComputedDef::new("Hidden", "A + B", "V")
    };

    let mut operand_cache = HashMap::new();
    let mut result_cache = HashMap::new();
    let out = evaluate_visible_defs(
        std::slice::from_ref(&hidden),
        &file,
        1,
        &mut operand_cache,
        &mut result_cache,
    );

    assert!(out.is_empty(), "a hidden definition produces no series");
    assert!(
        operand_cache.is_empty(),
        "a hidden definition must not even decode its operands"
    );
    assert!(result_cache.is_empty());
}

#[test]
fn an_unchanged_definition_reuses_its_cached_result() {
    // The whole point of the cache: the second frame of an unchanged plot
    // must not rebuild the union timebase and re-evaluate every sample. The
    // reused result is the same allocation, not an equal copy.
    let file = two_channel_file("cached");
    let def = ComputedDef::new("Sum", "A + B", "V");

    let mut operand_cache = HashMap::new();
    let mut result_cache = HashMap::new();

    let first = evaluate_visible_defs(std::slice::from_ref(&def), &file, 1, &mut operand_cache, &mut result_cache);
    assert_eq!(first.len(), 1);
    let first_signal = first[0].1.as_ref().expect("evaluation should succeed").clone();
    assert_eq!(first_signal.values, vec![11.0, 22.0, 33.0]);
    assert_eq!(result_cache.len(), 1);
    let decoded_operands = operand_cache.len();

    let second = evaluate_visible_defs(std::slice::from_ref(&def), &file, 1, &mut operand_cache, &mut result_cache);
    assert_eq!(second.len(), 1);
    let second_signal = second[0].1.as_ref().expect("cached evaluation should succeed").clone();

    assert!(
        Arc::ptr_eq(&first_signal, &second_signal),
        "nothing changed, so the cached result must be reused, not recomputed"
    );
    assert_eq!(result_cache.len(), 1);
    assert_eq!(operand_cache.len(), decoded_operands);
}

#[test]
fn editing_or_hiding_a_definition_drops_its_cached_result() {
    let file = two_channel_file("edited");
    let original = ComputedDef::new("Sum", "A + B", "V");

    let mut operand_cache = HashMap::new();
    let mut result_cache = HashMap::new();
    let first = evaluate_visible_defs(std::slice::from_ref(&original), &file, 1, &mut operand_cache, &mut result_cache);
    let first_signal = first[0].1.as_ref().expect("evaluation should succeed").clone();

    // Same name, different expression: a different definition, which must
    // re-evaluate rather than serve the stale sum.
    let edited = ComputedDef::new("Sum", "A - B", "V");
    let second = evaluate_visible_defs(std::slice::from_ref(&edited), &file, 1, &mut operand_cache, &mut result_cache);
    let second_signal = second[0].1.as_ref().expect("evaluation should succeed").clone();

    assert_eq!(second_signal.values, vec![-9.0, -18.0, -27.0]);
    assert!(!Arc::ptr_eq(&first_signal, &second_signal));
    assert_eq!(result_cache.len(), 1, "the stale entry must be pruned, not piled up");
}

#[test]
fn a_result_cached_for_another_file_is_never_served() {
    // Switching files keeps the panel alive, so the cache has to treat a
    // different file identity as a total invalidation — serving the old
    // file's computed samples over the new file's plot would be silent wrong
    // data.
    let file = two_channel_file("file_switch");
    let def = ComputedDef::new("Sum", "A + B", "V");

    let mut operand_cache = HashMap::new();
    let mut result_cache = HashMap::new();
    let first = evaluate_visible_defs(std::slice::from_ref(&def), &file, 1, &mut operand_cache, &mut result_cache);
    let first_signal = first[0].1.as_ref().expect("evaluation should succeed").clone();

    let second = evaluate_visible_defs(std::slice::from_ref(&def), &file, 2, &mut operand_cache, &mut result_cache);
    let second_signal = second[0].1.as_ref().expect("evaluation should succeed").clone();

    assert_eq!(second_signal.values, vec![11.0, 22.0, 33.0]);
    assert!(
        !Arc::ptr_eq(&first_signal, &second_signal),
        "a different file id must invalidate the cached result"
    );
}
