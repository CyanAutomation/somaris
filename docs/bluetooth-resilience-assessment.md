# Bluetooth resilience assessment

**Observed on 2026-11-05, using the Agentlab Somaris deployment through Termius.**

## What was exercised

The scanner was healthy and scanning. Somaris had three enrolled physical blinds with fresh, unique advertisements: Bedroom Left, Bedroom Right, and Hallway Blinds. All three initially reported fully closed (`current_closed_percent: 100`, `target_closed_percent: 100`). The two simulators were not included in the physical command batches.

Open, Stop, and Close were submitted as batches. Batches use the service's sequential, label-ordered command queue. Position readings below are advertisement telemetry, not inferred from write acknowledgements.

| Action | Bedroom Left | Bedroom Right | Hallway Blinds |
| --- | --- | --- | --- |
| Open | GATT write acknowledged on attempt 1; telemetry progressed from 100% closed through 33% to 0%. | Acknowledged on attempt 1; telemetry progressed through 68% and 43% to 39% closed at Stop. | Acknowledged on attempt 1; telemetry progressed through 58% to 0%. |
| Stop | Acknowledged on attempt 2. Attempt 1 timed out during connection after 30 seconds; attempt 2 acknowledged the write. Total command time was about 42 seconds, by which time the blind was fully open. | Acknowledged on attempt 1; about 6.3 seconds for the attempt. | Acknowledged on attempt 1; about 6.4 seconds for the attempt. |
| Close | Failed after two automatic pre-write connection attempts. A later, explicitly safe resubmission also failed after two attempts. No GATT write was sent; the blind remained fully open. | Acknowledged on attempt 2 after one pre-write connection failure; telemetry reached 100% closed. | Acknowledged on attempt 1; telemetry reached 100% closed. |

The final snapshot showed a healthy, scanning adapter and fresh advertisements for all three blinds. Bedroom Right and Hallway Blinds were fully closed and had `write_acknowledged` as their latest GATT outcome. Bedroom Left remained fully open with `gatt_readiness.status: connection_failed` and two attempts on the latest command. Its advertisement-based `availability` still read `ready`.

The safe Close resubmission for Bedroom Left was made only after checking that its advertisement was fresh and unique, its reported position was still at the open target, its latest command had failed before any write, and the command was marked retryable. A new idempotency key was used. Its two new attempts again failed before the write.

## What the run establishes

- A fresh advertisement is useful evidence that a blind is nearby, but it does not establish that a GATT connection can be made. `/api/v1/ready` describes adapter/scanner availability; managed-device `availability: ready` describes a fresh supported observation; `gatt_readiness` describes the last retained command outcome. Those signals need to stay distinct in both the API and the user interface.
- The existing safety boundary worked: connection failures were recorded as `device_connection_failed` with `write_state: not_sent`, and Somaris retried only before a write. The Close resubmission was safe. No command with a possibly sent write was replayed.
- One retry helped twice: Bedroom Left's Stop and Bedroom Right's Close succeeded on attempt 2. It did not restore Bedroom Left, which failed across four pre-write attempts over two command records.
- The fixed 250 ms backoff can be too short to let the local Bluetooth stack recover from an immediate abort. It also does not help a connection attempt that consumes the 30-second connect timeout. A timeout followed by a retry consumed about 42 seconds for Stop.
- A Stop batch can inherit head-of-line delay from a slow or failed connection because the batch processes one blind at a time while adapter concurrency is one. Stop writes were acknowledged, but the delayed Bedroom Left Stop arrived only after that motor reached its open endpoint.
- GATT acknowledgement and physical movement are separate outcomes. The position advertisements confirmed movement and endpoint arrival; write responses alone would not have done so. The three motors also progressed at different rates, so one assumed travel rate is not enough for stall detection.

## Host evidence

Somaris logged connection failures as `le-connection-abort-by-local` and `device_connection_failed`. BlueZ `bluetoothd` also logged `No matching connection for device` around the test. That BlueZ message appeared near successful commands as well, so it is not sufficient on its own to identify the cause. The evidence points to intermittent local BlueZ/adapter connection setup, but does not prove whether the blind, radio conditions, or host stack initiated each abort.

The host also logged a Battery Level notification warning. It was not tied to a Somaris movement command and should be investigated separately from the BA71 cache and command path.

## Controller design to take forward

