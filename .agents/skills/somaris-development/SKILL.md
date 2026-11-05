---
name: somaris-development
description: Use when implementing, reviewing, or debugging Somaris, including its Rust service, HTTP API, BLE control, MCP adapter, dashboard, integrations, tests, or container workflow. Follow Somaris command-safety, contract, and deployment constraints.
---

# Somaris development

Use this skill for changes to the Somaris repository. Read `README.md` and inspect the current implementation and tests for the area being changed; repository behavior and test coverage are more authoritative than assumptions based on names or older design notes.

## Repository map

- `src/api.rs`: HTTP routes, application state, simulator, persistence, command queue, and API documentation handlers.
- `src/controller.rs` and `src/ble/`: controller boundary, physical Bluetooth adapter, and Soma advertisement protocol.
- `src/client.rs`: HTTP client used to communicate with the service.
- `src/mcp.rs`: stdio MCP tools and resources, forwarded through the HTTP API.
- `src/integration.rs`: transport-neutral device capabilities and process-local state-change notifications.
- `ui/`: browser dashboard and its JavaScript and CSS components.
- `openapi.json`: source contract for the HTTP API. `/openapi.yaml` is generated from this document when served.
- `tests/`: Rust contract, edge, physical-adapter, BLE, client, core, and asset tests. `ui/tests/`: dashboard unit tests.
- `docs/`: operational assessments and integration planning; check dates and implementation status before treating proposals as current behavior.

## Device and service constraints

- Physical commands can move real blinds. Use simulators and test doubles for development and automated checks. Do not issue commands, run calibration, or perform live adapter checks against a physical motor unless the user explicitly asks for that operation.
- Physical devices must be freshly observed and explicitly enrolled before control. Preserve the distinction between adapter readiness, fresh advertisement, recent GATT outcome, and movement observed in telemetry.
- Mutating control requests require an `Idempotency-Key`. Acceptance is asynchronous; clients must poll the command or batch resource.
- A GATT write acknowledgement confirms the write response, not movement. If a write may have started or its outcome is `outcome_unknown`, check fresh telemetry before retrying. Keep automatic retries limited to failures proven to have occurred before a write.
- Position values are percent closed: `0` is open and `100` is closed. Preserve this meaning in API payloads, UI labels, MCP arguments, tests, and documentation.
- Preserve supervised endpoint calibration sequencing and its movement lock. Motor-speed changes require SOMA firmware 2.2.0 or later. Do not expose arbitrary GATT writes.
- The HTTP API and MCP tools do not authenticate callers. Keep loopback or trusted-network deployment boundaries; CORS does not provide authentication. Do not imply otherwise in UI, API, or deployment changes.

## Change workflow

1. Read the relevant source, tests, and current README or operational docs. Check for existing contract expectations before changing behavior.
2. For behavior changes, write or update the smallest relevant regression test first, confirm it fails for the intended reason, then implement the change. Prefer simulators, fakes, and deterministic tests; never make a hardware command a test fixture.
3. For HTTP route, schema, status, or response changes, update `openapi.json` and the relevant API contract tests. Check both served OpenAPI formats; YAML is derived from the JSON document.
4. For MCP changes, preserve the HTTP API as the service boundary and update the MCP unit tests and documented tool contract. Do not add simulator configuration tools to MCP without an explicit product requirement.
5. For dashboard behavior, update the relevant `ui/tests/` cases and keep browser behavior aligned with API semantics, especially telemetry freshness and asynchronous command outcomes.
6. For BLE changes, cover protocol decoding and adapter outcomes with tests. Keep uncertain writes distinct from confirmed pre-write failures, and preserve serialized adapter access and safe cancellation behavior.
7. For integration changes, remember that state-change notifications are bounded, process-local hints rather than a durable event log; consumers must refresh authoritative state after startup or lag.
8. Run focused checks first. Before finishing, run the same relevant checks as CI:

   ```sh
   cargo fmt --all -- --check
   cargo clippy --all-targets --locked -- -D warnings
   cargo test --locked
   node --test ui/tests/*.test.mjs
   ```

   Container or dependency-workflow changes should also be checked against `.github/workflows/publish-image.yml` and the relevant Docker or Compose configuration.

When a requested change conflicts with a device-safety or deployment constraint, explain the conflict and propose the smallest safe alternative before changing that boundary.
