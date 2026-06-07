/*!
`tool-call-dedup`: detect and skip duplicate tool calls in agent runs.

An agent loop sometimes calls the same tool with the same arguments more
than once. This crate records canonical `(name, args)` pairs and tells the
caller whether a call is a duplicate so results can be served from cache
or the call can be skipped entirely — saving wasted API spend and latency.

# How matching works

Arguments are compared by *value*, not by their textual representation:

- Object keys are sorted, so `{"a": 1, "b": 2}` and `{"b": 2, "a": 1}` match.
- Comparison is recursive, so nested objects are normalized at every level.
- Arrays are order-sensitive (`[1, 2]` and `[2, 1]` are different calls).
- The tool name is part of the key, so the same arguments to two different
  tools are tracked separately.

# Example

```rust
use tool_call_dedup::CallDedup;
use serde_json::json;

let mut dedup = CallDedup::new();

// First time we see this call, so it is not a duplicate.
assert!(!dedup.is_duplicate("search", &json!({"q": "rust"})));

// The same call again — now flagged as a duplicate.
assert!(dedup.is_duplicate("search", &json!({"q": "rust"})));

// Key order does not matter; this is still the same call.
assert!(dedup.is_duplicate("search", &json!({"q": "rust"})));

assert_eq!(dedup.unique_count(), 1);
assert_eq!(dedup.total_count(), 3);
```

# Typical agent-loop usage

```rust
use tool_call_dedup::CallDedup;
use serde_json::{json, Value};

fn run_tool(_tool: &str, _args: &Value) -> &'static str {
    "result"
}

let mut dedup = CallDedup::new();
let queue = vec![
    ("search", json!({"q": "rust"})),
    ("search", json!({"q": "rust"})), // duplicate, will be skipped
];

for (tool, args) in &queue {
    if dedup.is_duplicate(tool, args) {
        continue; // serve from cache / skip the redundant call
    }
    let _ = run_tool(tool, args);
}
assert_eq!(dedup.unique_count(), 1);
```
*/

use serde_json::Value;
use std::collections::{HashMap, HashSet};

fn canonical_key(tool: &str, args: &Value) -> String {
    // The tool name is quoted/escaped for the same reason object keys are: so a
    // name containing the `:` delimiter cannot blur the boundary between the
    // tool name and the serialized arguments.
    format!("{}:{}", quote_str(tool), canonical_json(args))
}

fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let pairs: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", quote_str(k), canonical_json(&m[*k])))
                .collect();
            format!("{{{}}}", pairs.join(","))
        }
        Value::Array(a) => {
            let items: Vec<String> = a.iter().map(canonical_json).collect();
            format!("[{}]", items.join(","))
        }
        // Strings, numbers, bools and null already serialize unambiguously
        // (strings are JSON-quoted and escaped by `to_string`).
        other => other.to_string(),
    }
}

/// Render a string as a JSON-quoted, escaped literal.
///
/// Object keys are run through this so that delimiter characters (`:`, `,`,
/// `{`, `}`) appearing inside a key cannot forge the structural shape of the
/// canonical string. Without quoting, `{"a:1,b": 2}` and `{"a": 1, "b": 2}`
/// would produce the same key and be treated as the same call.
fn quote_str(s: &str) -> String {
    Value::String(s.to_owned()).to_string()
}

/// Tracks seen `(tool, args)` pairs within a run.
///
/// A `CallDedup` is meant to live for the duration of a single agent run.
/// Create one with [`CallDedup::new`], feed every tool call through
/// [`is_duplicate`](CallDedup::is_duplicate), and call [`reset`](CallDedup::reset)
/// to start over.
#[derive(Debug, Default)]
pub struct CallDedup {
    seen: HashSet<String>,
    counts: HashMap<String, usize>,
}

impl CallDedup {
    /// Create an empty deduplicator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if this exact call has been seen before; records it either way.
    ///
    /// The call is always counted, so calling this in a loop keeps
    /// [`call_count`](Self::call_count) and [`total_count`](Self::total_count)
    /// accurate even for repeated calls.
    ///
    /// ```
    /// use tool_call_dedup::CallDedup;
    /// use serde_json::json;
    ///
    /// let mut d = CallDedup::new();
    /// assert!(!d.is_duplicate("t", &json!({"a": 1})));
    /// assert!(d.is_duplicate("t", &json!({"a": 1})));
    /// ```
    pub fn is_duplicate(&mut self, tool: &str, args: &Value) -> bool {
        let key = canonical_key(tool, args);
        let count = self.counts.entry(key.clone()).or_insert(0);
        *count += 1;
        if self.seen.contains(&key) {
            return true;
        }
        self.seen.insert(key);
        false
    }

