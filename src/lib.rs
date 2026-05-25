/*!
tool-call-dedup: deduplicate repeated identical tool calls in AI agent loops.

Detects when an agent is calling the same tool with the same arguments in a
rolling window. Use this alongside `llm-circuit-breaker` and `tool-loop-guard`
to prevent runaway loops.

```rust
use tool_call_dedup::ToolCallDedup;
use serde_json::json;

let mut dedup = ToolCallDedup::new(10);
assert!(!dedup.is_duplicate("search", &json!({"q": "hello"})));
dedup.record("search", &json!({"q": "hello"}));
assert!(dedup.is_duplicate("search", &json!({"q": "hello"})));
```
*/

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};

// ---- canonical JSON for stable hashing ------------------------------------

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(m) => {
            let sorted: BTreeMap<&String, &Value> = m.iter().collect();
            let out: serde_json::Map<String, Value> = sorted
                .into_iter()
                .map(|(k, v)| (k.clone(), canonical_value(v)))
                .collect();
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(canonical_value).collect()),
        other => other.clone(),
    }
}

fn call_key(name: &str, args: &Value) -> String {
    let canon = canonical_value(args);
    let s = format!("{}:{}", name, serde_json::to_string(&canon).unwrap_or_default());
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

// ---- DedupResult ----------------------------------------------------------

/// Outcome of a `check()` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DedupResult {
    /// This call has not been seen in the current window.
    Novel,
    /// This exact call was seen `count` times in the current window.
    Duplicate { count: usize },
}

impl DedupResult {
    pub fn is_duplicate(&self) -> bool {
        matches!(self, DedupResult::Duplicate { .. })
    }
}

// ---- ToolCallDedup --------------------------------------------------------

/// Rolling-window deduplicator for agent tool calls.
pub struct ToolCallDedup {
    window: usize,
    history: VecDeque<String>,
}

impl ToolCallDedup {
    /// Create a deduplicator with a rolling window of `window` entries.
    pub fn new(window: usize) -> Self {
        Self {
            window: window.max(1),
            history: VecDeque::with_capacity(window),
        }
    }

    /// True if `(name, args)` appears in the current window (without recording it).
    pub fn is_duplicate(&self, name: &str, args: &Value) -> bool {
        let key = call_key(name, args);
        self.history.contains(&key)
    }

    /// Count how many times `(name, args)` appears in the current window.
    pub fn count(&self, name: &str, args: &Value) -> usize {
        let key = call_key(name, args);
        self.history.iter().filter(|k| *k == &key).count()
    }

    /// Record a call in the window (evicts oldest if full).
    pub fn record(&mut self, name: &str, args: &Value) {
        let key = call_key(name, args);
        if self.history.len() == self.window {
            self.history.pop_front();
        }
        self.history.push_back(key);
    }

    /// Check if the call is a duplicate and record it atomically.
    ///
    /// Returns `DedupResult::Novel` if not seen before, or `DedupResult::Duplicate`
    /// with the count of prior occurrences in this window.
    pub fn check(&mut self, name: &str, args: &Value) -> DedupResult {
        let count = self.count(name, args);
        self.record(name, args);
        if count == 0 {
            DedupResult::Novel
        } else {
            DedupResult::Duplicate { count }
        }
    }

    /// Clear the window.
    pub fn clear(&mut self) {
        self.history.clear();
    }

    /// Number of entries currently in the window.
    pub fn len(&self) -> usize {
        self.history.len()
    }

    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }

    pub fn window_size(&self) -> usize {
        self.window
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn novel_first_call() {
        let mut d = ToolCallDedup::new(10);
        let r = d.check("search", &json!({"q": "hi"}));
        assert_eq!(r, DedupResult::Novel);
    }

    #[test]
    fn duplicate_second_call() {
        let mut d = ToolCallDedup::new(10);
        d.record("search", &json!({"q": "hi"}));
        assert!(d.is_duplicate("search", &json!({"q": "hi"})));
    }

    #[test]
    fn different_args_not_duplicate() {
        let mut d = ToolCallDedup::new(10);
        d.record("search", &json!({"q": "hi"}));
        assert!(!d.is_duplicate("search", &json!({"q": "bye"})));
    }

    #[test]
    fn different_name_not_duplicate() {
        let mut d = ToolCallDedup::new(10);
        d.record("search", &json!({"q": "hi"}));
        assert!(!d.is_duplicate("fetch", &json!({"q": "hi"})));
    }

    #[test]
    fn check_returns_duplicate_on_second() {
        let mut d = ToolCallDedup::new(10);
        d.check("search", &json!({}));
        let r = d.check("search", &json!({}));
        assert_eq!(r, DedupResult::Duplicate { count: 1 });
    }

    #[test]
    fn count_matches_occurrences() {
        let mut d = ToolCallDedup::new(10);
        d.record("t", &json!(1));
        d.record("t", &json!(1));
        d.record("t", &json!(1));
        assert_eq!(d.count("t", &json!(1)), 3);
    }

    #[test]
    fn window_evicts_oldest() {
        let mut d = ToolCallDedup::new(3);
        d.record("t", &json!(1)); // evicted after 3 more
        d.record("t", &json!(2));
        d.record("t", &json!(3));
        d.record("t", &json!(4)); // evicts json!(1)
        assert!(!d.is_duplicate("t", &json!(1)));
        assert!(d.is_duplicate("t", &json!(2)));
    }

    #[test]
    fn clear_resets() {
        let mut d = ToolCallDedup::new(10);
        d.record("t", &json!(1));
        d.clear();
        assert!(!d.is_duplicate("t", &json!(1)));
        assert!(d.is_empty());
    }

    #[test]
    fn len_tracks_entries() {
        let mut d = ToolCallDedup::new(10);
        assert_eq!(d.len(), 0);
        d.record("t", &json!(1));
        d.record("t", &json!(2));
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn len_capped_at_window() {
        let mut d = ToolCallDedup::new(3);
        for i in 0..10u32 {
            d.record("t", &json!(i));
        }
        assert_eq!(d.len(), 3);
    }

    #[test]
    fn key_order_independent() {
        let mut d = ToolCallDedup::new(10);
        d.record("t", &json!({"b": 2, "a": 1}));
        assert!(d.is_duplicate("t", &json!({"a": 1, "b": 2})));
    }

    #[test]
    fn is_duplicate_does_not_record() {
        let d = ToolCallDedup::new(10);
        d.is_duplicate("t", &json!({}));
        assert_eq!(d.len(), 0);
    }

    #[test]
    fn dedup_result_is_duplicate_method() {
        assert!(DedupResult::Duplicate { count: 2 }.is_duplicate());
        assert!(!DedupResult::Novel.is_duplicate());
    }

    #[test]
    fn window_size_accessor() {
        let d = ToolCallDedup::new(5);
        assert_eq!(d.window_size(), 5);
    }

    #[test]
    fn window_1_always_evicts() {
        let mut d = ToolCallDedup::new(1);
        d.record("t", &json!(1));
        d.record("t", &json!(2)); // evicts json!(1)
        assert!(!d.is_duplicate("t", &json!(1)));
        assert!(d.is_duplicate("t", &json!(2)));
    }
}
