# Scheduled automations: future-phase report and plan

**Status:** planning only. Scheduled automations are intentionally not implemented in this change set.

## Report

### Opportunity

Scheduled blind movement is a useful next step for Somaris: people can set a blind to open or close at a chosen time without relying on a phone being present. The first release should provide a small, dependable recurring schedule feature rather than a general-purpose automation language.

SOMA documents a device-side Morning Mode feature for supported firmware, but Somaris should not depend on an undocumented motor command or assume every motor supports it. A Somaris schedule can instead submit the same supported open, close, or percentage-position command that a user can submit manually. Treat native Morning Mode as a separate future capability if its protocol and readback behavior are verified. See [SOMA firmware updates](https://support.somasmarthome.com/hc/en-us/articles/360014397560-Firmware-Updates).

### Current Somaris foundation

- The service already accepts open, close, stop, percentage-position, and motor-speed commands through its durable command queue. Endpoint calibration is a supervised operation.
- Commands use idempotency keys, retain execution outcomes, and distinguish GATT write acknowledgement from observed movement. A scheduler must use this path rather than writing to BLE itself.
- Service state is persisted in a JSON state file and restored at startup. There is currently no automation model, scheduler worker, or automation API/UI.
- The HTTP management API has no authentication. Schedules create future control actions, so deployment boundaries and access expectations must be reviewed before enabling persistent automations.
- A schedule cannot run while Somaris is stopped. The UI and API must show the last outcome and explain missed runs; the service must not replay an unbounded backlog after restart.

### Recommended first release

Support recurring weekly schedules defined by a local wall-clock time, an explicit IANA time zone, selected weekdays, one or more enrolled blinds, and one fixed target per schedule: open, close, or a closed-percent position. Allow schedules to be created, edited, paused/resumed, and deleted, and show the next run plus recent run outcomes.

Do not include arbitrary sequences, conditional rules, sunrise/sunset calculations, toggle actions, motor-speed changes, endpoint calibration, or scheduled Stop in the first release. Toggle depends on current position being fresh and unambiguous; calibration is supervised; Stop has meaning only while a movement is in progress. Keep those out of unattended automation.

## Implementation plan

### 1. Agree the behavior and safety boundaries

Resolve these product decisions before implementation. The defaults below are recommendations for the sprint, not behavior that exists today.

| Decision | Recommended first-release behavior |
| --- | --- |
| Time basis | Store an IANA time-zone identifier per schedule; evaluate the selected local wall-clock time in that zone. Do not inherit a changing host time zone silently. |
| Daylight-saving gaps | If a selected local time does not exist on the spring-forward date, skip that occurrence and record why. |
| Daylight-saving repeats | If a local time occurs twice on fall-back, execute once for that scheduled date, at the first occurrence. |
| Service downtime | Execute a due run only within a short, documented grace window (recommend 5 minutes); otherwise record it as missed. Never burst-replay older occurrences. |
| Offline or stale blind | Record the target as skipped/unavailable for that occurrence. Do not start repeated BLE reconnects outside the normal queue policy. |
| Command failure | Preserve the normal command status and idempotency rules. Never retry a command whose write outcome is uncertain. |
| Target lifecycle | If an enrollment is removed or loses a supported capability, retain the schedule but pause/flag it for repair instead of silently retargeting it. |
| Authorization | Keep operation within the existing trusted local/API boundary. If remote access is a requirement, resolve authentication and permissions before adding persistent schedule writes. |

### 2. Add a small persisted schedule model

Represent each schedule with a stable ID, name, enabled state, weekday set, local time, IANA time zone, explicit target enrollment IDs, target action, creation/update timestamps, and computed next-run instant. Do not store only a device label or an ephemeral BLE identifier.

Record each execution occurrence with a unique schedule ID plus nominal scheduled instant. Persist a claim before dispatch so restart or concurrent scheduler ticks cannot enqueue the same occurrence twice. Associate child command IDs and terminal outcomes per target. Bound run-history retention using a documented count or age, consistent with the existing command and activity journals.

### 3. Build a restart-safe scheduler around the existing command queue

Add one Tokio scheduler worker that loads enabled schedules, calculates their next occurrence, and submits due actions through the same validation, cooldown, idempotency, queue, BLE, and telemetry-reconciliation path as manual commands. The worker should not call the BLE adapter directly.

Use a bounded polling interval suitable for blind schedules (a 10–15 second tick is a reasonable starting point), calculate due instants with timezone-aware code, and persist occurrence claims and next-run updates atomically with service state. Do not hold the state lock while awaiting a command. Use deterministic occurrence keys so duplicate ticks and restart recovery cannot create duplicate movement.

### 4. Add an automation API and contract

Provide CRUD operations to list, create, inspect, update, enable/disable, and delete schedules. Add a read endpoint for recent run records. Validate weekday/time-zone/action/target inputs, enforce capability checks for percentage positioning, and return a clear conflict when a requested blind is calibrating or no longer enrolled. Reuse existing response envelopes, idempotency patterns, OpenAPI contract generation, and audit logging.

For batch targets, return an occurrence record with independent child results per blind. A successful schedule dispatch means commands were accepted; it must not be displayed as proof that blinds moved. Movement remains confirmed only by subsequent observations.

### 5. Add dashboard management and status

Add a schedules view that supports create/edit, selected weekdays, local time and time zone, target blinds/action, pause/resume, delete, next-run display, and recent outcomes. Make the displayed time zone explicit. Show scheduled, dispatched, completed/observed, skipped, missed, and failed outcomes distinctly; show uncertain command outcomes as requiring telemetry reconciliation.

On mobile/panel layouts, keep controls usable and avoid hiding an active schedule's next run or failure state. Do not imply that a paused, invalid, unavailable, or missed schedule will move a blind.

### 6. Add tests using TDD

- **Time calculation:** weekday selection, next occurrence, invalid zones, midnight boundaries, spring-forward gaps, fall-back duplicates, and grace-window expiry.
- **Scheduler state:** one occurrence despite repeated ticks, atomic claim before dispatch, restart after claim, restart before claim, edit/pause/delete while an occurrence is due, bounded history, and no catch-up burst.
- **Command integration:** dispatch uses the regular queue; command IDs and per-target outcomes are retained; cooldown, stale/ambiguous observations, missing capability, calibration lock, and uncertain-write outcomes are represented without unsafe retries.
- **API contract:** validation, CRUD, idempotent creation where applicable, pause/resume, run history, error cases, and OpenAPI schemas.
- **Dashboard:** form validation, explicit time zone, accessible controls, pause/resume, next-run and outcome rendering, and safe handling of failed refreshes.
- **End-to-end:** deterministic simulator runs with a controllable clock; verify occurrence scheduling and observed movement without physical BLE.

After simulator validation, perform a supervised Hallway Blinds check with a near-future schedule, confirm its run record and observed result, then remove or disable the test schedule. Do not use endpoint calibration for schedule verification.

### 7. Suggested delivery phases

1. **Design gate:** agree time-zone/DST semantics, missed-run policy, target lifecycle, authentication boundary, and history retention.
2. **Core and scheduler:** add persisted models, timezone-aware occurrence calculation, deduplicated claims, worker lifecycle, and TDD coverage using a fake clock.
3. **API and OpenAPI:** implement schedule management and run history, capability validation, audit entries, and client contract tests.
4. **Dashboard:** add schedule forms, explicit status and time-zone presentation, accessibility, and responsive UI tests.
5. **Operational verification:** test simulator restart and downtime scenarios, inspect logs/history, then perform one supervised physical run and document limits.
6. **MCP follow-up:** start with read-only schedule and run-history tools. Add schedule mutation tools only after the API semantics and authorization boundary are settled.

## Acceptance criteria

- A user can create a weekly local-time schedule for one or more enrolled blinds and an open, close, or supported percentage target.
- Pausing, resuming, editing, and deleting schedules is reflected durably across a service restart.
- Every due occurrence is claimed and dispatched at most once, and older missed occurrences are not replayed in a burst.
- Runs use the normal Somaris command queue and preserve its idempotency, capability, safety, cooldown, and uncertain-outcome behavior.
- The UI and API expose the next run, time zone, per-target command outcome, missed/skipped reason, and recent history without equating write acknowledgement with movement.
- Time-zone and daylight-saving behavior follows the agreed semantics and is covered by deterministic tests.
- The feature remains within the approved access boundary, and no schedule runs while disabled or invalid.

## Explicitly out of scope for this change set

This file is a planning artifact only. No schedule storage, scheduler task, schedule API, dashboard feature, or scheduled blind command is added here. Those changes belong to a future sprint/phase.
