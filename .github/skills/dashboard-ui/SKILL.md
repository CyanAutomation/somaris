---
name: dashboard-ui
description: Use when changing Somaris dashboard behavior, accessibility, responsive layouts, browser API interactions, styles, or bundled browser modules.
---

# Dashboard changes

- Read the relevant `.mts` source, matching tests in `ui/tests/`, and dashboard behavior in `README.md` before editing. The TypeScript module files are authored sources; compiled `.mjs` files are embedded by the Rust service and must remain current.
- Preserve accessible names, keyboard behavior, safe text rendering, responsive layouts, and the dashboard's distinctions between last-reported position and accepted commands.
- Add a focused UI regression test before changing observable behavior. Use the existing DOM and Playwright test helpers rather than introducing a parallel test framework.
- Run `npm ci` when dependencies are needed, then `npm run check:ui`. This type-checks source and tests, compiles browser modules, and runs UI tests. Confirm generated `ui/*.mjs` changes are intentional.
- For API behavior changes, follow `.github/skills/api-contract/SKILL.md` too. Never use demo mode or browser controls to operate physical devices as part of automated tests.
