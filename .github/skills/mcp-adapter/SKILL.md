---
name: mcp-adapter
description: Use when changing the Somaris stdio MCP server, tools, resources, schemas, or its forwarding behavior to the HTTP service.
---

# MCP adapter changes

- Read `src/mcp.rs`, the matching README MCP section, and MCP tests before editing. The MCP process forwards to the HTTP service; do not create a second Bluetooth scanner or duplicate service state.
- Keep MCP tools aligned with the HTTP API and OpenAPI contract. Preserve tool argument validation, response semantics, error mapping, and the live OpenAPI resource.
- Mutating control tools must retain idempotency-key handling and asynchronous command status polling. Uncertain physical writes require telemetry checks before retry.
- Preserve the trusted-network security boundary: MCP and HTTP do not authenticate callers. Do not treat CORS as protection or add simulator configuration tools without an explicit product requirement.
- Add deterministic tool/server tests first for behavior changes, using fakes instead of a live Somaris process or motor. Run the Rust formatting, Clippy, and test commands in `.github/skills/rust-service/SKILL.md`.
