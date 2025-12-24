//! Tests for the model layer.

use falcon_mdf::model::{Signal, SignalValue};

#[test]
fn test_signal_empty() {
    let signal = Signal::empty("test_channel".to_string(), "V".to_string());
    
    assert_eq!(signal.channel_name(), "test_channel");
    assert_eq!(signal.unit(), "V");
    assert_eq!(signal.len(), 0);
    assert!(signal.is_empty());
}

#[test]
fn test_signal_with_data() {
    let values = vec![
        SignalValue::Float(1.0),
        SignalValue::Float(2.0),
        SignalValue::Float(3.0),
    ];
    let timestamps = vec![0.0, 0.1, 0.2];
    
    let signal = Signal::new(
        "voltage".to_string(),
        "V".to_string(),
        values,
        Some(timestamps),
    );
    
    assert_eq!(signal.len(), 3);
    assert!(!signal.is_empty());
    
    // Check values via iterator
    let collected: Vec<_> = signal.iter().collect();
    assert_eq!(collected.len(), 3);
}

#[test]
fn test_signal_value_conversion() {
    let int_val = SignalValue::Integer(42);
    let float_val = SignalValue::Float(3.14);
    let bytes_val = SignalValue::Bytes(vec![1, 2, 3, 4]);
    
    assert!(matches!(int_val, SignalValue::Integer(42)));
    assert!(matches!(float_val, SignalValue::Float(x) if (x - 3.14).abs() < 0.001));
    assert!(matches!(bytes_val, SignalValue::Bytes(_)));
}

#[test]
fn test_signal_iterator() {
    let values = vec![
        SignalValue::Float(1.0),
        SignalValue::Float(2.0),
        SignalValue::Float(3.0),
    ];
    let timestamps = vec![0.0, 0.1, 0.2];
    
    let signal = Signal::new(
        "test".to_string(),
        "".to_string(),
        values,
        Some(timestamps),
    );
    
    // Test iteration
    let items: Vec<_> = signal.iter().collect();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].timestamp, Some(0.0));
    assert_eq!(items[2].timestamp, Some(0.2));
}

#[test]
fn test_signal_without_timestamps() {
    let values = vec![
        SignalValue::Integer(100),
        SignalValue::Integer(200),
    ];
    
    let signal = Signal::new(
        "counter".to_string(),
        "count".to_string(),
        values,
        None,
    );
    
    let items: Vec<_> = signal.iter().collect();
    assert!(items[0].timestamp.is_none());
    assert!(items[1].timestamp.is_none());
}
