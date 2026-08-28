//! Tests of the public API as a surface, rather than of any one behaviour.
//!
//! These check the things a user notices before they notice a decoding bug:
//! that a type handed back by a public method can be named without reaching
//! into a private module, that `len` and `is_empty` agree, that types are
//! printable and comparable, and that internals stay internal.
//!
//! Everything here compiles against the crate exactly as a dependent would see
//! it — through `falcon_mdf::`, never `crate::`.

use falcon_mdf::{
    BusSignal, BusSignals, CanDatabase, IdMatching, Metadata, Mf4File, Signal, SignalValues,
    UnreadableReason, ValueKind,
};

// A GUI holds `Arc<Mf4File>` and decodes on worker threads, and moves the
// resulting `Signal`s across threads too. `ByteSource` is already `Send +
// Sync` and every cache on `Mf4File` is behind a `RwLock`, so this should
// already hold — this pins it so a future field can't silently break it.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Mf4File>();
    assert_send_sync::<Signal>();
};

#[test]
fn every_type_a_public_method_returns_can_be_named_from_the_crate_root() {
    // `Mf4File::metadata()` returns `&Metadata` and `Channel::unreadable()`
    // returns `Option<UnreadableReason>`. Both were once reachable only through
    // `falcon_mdf::model::…`, so writing the type of a value you were handed
    // meant knowing the module layout.
    fn _takes_metadata(_: &Metadata) {}
    fn _takes_reason(_: UnreadableReason) {}
    fn _takes_values(_: &SignalValues) {}
    fn _takes_kind(_: ValueKind) {}

    // The prelude must offer the same set.
    #[allow(unused_imports)]
    use falcon_mdf::prelude::*;
}

#[test]
// Comparing `len()` to zero is the point here: the assertion is that the two
// agree. Rewriting it as `is_empty()`, which clippy suggests, would compare
// `is_empty()` against itself and check nothing.
#[allow(clippy::len_zero)]
fn len_and_is_empty_agree_on_every_type_that_has_both() {
    // The universal Rust contract: `len() == 0` exactly when `is_empty()`.
    // `Metadata` broke it — a block holding only a comment reported zero
    // properties and non-empty — which is why it now counts properties under a
    // name that says so.
    let values = SignalValues::U8(Vec::new());
    assert_eq!(values.len() == 0, values.is_empty());

    let values = SignalValues::U8(vec![1, 2, 3]);
    assert_eq!(values.len() == 0, values.is_empty());

    let empty = SignalValues::Bytes {
        data: Vec::new(),
        width: 4,
    };
    assert_eq!(empty.len() == 0, empty.is_empty());

    // `Mf4File::decode_bus` hands back both of these, so both owe the contract.
    let signals = BusSignals::default();
    assert_eq!(signals.len() == 0, signals.is_empty());
}

/// `decode_bus` returns `BusSignals<'_>`, whose elements are `BusSignal<'_>`,
/// and takes an `IdMatching`. A caller has to be able to write all three
/// without naming a module.
#[test]
fn the_bus_decoding_types_are_nameable_from_the_crate_root() {
    fn _takes_signals(_: &BusSignals<'_>) {}
    fn _takes_signal(_: &BusSignal<'_>) {}
    fn _takes_matching(_: IdMatching) {}

    // The matching mode a database has unless told otherwise: exact, because
    // treating an ordinary extended identifier as a J1939 parameter group would
    // merge messages that are not the same message.
    assert_eq!(IdMatching::default(), IdMatching::Exact);
    assert_eq!(CanDatabase::default().matching(), IdMatching::Exact);
}

#[test]
fn metadata_distinguishes_having_no_properties_from_being_empty() {
    let comment_only = Metadata::parse("<c><TX>a comment</TX></c>");
    assert_eq!(comment_only.property_count(), 0);
    assert!(
        !comment_only.is_empty(),
        "a block carrying a comment is not empty"
    );
    assert_eq!(comment_only.text(), "a comment");

    let nothing = Metadata::parse("<c/>");
    assert_eq!(nothing.property_count(), 0);
    assert!(nothing.is_empty());
}

#[test]
fn value_kinds_print_as_their_name() {
    // `name()` existed while `Display` did not, so callers wrote
    // `format!("{}", kind.name())` where `{kind}` should work.
    assert_eq!(ValueKind::U32.to_string(), "u32");
    assert_eq!(ValueKind::Bytes.to_string(), "bytes");
    assert_eq!(format!("{}", ValueKind::F64), ValueKind::F64.name());
}

#[test]
fn an_unreadable_reason_prints_its_explanation() {
    let reason = UnreadableReason::ArrayComposition;
    assert_eq!(reason.to_string(), reason.detail());
    assert!(
        reason.to_string().contains("array"),
        "the explanation should say what is wrong: {reason}"
    );
}

#[test]
fn decoded_values_are_comparable_and_printable() {
    // Comparability is what lets a test assert two reads agree; printability is
    // what makes a failure legible.
    let a = SignalValues::U16(vec![1, 2, 3]);
    let b = SignalValues::U16(vec![1, 2, 3]);
    let c = SignalValues::U16(vec![1, 2, 4]);
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert!(format!("{a:?}").contains('1'));

    // Different variants holding equivalent numbers are still different values.
    assert_ne!(SignalValues::U8(vec![1]), SignalValues::U16(vec![1]));
}

#[test]
fn kinds_are_comparable_and_copyable() {
    let kind = ValueKind::I32;
    let copied = kind;
    assert_eq!(kind, copied, "ValueKind should be Copy, not moved");
    assert!(kind.is_numeric());
    assert!(!ValueKind::Str.is_numeric());
    assert!(!ValueKind::Bytes.is_numeric());
}

#[test]
fn metadata_is_comparable_and_defaultable() {
    assert_eq!(Metadata::default(), Metadata::default());
    assert!(Metadata::default().is_empty());
    assert_eq!(Metadata::parse("<c/>"), Metadata::parse("<c/>"));
}

#[test]
fn a_signal_prints_a_summary_rather_than_its_samples() {
    // A signal can hold millions of values; `{:?}` must stay usable.
    let corpus_files = corpus();
    if corpus_files.is_empty() {
        eprintln!("SKIP: no corpus under test_data/");
        return;
    }

    let mut checked = false;
    for path in corpus_files {
        let Ok(file) = falcon_mdf::Mf4File::open(&path) else {
            continue;
        };
        let Some(channel) = file.channels().next() else {
            continue;
        };
        let Ok(signal) = file.signal(channel) else {
            continue;
        };

        let text = format!("{signal:?}");
        assert!(text.starts_with("Signal"), "unexpected debug form: {text}");
        assert!(text.contains(&channel.name), "should name the channel");
        assert!(
            text.len() < 500,
            "debug output should summarise, not dump samples: {} chars",
            text.len()
        );
        checked = true;
        break;
    }
    assert!(checked, "expected at least one readable corpus file");
}

fn corpus() -> Vec<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|s| s.to_str()) == Some("MF4") {
                out.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(std::path::Path::new("test_data"), &mut found);
    if found.is_empty() {
        walk(
            std::path::Path::new("../../falcon_mdf/test_data"),
            &mut found,
        );
    }
    found.sort();
    found
}
