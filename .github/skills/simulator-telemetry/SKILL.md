---
name: simulator-telemetry
description: Use when changing Somaris simulator devices, simulation profiles or scenarios, demo-mode state, telemetry normalization, battery estimates, or history behavior.
---

# Simulator and telemetry changes

- Read the simulator and telemetry implementation in `src/api.rs`, `src/telemetry.rs`, and `src/device_info.rs`, the applicable tests, and the corresponding README sections before editing. Simulator behavior currently lives in the API module; do not assume it has a separate simulator module.
- Keep normal-mode simulators distinct from demo mode. Normal mode provides built-in simulators; demo mode creates session-scoped, pre-enrolled devices. Preserve per-session isolation, reset behavior, and explicit demo-mode signaling.
- Preserve units and observation semantics. API position values are percent closed (`0` is open, `100` is closed); battery values are millivolts with estimated percentages. Simulator command acceptance, write acknowledgement, and observed movement remain separate states.
- Keep simulator scenarios useful for deterministic failure and recovery coverage. Do not make generated timing, battery, or wireless data imply a physical observation, and avoid wall-clock-sensitive assertions when a fake or controlled scenario can exercise the behavior.
- Keep telemetry history behavior aligned with its configured journal, pagination, and persistence contract. When changing externally visible shapes or statuses, follow `.github/skills/api-contract/SKILL.md` and update `openapi.json` and contract tests.
- Use simulators and fakes for tests; never use a physical motor as a simulator fixture. Add the smallest deterministic regression test first for behavior changes. Relevant coverage includes `tests/api_edges.rs`, `tests/api_contract.rs`, `tests/api_demo.rs`, and `tests/core_contract.rs`.
- For physical advertisement or GATT-derived telemetry, also follow `.github/skills/ble-device-protocol/SKILL.md`. Run the Rust formatting, Clippy, and test commands in `.github/skills/rust-service/SKILL.md` for Rust changes.
