//! Unit tests for the parser module.

use falcon_mdf::parser::Mf4Version;

#[test]
fn test_version_from_version_number() {
    // Test all known versions
    assert_eq!(Mf4Version::from_version_number(400), Mf4Version::V4_0);
    assert_eq!(Mf4Version::from_version_number(410), Mf4Version::V4_1);
    assert_eq!(Mf4Version::from_version_number(420), Mf4Version::V4_2);
    
    // Future version
    let future = Mf4Version::from_version_number(500);
    assert!(!future.is_supported());
}

#[test]
fn test_version_comparison() {
    assert!(Mf4Version::V4_0 < Mf4Version::V4_1);
    assert!(Mf4Version::V4_1 < Mf4Version::V4_2);
    assert!(Mf4Version::V4_0 < Mf4Version::V4_2);
}

#[test]
fn test_version_is_supported() {
    assert!(Mf4Version::V4_0.is_supported());
    assert!(Mf4Version::V4_1.is_supported());
    assert!(Mf4Version::V4_2.is_supported());
    
    let unknown = Mf4Version::Unknown(999);
    assert!(!unknown.is_supported());
}

#[test]
fn test_version_supports_bus_events() {
    // Bus events were introduced in 4.1
    assert!(!Mf4Version::V4_0.supports_bus_events());
    assert!(Mf4Version::V4_1.supports_bus_events());
    assert!(Mf4Version::V4_2.supports_bus_events());
}

#[test]
fn test_version_supports_attachment_embedding() {
    // Attachment embedding was introduced in 4.1
    assert!(!Mf4Version::V4_0.supports_attachment_embedding());
    assert!(Mf4Version::V4_1.supports_attachment_embedding());
    assert!(Mf4Version::V4_2.supports_attachment_embedding());
}

#[test]
fn test_version_supports_sorted_data_layout() {
    // Sorted data layout was introduced in 4.2
    assert!(!Mf4Version::V4_0.supports_sorted_data_layout());
    assert!(!Mf4Version::V4_1.supports_sorted_data_layout());
    assert!(Mf4Version::V4_2.supports_sorted_data_layout());
}

#[test]
fn test_version_display() {
    assert_eq!(format!("{}", Mf4Version::V4_0), "MDF 4.0");
    assert_eq!(format!("{}", Mf4Version::V4_1), "MDF 4.1");
    assert_eq!(format!("{}", Mf4Version::V4_2), "MDF 4.2");
    assert_eq!(format!("{}", Mf4Version::Unknown(500)), "MDF Unknown(500)");
}
