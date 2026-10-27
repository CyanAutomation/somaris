# Somaris

Somaris is a Rust service and API for Wazombi Labs smart blind motors. The initial implementation focuses on telemetry, simulator-backed control, command tracking, and API reliability. A dashboard can be added after the service contract is established.

## Run

```sh
cargo run
```

The service listens on `127.0.0.1:8080` by default. Set `SOMARIS_LISTEN` to choose another address. The built-in simulator is enabled by default. Physical discovery and control use the host Bluetooth adapter through `btleplug`; Linux deployments need a working BlueZ adapter and permission to access it. If Bluetooth is unavailable, the HTTP API stays up in a degraded state and retries adapter startup.

Persistent service state is stored in `data/state.json` by default. Set `SOMARIS_DATA_DIR` to change its directory. Set `SOMARIS_HISTORY_PATH` to enable a separate history journal. Physical devices must be freshly observed and explicitly enrolled. The controller supports only open, close, stop, and toggle; percentage positioning and arbitrary GATT writes are not exposed. Battery voltage is cached from background BA71 reads for enrolled blinds. Control endpoints do not provide authentication, so keep the default loopback binding or place the service behind a trusted access boundary before exposing it to other machines.

Set `SOMARIS_ALLOWED_ORIGINS` to a comma-separated list of exact HTTP or HTTPS origins for browser access. The default is an empty list. Queue defaults are 1,000 outstanding commands, 32 per device, one adapter operation at a time, and a 120-second queue wait. Override them with `SOMARIS_COMMAND_MAX_OUTSTANDING`, `SOMARIS_COMMAND_MAX_QUEUED_PER_DEVICE`, `SOMARIS_ADAPTER_CONCURRENCY`, and `SOMARIS_COMMAND_QUEUE_WAIT_TIMEOUT_SECONDS`.

## API

The machine-readable contract is served at `/openapi.json` and `/openapi.yaml`. Control requests require an `Idempotency-Key`; command acceptance is asynchronous, so clients poll the returned command resource. A write acknowledgement does not confirm that a physical blind moved.