    /// Record a call without returning whether it was a duplicate.
    ///
    /// Equivalent to [`is_duplicate`](Self::is_duplicate) with the result
    /// ignored; useful for replaying a log of calls before inspecting counts.
    pub fn record(&mut self, tool: &str, args: &Value) {
        self.is_duplicate(tool, args);
    }

    /// How many times has this exact call been recorded? Returns `0` if unseen.
    pub fn call_count(&self, tool: &str, args: &Value) -> usize {
        let key = canonical_key(tool, args);
        *self.counts.get(&key).unwrap_or(&0)
    }

    /// Number of distinct calls recorded so far.
    pub fn unique_count(&self) -> usize {
        self.seen.len()
    }

    /// Total calls recorded, including duplicates.
    pub fn total_count(&self) -> usize {
        self.counts.values().sum()
    }

    /// All calls that were recorded more than once, as `(canonical_key, count)`
    /// pairs with `count > 1`.
    ///
    /// The order of the returned vector is unspecified.
    pub fn duplicates(&self) -> Vec<(String, usize)> {
        self.counts
            .iter()
            .filter(|(_, &c)| c > 1)
            .map(|(k, &c)| (k.clone(), c))
            .collect()
    }

    /// Clear all recorded history, returning the deduplicator to its initial
    /// empty state so it can be reused for a new run.
    pub fn reset(&mut self) {
        self.seen.clear();
        self.counts.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn first_call_not_duplicate() {
        let mut d = CallDedup::new();
        assert!(!d.is_duplicate("search", &json!({"q": "rust"})));
    }

    #[test]
    fn second_identical_call_is_duplicate() {
        let mut d = CallDedup::new();
        d.is_duplicate("search", &json!({"q": "rust"}));
        assert!(d.is_duplicate("search", &json!({"q": "rust"})));
    }

    #[test]
    fn different_args_not_duplicate() {
        let mut d = CallDedup::new();
        d.is_duplicate("search", &json!({"q": "rust"}));
        assert!(!d.is_duplicate("search", &json!({"q": "python"})));
    }

    #[test]
    fn different_tool_same_args_not_duplicate() {
        let mut d = CallDedup::new();
        d.is_duplicate("search", &json!({"q": "rust"}));
        assert!(!d.is_duplicate("lookup", &json!({"q": "rust"})));
    }

    #[test]
    fn call_count_tracks_repeats() {
        let mut d = CallDedup::new();
        d.is_duplicate("fetch", &json!({"url": "http://example.com"}));
        d.is_duplicate("fetch", &json!({"url": "http://example.com"}));
        d.is_duplicate("fetch", &json!({"url": "http://example.com"}));
        assert_eq!(
            d.call_count("fetch", &json!({"url": "http://example.com"})),
            3
        );
    }

    #[test]
    fn unique_count() {
        let mut d = CallDedup::new();
        d.is_duplicate("a", &json!({}));
        d.is_duplicate("b", &json!({}));
        d.is_duplicate("a", &json!({})); // duplicate
        assert_eq!(d.unique_count(), 2);
    }

    #[test]
    fn total_count_includes_duplicates() {
        let mut d = CallDedup::new();
        d.is_duplicate("a", &json!({}));
        d.is_duplicate("a", &json!({}));
        d.is_duplicate("b", &json!({}));
        assert_eq!(d.total_count(), 3);
    }

    #[test]
    fn duplicates_list() {
        let mut d = CallDedup::new();
        d.is_duplicate("a", &json!({}));
        d.is_duplicate("a", &json!({}));
        d.is_duplicate("b", &json!({}));
        let dups = d.duplicates();
        assert_eq!(dups.len(), 1);
        assert!(dups[0].1 > 1);
    }

    #[test]
    fn key_ordering_is_canonical() {
        let mut d = CallDedup::new();
        d.is_duplicate("get", &json!({"b": 2, "a": 1}));
        // Same args different insertion order
        assert!(d.is_duplicate("get", &json!({"a": 1, "b": 2})));
    }

    #[test]
    fn reset_clears_state() {
        let mut d = CallDedup::new();
        d.is_duplicate("x", &json!(null));
        d.reset();
        assert!(!d.is_duplicate("x", &json!(null)));
        assert_eq!(d.unique_count(), 1);
    }

    #[test]
    fn null_args() {
        let mut d = CallDedup::new();
        assert!(!d.is_duplicate("ping", &Value::Null));
        assert!(d.is_duplicate("ping", &Value::Null));
    }

    #[test]
    fn array_args_canonical() {
        let mut d = CallDedup::new();
        d.is_duplicate("list", &json!([1, 2, 3]));
        assert!(d.is_duplicate("list", &json!([1, 2, 3])));
    }

    #[test]
    fn record_without_return() {
        let mut d = CallDedup::new();
        d.record("ping", &json!({}));
        assert!(d.is_duplicate("ping", &json!({})));
    }

    #[test]
    fn key_with_delimiters_does_not_collide() {
        // Regression: a key containing the structural delimiters used by the
        // canonical encoding must not be confused with a different object.
        let mut d = CallDedup::new();
        assert!(!d.is_duplicate("t", &json!({"a:1,b": 2})));
        // A genuinely different object with two keys must still be unique.
        assert!(!d.is_duplicate("t", &json!({"a": 1, "b": 2})));
        assert_eq!(d.unique_count(), 2);
    }

    #[test]
    fn string_value_with_delimiters_is_distinct_from_structure() {
        let mut d = CallDedup::new();
        assert!(!d.is_duplicate("t", &json!({"k": "1,b:2"})));
        assert!(!d.is_duplicate("t", &json!({"k": 1, "b": 2})));
        assert_eq!(d.unique_count(), 2);
    }

    #[test]
    fn tool_name_with_delimiter_does_not_collide() {
        // The `:` between tool name and args must not be forgeable by a tool
        // name that itself contains a `:`.
        let mut d = CallDedup::new();
        assert!(!d.is_duplicate("a", &json!("b")));
        assert!(!d.is_duplicate("a:\"b\"", &Value::Null));
        assert_eq!(d.unique_count(), 2);
    }

    #[test]
    fn nested_objects_dedup() {
        let mut d = CallDedup::new();
        let a = json!({"outer": {"b": 2, "a": 1}, "list": [3, {"z": 9}]});
        let b = json!({"list": [3, {"z": 9}], "outer": {"a": 1, "b": 2}});
        assert!(!d.is_duplicate("nest", &a));
        // Same content, different key insertion order at every level.
        assert!(d.is_duplicate("nest", &b));
    }

    #[test]
    fn array_order_is_significant() {
        let mut d = CallDedup::new();
        assert!(!d.is_duplicate("t", &json!([1, 2, 3])));
        // Arrays are ordered, so a reordering is a distinct call.
        assert!(!d.is_duplicate("t", &json!([3, 2, 1])));
    }

    #[test]
    fn number_and_string_are_distinct() {
        let mut d = CallDedup::new();
        assert!(!d.is_duplicate("t", &json!({"v": 1})));
        assert!(!d.is_duplicate("t", &json!({"v": "1"})));
        assert_eq!(d.unique_count(), 2);
    }

    #[test]
    fn empty_object_and_null_are_distinct() {
        let mut d = CallDedup::new();
        assert!(!d.is_duplicate("t", &json!({})));
        assert!(!d.is_duplicate("t", &Value::Null));
        assert_eq!(d.unique_count(), 2);
    }

    #[test]
    fn record_increments_count() {
        let mut d = CallDedup::new();
        d.record("ping", &json!({}));
        d.record("ping", &json!({}));
        assert_eq!(d.call_count("ping", &json!({})), 2);
        assert_eq!(d.unique_count(), 1);
        assert_eq!(d.total_count(), 2);
    }

    #[test]
    fn call_count_zero_for_unseen() {
        let d = CallDedup::new();
        assert_eq!(d.call_count("never", &json!({})), 0);
    }

    #[test]
    fn duplicates_reports_correct_count() {
        let mut d = CallDedup::new();
        for _ in 0..4 {
            d.is_duplicate("a", &json!({"x": 1}));
        }
        d.is_duplicate("b", &json!({}));
        let dups = d.duplicates();
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].1, 4);
    }

    #[test]
    fn reset_then_reuse() {
        let mut d = CallDedup::new();
        d.is_duplicate("a", &json!({}));
        d.is_duplicate("a", &json!({}));
        assert_eq!(d.total_count(), 2);
        d.reset();
        assert_eq!(d.total_count(), 0);
        assert_eq!(d.unique_count(), 0);
        assert_eq!(d.call_count("a", &json!({})), 0);
    }
}
