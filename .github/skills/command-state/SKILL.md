---
name: command-state
description: Use when changing Somaris command queues, idempotency, command or batch lifecycle, persistence, timeouts, or recovery behavior.
---

# Command state and queue changes

- Trace the full lifecycle through `src/api.rs`, `src/controller.rs`, persistent state, and the relevant API/core tests before editing.
- Preserve idempotency behavior, queue limits, per-device fairness/limits, adapter serialization, timeout semantics, and terminal command states. Check README configuration defaults before changing any setting.
- Use temporary data directories and deterministic simulators/fakes for tests. Do not use a physical blind as a fixture.
- For writes that may have reached hardware, preserve `outcome_unknown` semantics and require fresh telemetry before a retry. Only retry failures proven to have happened before the write.
- Add a focused regression test first for each changed lifecycle or recovery behavior. Run `cargo fmt --all -- --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo test --locked`.
- Update `openapi.json`, README configuration/API descriptions, and contract tests when externally visible behavior changes.
