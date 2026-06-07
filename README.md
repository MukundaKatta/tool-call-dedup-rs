# tool-call-dedup

[![CI](https://github.com/MukundaKatta/tool-call-dedup-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/MukundaKatta/tool-call-dedup-rs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/tool-call-dedup.svg)](https://crates.io/crates/tool-call-dedup)

Detect and skip duplicate tool calls in agent runs to prevent wasted API spend.

LLM agent loops frequently re-issue the *same* tool call with the *same*
arguments — the model re-reads a file it already read, re-runs an identical
search, re-fetches the same URL. Each redundant call costs latency and money.
`tool-call-dedup` records canonical `(tool, args)` pairs and tells you whether a
call has been seen before, so you can serve the result from cache or skip it.

## Why value-based matching

Arguments are compared by **value**, not by their raw text:

- **Object keys are sorted recursively**, so `{"a": 1, "b": 2}` and
  `{"b": 2, "a": 1}` are treated as the same call.
- **Arrays are order-sensitive** — `[1, 2]` and `[2, 1]` are distinct calls.
- **The tool name is part of the key**, so identical arguments to two different
  tools are tracked separately.
- Keys and the tool name are JSON-escaped before being combined, so a name or
  key that happens to contain the internal delimiters (`:`, `,`, `{`, `}`)
  cannot collide with a structurally different call.

## Install

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
tool-call-dedup = "0.1"
serde_json = "1"
```

Or with cargo:

```sh
cargo add tool-call-dedup serde_json
```

## Usage

```rust
use tool_call_dedup::CallDedup;
use serde_json::json;

let mut dedup = CallDedup::new();

// First time we see this call -> not a duplicate.
assert!(!dedup.is_duplicate("search", &json!({"q": "rust"})));

// Same call again -> duplicate.
assert!(dedup.is_duplicate("search", &json!({"q": "rust"})));

// Key order does not matter; still the same call.
assert!(dedup.is_duplicate("search", &json!({"q": "rust"})));

assert_eq!(dedup.unique_count(), 1);
assert_eq!(dedup.total_count(), 3);
```

### In an agent loop

```rust
use tool_call_dedup::CallDedup;
use serde_json::{json, Value};

fn run_tool(tool: &str, args: &Value) -> String {
    // ... actually invoke the tool ...
    format!("ran {tool} with {args}")
}

let mut dedup = CallDedup::new();
let queue = vec![
    ("search", json!({"q": "rust"})),
    ("search", json!({"q": "rust"})), // duplicate -> skipped
    ("fetch",  json!({"url": "https://example.com"})),
];

for (tool, args) in &queue {
    if dedup.is_duplicate(tool, args) {
        continue; // serve from cache / skip the redundant call
    }
    let _result = run_tool(tool, args);
}

assert_eq!(dedup.unique_count(), 2);
```

## API

All methods are on the [`CallDedup`] struct.

| Method | Description |
| --- | --- |
| `CallDedup::new()` | Create an empty deduplicator. |
| `is_duplicate(tool, args) -> bool` | Record the call and return `true` if it was seen before. |
| `record(tool, args)` | Record a call without returning the duplicate flag. |
| `call_count(tool, args) -> usize` | How many times this exact call was recorded (`0` if unseen). |
| `unique_count() -> usize` | Number of distinct calls recorded. |
| `total_count() -> usize` | Total calls recorded, including duplicates. |
| `duplicates() -> Vec<(String, usize)>` | `(canonical_key, count)` for every call seen more than once. |
| `reset()` | Clear all history to reuse the deduplicator for a new run. |

[`CallDedup`]: https://docs.rs/tool-call-dedup/latest/tool_call_dedup/struct.CallDedup.html

## Development

```sh
cargo build
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

## License

Licensed under the [MIT License](LICENSE).
