# Test contracts

These stable identifiers link regression tests to the behavior they protect. They describe observable behavior; CSS values and component class names remain implementation choices.

## Dashboard

- **UI-PANEL-01 — Panel card layout:** At a 480 CSS-pixel viewport width, panel mode places blind cards in two columns. Each blind command control and each all-blinds command control has a target at least 44 CSS pixels high.
- **UI-PANEL-02 — Panel viewport use:** Panel mode keeps the top bar visible while the blind list scrolls within the available viewport. The document itself does not gain vertical scrolling from the panel content.
- **UI-PANEL-03 — Narrow panel layout:** Below 360 CSS pixels, panel cards stack in one column, the all-blinds controls stack above their actions, and the page has no horizontal overflow. Command targets remain at least 44 CSS pixels high.
- **UI-PANEL-04 — Square desk panel fit:** At 480×480 CSS pixels, up to four panel cards, their per-blind controls, and all-blinds controls fit in the panel viewport without scrolling. Compact notices retain their full accessible description. Every interactive button, including topbar actions, has a target at least 44×44 CSS pixels.
- **UI-PANEL-05 — Compact demo panel:** The demo banner is hidden in panel mode to preserve card space, while Reset demo remains available as a visibly labelled action in the top bar. At widths below 360px, the top bar and stacked cards do not overflow horizontally.
- **UI-CONTROL-01 — Accessible and safe controls:** Icon-only actions have an accessible name and an identifiable action. Device labels and confirmation copy are rendered as text, not executable markup. Confirmation controls retain both cancel and confirm actions and respect the disabled state.
- **UI-API-DOC-01 — API reference rendering:** The API reference shows supported operations from `/openapi.json`, renders descriptions as text, and reports an invalid or unavailable document in its status region without rendering partial operation rows.
- **UI-PWA-01 — Offline app shell:** The manifest and root-scoped worker are served with the correct content types; the worker caches only versioned static shell assets and bypasses every `/api/*` request.
- **UI-STATUS-01 — Plain-language status with diagnostics:** Main cards use the plain-language availability labels; technical GATT, advertisement-age, travel-rate, failure, and transport details remain available inside the device page’s Advanced disclosure.
- **UI-POSITION-01 — Direct and preset positioning:** Position-capable blinds expose a keyboard-accessible graphic slider, a range fallback, and five labelled percent-open presets. Position commands commit on release and convert to the API's percent-closed contract. The panel view keeps its compact controls.
- **UI-NAV-01 — Mobile navigation:** Mobile tabs show a text label below each icon, mark the active route, and provide 44×44 CSS-pixel targets. Panel mode hides the tab bar.
- **UI-REFRESH-01 — Stable refresh during interaction:** Periodic refresh pauses during a graphic or range drag, while a text input/select/textarea has focus, or while a confirmation is open. Quiet refreshes preserve card controls and focus.
- **UI-INPUT-01 — Inline enrollment validation:** Display names are required, limited to 64 characters, and report errors with `aria-invalid` and `aria-describedby` before the add request is sent.
- **UI-THEME-01 — Opt-in themes:** Dark remains the default; Light and High contrast are explicit choices, while Auto follows system color and contrast preferences.
- **UI-UNDO-01 — Undo positioning and bulk actions:** Position and completed all-blinds actions offer icon-only undo actions. A later command on an affected blind invalidates its bulk undo.
- **UI-PREF-01 — Display and theme selection:** Selecting a supported display mode or theme applies that preference. Unsupported values and unrelated actions do not apply a preference.
- **UI-ROOM-01 — Room grouping:** Assigned blinds are grouped under their room and blinds without a room appear under Unassigned. Grouped card names omit a matching room-name prefix and separator. At a desktop width that supports two columns, a one-blind group uses the same card width as each card in a pair.
- **UI-ROOM-02 — Room management:** Add-blind and blind-details flows offer Unassigned, existing room suggestions, and a custom room name. Room names are trimmed, limited to 40 characters, and reserve Unassigned. A room rename updates all members; removing a room moves them to Unassigned without changing enrollment IDs. Room controls remain keyboard accessible and provide 44 CSS-pixel targets on small screens.

## API

- **API-ROOM-01 — Persisted room assignment:** Enrollment accepts an optional room; the managed-device record exposes the persisted assignment. Updating or clearing a room keeps the enrollment ID and history association.
- **API-ROOM-02 — Derived room lifecycle:** Room listing contains the case-insensitive unique labels in use. Rename updates all members together; removal clears every member's assignment. Invalid, reserved, too-long, missing, and conflicting room names return documented errors.
- **API-BATCH-01 — Selected targets:** Batch requests may specify unique enrollment IDs. Omitted targets preserve the all-enrolled behavior; explicit selections are snapshotted and included in idempotency comparisons.
- **UI-SERVICE-01 — Service summary:** The dashboard exposes a visible action to open service status and renders its summary as text.

## Bluetooth cleanup

- **BLE-CLEANUP-01 — Bounded disconnect cleanup:** After a BLE operation, Somaris attempts disconnect cleanup and does not wait on a disconnect indefinitely. Cleanup finishes within the service's documented 10-second command-finalization allowance, including when the adapter returns an error or never completes.

## Demo session

- **DEMO-RESET-01 — Session reset:** Reset restores the seeded demo household and clears that visitor's command history and activity. An in-flight simulator command does not change the restored state after reset.

The dashboard contracts complement the user-facing behavior described in the README. BLE-CLEANUP-01 follows the command cleanup limit documented in the README's API section.
