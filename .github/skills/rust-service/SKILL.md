---
name: rust-service
description: Use when changing Somaris Rust service logic, HTTP handlers, shared state, client code, MCP adapter, or integrations.
---

# Rust service changes

- Start with the relevant module and its tests. `src/api.rs` owns HTTP routing, simulator behavior, persistence, and command coordination; `src/client.rs`, `src/mcp.rs`, and `src/integration.rs` are adapters around service capabilities.
- Keep asynchronous work bounded and cancellation-safe. Preserve the single-adapter-operation rule for physical Bluetooth access and avoid holding shared-state locks across unrelated awaits.
- Preserve existing error distinctions and observable status codes. Do not turn uncertain external outcomes into confirmed failures or success.
- Add or update the smallest deterministic regression test before implementation when behavior changes. Prefer fakes, in-memory routers, and temporary directories over live services or hardware.
- For Rust changes, run `cargo fmt --all -- --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo test --locked`.
- If HTTP contract behavior changes, follow the `api-contract` skill. For command lifecycle or stored state, also follow `command-state`.
