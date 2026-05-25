/*!
tool-call-dedup: detect and skip duplicate tool calls in agent runs.

An agent loop sometimes calls the same tool with the same arguments more
than once. This crate records canonical (name, args) pairs and tells the
caller whether a call is a duplicate so results can be served from cache
or the call can be skipped entirely.

```rust
use tool_call_dedup::CallDedup;
use serde_json::json;

let mut dedup = CallDedup::new();
assert!(!dedup.is_duplicate("search", &json!({"q": "rust"})));
assert!(dedup.is_duplicate("search", &json!({"q": "rust"})));
```
*/

use serde_json::Value;
use std::collections::{HashMap, HashSet};

fn canonical_key(tool: &str, args: &Value) -> String {
    format!("{}:{}", tool, canonical_json(args))
}

fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let pairs: Vec<String> = keys.iter()
                .map(|k| format!("{}:{}", k, canonical_json(&m[*k])))
                .collect();
            format!("{{{}}}", pairs.join(","))
        }
        Value::Array(a) => {
            let items: Vec<String> = a.iter().map(canonical_json).collect();
            format!("[{}]", items.join(","))
        }
        other => other.to_string(),
    }
}

/// Tracks seen (tool, args) pairs within a run.
#[derive(Debug, Default)]
pub struct CallDedup {
    seen: HashSet<String>,
    counts: HashMap<String, usize>,
}

impl CallDedup {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if this exact call has been seen before; records it either way.
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

    /// Record a call without checking for duplicates.
    pub fn record(&mut self, tool: &str, args: &Value) {
        self.is_duplicate(tool, args);
    }

    /// How many times has this exact call been seen?
    pub fn call_count(&self, tool: &str, args: &Value) -> usize {
        let key = canonical_key(tool, args);
        *self.counts.get(&key).unwrap_or(&0)
    }

    /// Total distinct calls recorded.
    pub fn unique_count(&self) -> usize {
        self.seen.len()
    }

    /// Total calls recorded (including duplicates).
    pub fn total_count(&self) -> usize {
        self.counts.values().sum()
    }

    /// All duplicate counts (key -> count) where count > 1.
    pub fn duplicates(&self) -> Vec<(String, usize)> {
        self.counts.iter()
            .filter(|(_, &c)| c > 1)
            .map(|(k, &c)| (k.clone(), c))
            .collect()
    }

    /// Clear all history.
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
        assert_eq!(d.call_count("fetch", &json!({"url": "http://example.com"})), 3);
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
}
