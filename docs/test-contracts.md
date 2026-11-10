# Test contracts

These stable identifiers link regression tests to the behavior they protect. They describe observable behavior; CSS values and component class names remain implementation choices.

## Dashboard

- **UI-PANEL-01 — Panel card layout:** At a 480 CSS-pixel viewport width, panel mode places blind cards in two columns. Each blind command control and each all-blinds command control has a target at least 44 CSS pixels high.
- **UI-PANEL-02 — Panel viewport use:** Panel mode keeps the top bar visible while the blind list scrolls within the available viewport. The document itself does not gain vertical scrolling from the panel content.
- **UI-PANEL-03 — Narrow panel layout:** Below 360 CSS pixels, panel cards stack in one column, the all-blinds controls stack above their actions, and the page has no horizontal overflow. Command targets remain at least 44 CSS pixels high.
- **UI-CONTROL-01 — Accessible and safe controls:** Icon-only actions have an accessible name and an identifiable action. Device labels and confirmation copy are rendered as text, not executable markup. Confirmation controls retain both cancel and confirm actions and respect the disabled state.
- **UI-API-DOC-01 — API reference rendering:** The API reference shows supported operations from `/openapi.json`, renders descriptions as text, and reports an invalid or unavailable document in its status region without rendering partial operation rows.

## Bluetooth cleanup

- **BLE-CLEANUP-01 — Bounded disconnect cleanup:** After a BLE operation, Somaris attempts disconnect cleanup and does not wait on a disconnect indefinitely. Cleanup finishes within the service's documented 10-second command-finalization allowance, including when the adapter returns an error or never completes.

The dashboard contracts complement the user-facing behavior described in the README. BLE-CLEANUP-01 follows the command cleanup limit documented in the README's API section.
