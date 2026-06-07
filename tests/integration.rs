//! Integration tests that exercise `CallDedup` through its public API only,
//! the way a downstream crate would use it.

use serde_json::{json, Value};
use tool_call_dedup::CallDedup;

/// Simulate an agent loop that issues some repeated tool calls and only
/// "executes" the ones that are not duplicates.
#[test]
fn agent_loop_skips_repeats() {
    let calls = vec![
        ("search", json!({"q": "rust"})),
        ("search", json!({"q": "rust"})), // duplicate
        ("fetch", json!({"url": "https://example.com"})),
        ("search", json!({"q": "python"})),
        ("fetch", json!({"url": "https://example.com"})), // duplicate
        ("search", json!({"q": "rust"})),                 // duplicate
    ];

    let mut dedup = CallDedup::new();
    let mut executed = 0usize;
    for (tool, args) in &calls {
        if !dedup.is_duplicate(tool, args) {
            executed += 1;
        }
    }

    assert_eq!(executed, 3, "only 3 unique calls should execute");
    assert_eq!(dedup.unique_count(), 3);
    assert_eq!(dedup.total_count(), calls.len());
}

#[test]
fn key_order_independent_across_runs() {
    let mut dedup = CallDedup::new();
    assert!(!dedup.is_duplicate("cfg", &json!({"a": 1, "b": 2, "c": 3})));
    // Same logical arguments, different key order -> duplicate.
    assert!(dedup.is_duplicate("cfg", &json!({"c": 3, "a": 1, "b": 2})));
}

#[test]
fn reset_starts_a_fresh_run() {
    let mut dedup = CallDedup::new();
    dedup.record("ping", &Value::Null);
    dedup.record("ping", &Value::Null);
    assert_eq!(dedup.call_count("ping", &Value::Null), 2);

    dedup.reset();
    assert_eq!(dedup.total_count(), 0);
    assert!(!dedup.is_duplicate("ping", &Value::Null));
}

#[test]
fn duplicates_summary_after_run() {
    let mut dedup = CallDedup::new();
    for _ in 0..3 {
        dedup.record("a", &json!({"x": 1}));
    }
    dedup.record("b", &json!({}));

    let mut dups = dedup.duplicates();
    dups.sort();
    assert_eq!(dups.len(), 1);
    assert_eq!(dups[0].1, 3);
}