1. **Expose readiness as separate evidence.** Keep adapter readiness, fresh/unique advertisement, recent GATT acknowledgement, recent pre-write failure, uncertain write outcome, and movement feedback as separate states. Include observation and attempt timestamps. Do not label a fresh advertisement as proof that the blind is ready to accept a command.
2. **Keep retry decisions tied to write certainty.** Automatically retry only failures proven to precede the GATT write. Never retry a write timeout or other `possibly_sent` result as an open/close movement; reconcile it against telemetry first. Preserve idempotency keys and the durable command history.
3. **Instrument before tuning timeouts.** Record per-attempt durations for peripheral lookup, connect, service discovery, identity verification, GATT write, and disconnect, along with sanitized failure class and the advertisement age/RSSI used for the attempt. Current total attempt duration cannot tell which phase needs more time.
4. **Adapt retry delay and budget by failure class and device history.** Use capped exponential backoff with jitter for repeated, confirmed pre-write failures, resetting the failure streak after an acknowledgement. Add a per-device cooldown/circuit breaker after repeated failures so callers cannot immediately create an unbounded series of safe-but-costly reconnect attempts. Keep the budget inside the shared execution deadline; tune it from phase-level latency percentiles rather than increasing retries blindly.
5. **Give Stop a bounded priority path.** Stop should cancel queued movement and jump ahead of work that has not begun a GATT write. Keep one adapter operation at a time and never interrupt a write that may already have reached a motor. Use a shorter first connection window for Stop, then a bounded recovery attempt if the overall Stop budget allows it.
6. **Treat movement as a separate feedback loop.** After an ACK, watch fresh position advertisements for progress toward the requested endpoint. Learn travel-rate and no-progress thresholds per blind, report `motion_observed` or `stalled` separately from GATT status, and do not issue a corrective movement automatically after an uncertain write.

The first implementation step is phase-level timing and a readiness-state split. Those measurements can then guide per-stage timeout tuning without making a healthy but slower blind look unavailable or replaying an uncertain motor command.

## Implemented follow-through

The controller records peripheral lookup, connect, service discovery, identity verification, GATT discovery, GATT write, and disconnect durations on each physical command attempt, plus firmware-check time for motor-speed writes. It also records the advertisement timestamp, age, and RSSI used for that attempt. Scanner diagnostics now distinguish adapter-stage errors from successful scanner recovery with a recovery count and last-recovered timestamp. The managed-device API and dashboard show advertisement freshness separately from recent GATT evidence; neither is presented as a live connection test.

Automatic retries remain limited to confirmed connection or service-resolution failures before a write. Their delay now uses capped exponential backoff with deterministic per-command jitter and grows with the blind's recent pre-write failure streak. Four failed attempts within ten minutes open a 15-second cooldown that grows to at most 120 seconds. New Open/Close requests receive `429` and `Retry-After`; commands accepted before the cooldown opened are deferred at execution. Stop bypasses the cooldown.

Stop batches cancel queued movement and preempt an in-flight movement before its GATT write on each accepted target, reserve adapter access, make one attempt for every target, then retry eligible failures. Current connection windows are 4 seconds for a Stop's first attempt and 6 seconds for its recovery attempt; Open and Close use 8 seconds on the first attempt and 6 seconds on retry. This shorter Stop window is a bounded latency adjustment based on the Agentlab attempt timings; the single-blind sample is too small to establish optimal timeout percentiles, so continue measuring on the target adapter before further tuning.

Motor telemetry now reports progress, learned travel rate, target reached, or stalled independently from GATT status. A stalled report is observational only; Somaris does not send a corrective movement after a stall or uncertain write.

## Motor settings GATT mapping

The speed setter was already mapped to the Shade Config service `0x1890` and characteristic `0x1896`, with typed payload `[0x01, 0x01, percent]`. It now checks the standard Device Information firmware revision characteristic `0x2A26` before sending that payload and refuses to write below firmware 2.2.0 or when the revision cannot be read. The added `GET /api/control/v1/devices/{id}/motor-settings` query subscribes to Shade Config notifications and sends `[0xFF, 0x01, 0x01]`; a response tuple with setting ID `0x01`, length `0x01`, and a value from 0 to 100 becomes the reported speed. On older firmware it reports the version and a null speed. Stop can cancel the read before it completes.

The Shade Config query and tuple format follow third-party reverse engineering in the [archived Homebridge SOMA client](https://github.com/ebaauw/homebridge-soma/blob/main/lib/SomaClient.js) and [SOMA V2 protocol notes](https://github.com/rzuppur/smart-shades-documentation/blob/master/V2.md); they are not manufacturer specifications. Those sources also surface other settings such as local time offset, boot sequence, sunrise, and sunset. Somaris does not expose those settings or arbitrary Shade Config writes until their semantics and device support are verified. The firmware gate and query path are covered by protocol and fake-adapter tests; the new settings read has not yet been exercised against HallwayBlind on Agentlab.
