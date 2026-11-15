---
name: ble-device-protocol
description: Use when changing Somaris BLE discovery, advertisement parsing, GATT operations, device enrollment, telemetry, or physical adapter behavior.
---

# BLE device protocol changes

- Read `src/ble/`, `src/controller.rs`, `src/device_info.rs`, and the corresponding BLE and physical-adapter tests before editing. Treat tests and current source as the record of supported protocol behavior.
- Use simulators and test doubles. Never issue commands, calibrate, or run live adapter checks against a physical motor unless the user explicitly asks for that operation.
- Keep physical devices freshly observed and explicitly enrolled before control. Preserve the separate meanings of adapter readiness, advertisement freshness, GATT outcome, and movement observed in telemetry.
- Treat a GATT acknowledgement as confirmation of the write response only. Keep uncertain write outcomes distinct; automatically retry only failures known to have occurred before a write.
- Preserve serialized adapter access, bounded scan windows, safe cancellation, and bounded disconnect cleanup. Do not add arbitrary GATT writes or percentage-position commands.
- Add deterministic tests for protocol decoding and adapter outcomes first when behavior changes. Run the Rust formatting, Clippy, and test commands in `.github/skills/rust-service/SKILL.md`.
