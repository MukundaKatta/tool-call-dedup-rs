# tool-call-dedup

Detect and skip duplicate tool calls in agent runs to prevent wasted API spend.

LLM agent loops sometimes invoke the same tool with the same arguments more than
once within a single run. Each redundant call can mean an extra network round
trip, extra latency, and extra cost. `tool-call-dedup` records canonical
`(tool, args)` pairs and tells the caller whether a call has been seen before, so
results can be served from a cache or the call can be skipped entirely.

## How it works

Arguments are serialized into a **canonical** form before hashing: object keys
are sorted recursively, so `{"a": 1, "b": 2}` and `{"b": 2, "a": 1}` map to the
same key. Arrays and scalar values are normalized as well. This means logically
identical calls are recognized as duplicates regardless of JSON field ordering.

The tracker keeps both the set of distinct calls seen and a per-call repeat
count, so you can inspect how often any individual call was repeated.

## Installation

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
tool-call-dedup = "0.1"
serde_json = "1"
```

## Usage

```rust
use tool_call_dedup::CallDedup;
use serde_json::json;

let mut dedup = CallDedup::new();

// First time a (tool, args) pair is seen -> not a duplicate.
assert!(!dedup.is_duplicate("search", &json!({"q": "rust"})));

// Same call again -> duplicate. Serve from cache or skip the call.
assert!(dedup.is_duplicate("search", &json!({"q": "rust"})));

// Key ordering does not matter.
assert!(dedup.is_duplicate("search", &json!({"q": "rust"})));
```

### API overview

| Method | Description |
| --- | --- |
| `CallDedup::new()` | Create an empty tracker. |
| `is_duplicate(tool, args) -> bool` | Returns `true` if the exact call was seen before; records it either way. |
| `record(tool, args)` | Record a call without inspecting the return value. |
| `call_count(tool, args) -> usize` | How many times this exact call has been seen. |
| `unique_count() -> usize` | Number of distinct calls recorded. |
| `total_count() -> usize` | Total calls recorded, including duplicates. |
| `duplicates() -> Vec<(String, usize)>` | All calls seen more than once, with their counts. |
| `reset()` | Clear all recorded history. |

### Typical integration

Wrap your tool dispatch so duplicates are short-circuited:

```rust
use tool_call_dedup::CallDedup;
use serde_json::Value;

fn dispatch(dedup: &mut CallDedup, tool: &str, args: &Value) -> Option<String> {
    if dedup.is_duplicate(tool, args) {
        // Already executed this run; reuse the cached result instead of calling out.
        return None;
    }
    Some(call_tool(tool, args))
}
# fn call_tool(_tool: &str, _args: &Value) -> String { String::new() }
```

## Tech stack

- **Language:** Rust (2021 edition)
- **Dependencies:** [`serde_json`](https://crates.io/crates/serde_json) for argument canonicalization
- **License:** MIT

## Development

```bash
cargo build      # compile the crate
cargo test       # run the test suite
cargo clippy     # lint
```

## License

Licensed under the MIT License.
