# Copilot instructions

- Read `README.md` and the relevant source, tests, and current operational docs before changing behavior. Treat code and tests as the source of current behavior; check dates and implementation status in planning documents.
- Prefer deterministic tests with simulators and fakes. Never issue BLE commands or calibration against physical motors as part of development or automated checks unless the user explicitly requests a live operation.
- Preserve API, OpenAPI, dashboard, and MCP contract alignment. Update `openapi.json` and relevant contract tests when HTTP routes, schemas, statuses, or responses change.
- Preserve command idempotency, asynchronous completion semantics, and the distinction between a GATT write acknowledgement and observed movement. Do not add arbitrary GATT writes.
- Keep unauthenticated HTTP and MCP interfaces behind loopback or a trusted network boundary; CORS is not authentication.
- Use the focused skills in `.github/skills/`: `rust-service`, `ble-device-protocol`, `api-contract`, `command-state`, `dashboard-ui`, `mcp-adapter`, and `container-release`. The broader repository workflow is in `.agents/skills/somaris-development/SKILL.md`.
- Run checks appropriate to the changed area. CI is defined in `.github/workflows/publish-image.yml`; do not duplicate its checks in another workflow without a concrete need.
