---
name: security-review
description: Use when changing Somaris authentication, control access, network exposure, browser origins, LAN integrations, secrets, file permissions, or deployment security boundaries.
---

# Security-sensitive changes

- Read the relevant security and deployment sections in `README.md`, the implementation, and its contract tests. Treat current code and tests as authoritative; confirm any operational assumption before changing it.
- Map which callers can reach the changed feature and what it can affect. Distinguish administrator authentication for Settings from blind-control authorization: the HTTP control API and MCP do not authenticate callers. CORS is not authentication. Preserve loopback defaults or an explicitly trusted network boundary unless the change adds and tests appropriate control authorization.
- Keep demo-session isolation distinct from user authentication. For demo mutations, preserve cookie scoping, same-origin checks, and the documented session limits; do not present the opaque cookie as proof of identity.
- For LAN device integrations, preserve the explicit outbound-origin allowlist and its validation, TLS certificate verification, disabled redirects, and strict provider configuration parsing. Do not persist credentials in provider configuration or silently broaden the set of reachable hosts.
- For administrator setup, login, and settings, preserve one-time setup, passphrase hashing, bounded sessions, and failed-attempt controls. Do not log setup codes, passphrases, session tokens, or other secrets beyond the intentionally documented first-start setup code.
- Review container and host integration boundaries together. A read-only D-Bus mount still permits bus messages allowed by host policy; do not use privileged containers or broader host permissions as a shortcut.
- Use fakes and temporary directories. Never operate a physical blind, perform calibration, or run live adapter checks for a security test unless the user explicitly requests that live operation.
- For behavior changes, add the smallest deterministic regression test first and confirm it fails for the intended reason. Relevant coverage includes the auth unit tests in `src/auth.rs`, `tests/api_contract.rs`, `tests/api_demo.rs`, `tests/api_edges.rs`, and `tests/shelly_integration.rs`. For workflow or image changes, inspect the release workflow and follow `.github/skills/container-release/SKILL.md`. Follow `.github/skills/rust-service/SKILL.md` for Rust checks.
