//! Unit tests for the parser module.

use falcon_mdf::parser::Mf4Version;

#[test]
fn test_version_from_version_number() {
    // Test all known versions
    assert!(matches!(
        Mf4Version::from_version_number(400),
        Mf4Version::V4_0 { .. }
    ));
    assert!(matches!(
        Mf4Version::from_version_number(410),
        Mf4Version::V4_1 { .. }
    ));
    assert!(matches!(
        Mf4Version::from_version_number(411),
        Mf4Version::V4_1 { raw: 411 }
    ));
    assert!(matches!(
        Mf4Version::from_version_number(420),
        Mf4Version::V4_2 { .. }
    ));

    // Future version
    let future = Mf4Version::from_version_number(500);
    assert!(!future.is_supported());
}

#[test]
fn test_version_comparison() {
    assert!(Mf4Version::V4_0 { raw: 400 } < Mf4Version::V4_1 { raw: 410 });
    assert!(Mf4Version::V4_1 { raw: 410 } < Mf4Version::V4_2 { raw: 420 });
    assert!(Mf4Version::V4_0 { raw: 400 } < Mf4Version::V4_2 { raw: 420 });
}

#[test]
fn test_version_is_supported() {
    assert!(Mf4Version::V4_0 { raw: 400 }.is_supported());
    assert!(Mf4Version::V4_1 { raw: 410 }.is_supported());
    assert!(Mf4Version::V4_2 { raw: 420 }.is_supported());

    let unknown = Mf4Version::Unknown {
        major: 9,
        minor: 99,
        raw: 999,
    };
    assert!(!unknown.is_supported());
}

#[test]
fn test_version_parts() {
    let v40 = Mf4Version::V4_0 { raw: 400 };
    assert_eq!(v40.major(), 4);
    assert_eq!(v40.minor(), 0);

    let v41 = Mf4Version::V4_1 { raw: 411 };
    assert_eq!(v41.major(), 4);
    assert_eq!(v41.minor(), 11);

    let v42 = Mf4Version::V4_2 { raw: 420 };
    assert_eq!(v42.major(), 4);
    assert_eq!(v42.minor(), 20);

    let unknown = Mf4Version::Unknown {
        major: 5,
        minor: 0,
        raw: 500,
    };
    assert_eq!(unknown.major(), 5);
    assert_eq!(unknown.minor(), 0);
}

#[test]
fn test_version_display() {
    assert_eq!(format!("{}", Mf4Version::V4_0 { raw: 400 }), "4.0");
    assert_eq!(format!("{}", Mf4Version::V4_1 { raw: 411 }), "4.11");
    assert_eq!(format!("{}", Mf4Version::V4_2 { raw: 420 }), "4.20");
    assert_eq!(
        format!(
            "{}",
            Mf4Version::Unknown {
                major: 5,
                minor: 0,
                raw: 500
            }
        ),
        "5.0"
    );
}

#[test]
fn test_version_validate() {
    assert!(Mf4Version::V4_0 { raw: 400 }.validate().is_ok());
    assert!(Mf4Version::V4_1 { raw: 410 }.validate().is_ok());
    assert!(Mf4Version::V4_2 { raw: 420 }.validate().is_ok());

    let unknown = Mf4Version::Unknown {
        major: 9,
        minor: 0,
        raw: 900,
    };
    assert!(unknown.validate().is_err());
}
