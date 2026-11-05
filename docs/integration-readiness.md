# Integration readiness contract

Somaris remains the authority for enrollment identity, BLE communication,
observed motor state, and command execution. A HomeKit or Homebridge adapter
should consume this contract and submit commands through Somaris; it must not
connect to motors or infer execution from a successful HTTP response.

## Managed devices and identity

The `enrollment.id` UUID is the stable logical ID. It is persisted with the
enrollment and is independent of transient BLE peripheral identifiers and
labels. BLE rediscovery rebinds observations to the enrollment using the
existing opaque logical identity and advertised name. Simulated enrollments
are explicitly marked and routed to independent in-memory simulators, so a
simulator cannot claim a physical enrollment. Removing an enrollment removes
that UUID; enrolling the same motor later creates a new identity. Renaming the
label does not change identity. Back up and restore Somaris's complete data
file to preserve enrollment mappings.

## Capabilities and positions

Managed device responses include explicit `capabilities`. Current physical
Wazombi support is open, close, stop, and position feedback. Arbitrary
percentage positioning is unsupported even though controller encoding helpers
exist in the source. Simulators expose the same command contract.

Positions use percent closed: 0 is fully open and 100 is fully closed. The
existing `position.current_closed_percent` and
`position.target_closed_percent` are motor-reported observation values.
`requested_position_closed_percent` is the most recent retained open/close
command intent and is reported separately. `position_observed_at`, source,
confidence, status, and freshness clarify whether telemetry is present and
current. Stale observations remain explicitly stale; missing observations are
omitted rather than represented as zero.

## Command lifecycle and reconciliation

Commands are durably accepted before the API returns 202, then queued and
executed serially per device under the shared adapter concurrency limit. The
command resource reports queued/connecting, connection failure, write
acknowledgement, write failure, uncertain outcome, expiry, or cancellation.
A write acknowledgement confirms the GATT write response only; it does not
confirm movement or destination. `outcome_unknown` means a write may have
reached the motor and must not be blindly retried. A fresh device observation
is the source of motor-reported current and target state. Clients reconcile by
reading the command resource and subsequent managed-device observation; an
uncertain command remains uncertain until checked against telemetry. Existing
idempotency, queue cancellation, stop, restart recovery, and safe pre-write
retry rules remain authoritative.

## Internal state notifications

Rust consumers may call `AppState::subscribe_state_changes`. The bounded Tokio
broadcast channel sends monotonically increasing process-local revision
notifications after persisted transitions and meaningful position or
availability changes. Unchanged advertisements and unchanged simulator
position samples do not generate notifications. Notifications contain no
device payload and are not a durable event log. Receivers must read current managed-device and
command state after subscribing, after restart, and after a broadcast lag
error. `AppState::managed_devices_snapshot` provides an immutable typed
snapshot for native consumers and backs the HTTP list route. Publication is
nonblocking; disconnected and slow consumers do not hold up BLE or commands.
There is no external event endpoint.

## Persistence, security, and deployment

The existing single JSON state file persists enrollment UUIDs, command journal,
idempotency records, simulator configuration, and activity records using
atomic replacement and private file permissions. New managed response fields
are derived and do not alter the persisted schema, so older state files remain
readable without migration. The service retains existing handling for corrupt
state and does not regenerate IDs as an upgrade mechanism.

The HTTP management API remains unauthenticated. The binary defaults to
`127.0.0.1:8080`; Docker listens inside its container while Compose publishes
to `127.0.0.1` by default. CORS restricts browser origins but is not
authentication. Treat direct network access as trusted. No credentials or
authorization layer exists today; future scoped access belongs at the HTTP/MCP
boundary with read, command-write, and event-read permissions, leaving domain
and BLE operations unchanged.

The current Docker setup retains non-root execution, ARM64 build support,
BlueZ D-Bus access, persistent `/data`, and its existing health check. A future
HomeKit/Homebridge deployment must plan routing from its adapter to the
loopback-oriented management API and any required host/container networking
separately. This readiness work does not publish new ports or enable host
networking, mDNS, HAP, Matter, or protocol-specific behavior.

## Adapter boundary and limitations

An eventual adapter can list enrolled devices, read capabilities and
freshness-aware observations, submit open/close/stop using idempotency keys,
read command outcomes, and resynchronize after missed internal notifications.
Somaris retains identity, command scheduling, retries, telemetry, and BLE
control. Current control supports endpoints and stop only; telemetry is
advertisement-based and may be stale or unavailable. Notifications currently
signal a state revision rather than carrying entity-specific event details.
