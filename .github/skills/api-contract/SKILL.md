---
name: api-contract
description: Use when changing Somaris HTTP routes, request or response shapes, status codes, OpenAPI, API client behavior, or browser-facing API semantics.
---

# API contract changes

- Read the route implementation in `src/api.rs`, `openapi.json`, and relevant tests in `tests/api_contract.rs`, `tests/api_edges.rs`, and `tests/client_contract.rs`.
- Treat `openapi.json` as the source document. `/openapi.yaml` is derived from it at runtime; do not maintain a second hand-edited YAML contract.
- Write or update a regression test for the observable behavior before changing implementation. Keep examples and UI/MCP consumers aligned with the contract.
- Preserve `Idempotency-Key` requirements for mutating commands and asynchronous command acceptance: callers poll the returned command resource. A successful write acknowledgement does not prove physical movement.
- If dashboard code is affected, update the relevant `ui/tests/` cases and verify generated `ui/*.mjs` files are current.
- Run Rust checks for server/client changes and `npm run check:ui` for UI changes. CI runs the full validation suite.
