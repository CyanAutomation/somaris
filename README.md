# Somaris

[![Publish Docker image](https://github.com/CyanAutomation/somaris/actions/workflows/publish-image.yml/badge.svg?branch=main)](https://github.com/CyanAutomation/somaris/actions/workflows/publish-image.yml)

Somaris is a Rust service and API for Wazombi Labs smart blind motors. The initial implementation focuses on telemetry, simulator-backed control, command tracking, and API reliability. The built-in dashboard provides a browser interface for enrollment, blind control, telemetry, and service status.

## Run

```sh
cargo run
```

The service listens on `127.0.0.1:8080` by default. Set `SOMARIS_LISTEN` to choose another address. The built-in simulator is enabled by default and starts as `Hallway Blinds0`, closed at 100%. Its default hallway profile uses the observed 80 ms connection and 72 ms write-response delays, moves at about one percentage point per second, varies RSSI over captured values, and retains a synthetic 4210 mV battery sample when a later service-resolution read fails. Physical discovery and control use the host Bluetooth adapter through `btleplug`; Linux deployments need a working BlueZ adapter and permission to access it. If Bluetooth is unavailable, the HTTP API stays up in a degraded state and retries adapter startup.

Persistent service state is stored in `data/state.json` by default. Set `SOMARIS_DATA_DIR` to change its directory. Set `SOMARIS_HISTORY_PATH` to enable a separate history journal. On Unix, these files are restricted to the service account. BLE discovery retains at most 1,024 recent physical devices. At capacity, it evicts the oldest safe entry or removes an un-enrolled ambiguous identity group together, while keeping enrolled candidates. A different peripheral ID cannot create a second fresh candidate for an already observed enrolled identity; it can refresh the identity once the retained observation is stale. Physical devices must be freshly observed and explicitly enrolled. The controller supports only open, close, stop, and toggle; percentage positioning and arbitrary GATT writes are not exposed. Battery voltage is cached from background BA71 reads for enrolled blinds. Control endpoints do not provide authentication, so keep the default loopback binding or place the service behind a trusted access boundary before exposing it to other machines.

Set `SOMARIS_ALLOWED_ORIGINS` to a comma-separated list of exact HTTP or HTTPS origins for browser access. The default is an empty list. Queue defaults are 1,000 outstanding commands, 32 per device, one adapter operation at a time, and a 120-second queue wait. BLE scanning yields the adapter between bounded 5-second scan windows so scans do not overlap GATT reads or writes; set `SOMARIS_ADAPTER_SCAN_WINDOW_MS` to tune the window from 100 to 10,000 milliseconds. The queue settings are `SOMARIS_COMMAND_MAX_OUTSTANDING`, `SOMARIS_COMMAND_MAX_QUEUED_PER_DEVICE`, `SOMARIS_ADAPTER_CONCURRENCY`, `SOMARIS_ADAPTER_SCAN_WINDOW_MS`, and `SOMARIS_COMMAND_QUEUE_WAIT_TIMEOUT_SECONDS`.

## Dashboard

Open `http://127.0.0.1:8080/` after starting Somaris (or use your configured address). Add a freshly discovered blind to control it; the simulator is available by default. The dashboard is included in the binary and container and uses the same-origin API. Position is last-reported telemetry, and a command acknowledgement does not confirm movement. Run the UI unit tests with `node --test ui/tests/*.test.mjs`.

The dashboard and control API have no authentication. Keep the default loopback binding or put the service behind a trusted access boundary before exposing it.

## API

The HTML API reference is available at `/docs`; the machine-readable contract is served at `/openapi.json` and `/openapi.yaml`. Control requests require an `Idempotency-Key`; command acceptance is asynchronous, so clients poll the returned command resource. A write acknowledgement does not confirm that a physical blind moved.

Live API responses are not stored by shared caches. Conditional reads use private revalidation with ETags, and the built-in HTTP and MCP clients reuse an ETag response when the data has not changed. The static API reference is cacheable for one hour and returns an ETag. When history is enabled, new samples are appended and synced in batches; the retained history is compacted periodically.

### MCP

Somaris also provides a local stdio MCP server. Keep the HTTP service running, then configure an MCP client to launch `somaris mcp`. The MCP process forwards tool calls to the HTTP service, so it shares its device observations, queue, and persistent journal without starting a second Bluetooth scanner. `SOMARIS_API_URL` selects the API base URL and defaults to `http://127.0.0.1:8080`; it can point to another Somaris service on a trusted LAN.

The MCP tools expose health, readiness, device discovery, telemetry history, enrollment, open/close/stop/toggle commands, batch commands, and command status. The `somaris://api/openapi.json` resource provides the live machine-readable HTTP contract. Mutating control tools require a caller-provided idempotency key. Commands return asynchronously; use the returned command or batch ID with the corresponding status tool. A write reported as possibly sent must be checked against telemetry before retrying. Simulator configuration is intentionally not exposed through MCP.

The control API and MCP tools do not authenticate callers. This deployment is intended for a trusted LAN; keep the service behind the network boundary you trust. CORS does not authenticate MCP or HTTP clients.

## Container image

The GitHub Actions workflow tests pull requests targeting `main`, audits `Cargo.lock`, and builds the Docker image for `linux/amd64` and `linux/arm64` without publishing. When changes reach `main`, it publishes both architectures under `cyanautomation/somaris:latest` and a commit tag with SBOM and provenance attestations. Docker selects the matching image automatically, so a Raspberry Pi running 64-bit Raspberry Pi OS can use the regular `docker pull cyanautomation/somaris` command. 32-bit Raspberry Pi OS (`linux/arm/v7`) is not currently supported.

```sh
docker pull cyanautomation/somaris
```

```sh
docker run --detach \
  --name somaris \
  --publish 127.0.0.1:8080:8080 \
  --volume somaris-data:/data \
  cyanautomation/somaris:latest
```

The container listens on port `8080` and stores persistent service state in `/data`. Docker checks `/api/v1/health` inside the container and reports the container as healthy when the HTTP service responds. This is a liveness check: Bluetooth can be degraded while the container is healthy. Use `/api/v1/ready` to monitor Bluetooth adapter availability. If you change the container's `SOMARIS_LISTEN` address or port, set `SOMARIS_HEALTH_URL` to a URL reachable inside the container (for example, `http://127.0.0.1:8081/api/v1/health`). Docker's health status alone does not restart an unhealthy container.

For a local Compose setup, copy the environment template and start the service:

```sh
cp .env.example .env
docker compose up -d
```

The API is available at `http://127.0.0.1:8080` by default, and a named volume retains its state across container restarts. Edit `.env` to change the host address or port, simulator, logging, browser origins, queue settings, or optional history journal; then run `docker compose up -d` to apply the changes.

### Linux Bluetooth with Compose

Compose mounts the host's `/run/dbus` directory read-only into the container, allowing Somaris to use the host system bus and BlueZ. The directory must already exist on the Docker daemon host; Compose will not create it. Set `SOMARIS_DBUS_DIR` in `.env` if that directory is elsewhere. The host must have a powered Bluetooth adapter, a running system D-Bus and BlueZ, and policies allowing the container's host-visible UID to use BlueZ. Docker Desktop runs its daemon in a Linux VM, so a desktop computer's native Bluetooth adapter is not made available by this mount.

The image runs as UID `10001` and owns `/data` with that UID. On Agentlab, D-Bus rejected this UID until a matching host account existed. If the same issue occurs on a Linux host without user-namespace remapping, first check `getent passwd 10001` and `getent passwd somaris`. If both are unused, an administrator can create a locked, non-login account with no home directory (the host group name and nologin path can vary by distribution):

```sh
sudo useradd --system --uid 10001 --gid nogroup --no-create-home \
  --home-dir /nonexistent --shell /usr/sbin/nologin somaris
getent passwd 10001
```

Do not create a duplicate or reuse an unrelated account if the name or UID is already assigned. With rootless Docker or user-namespace remapping, determine the actual host-visible UID and its access rules instead. Agentlab's existing BlueZ policy allowed scanning without a custom rule; other hosts may require different D-Bus, BlueZ, or security policy. Diagnose the specific denial before changing permissions. Avoid running the container as root or granting privileged mode merely to access BlueZ.

The mount exposes the host system bus to the container, subject to host policy. Read-only protects the mounted filesystem from modification; it does **not** make D-Bus messages read-only. A process in the container can call any host bus services its identity is authorized to use. The simulator can remain enabled while physical devices are tested; simulated devices are identified separately in the API.

Check `docker compose logs somaris`, `/api/v1/health`, and `/api/v1/ready` when diagnosing startup. A healthy scanner shows adapter availability, not proof that a blind can connect or accept commands. Verify discovery, enrollment, GATT reads, and a supervised movement and stop with a physical blind. If a write's outcome is uncertain, check telemetry before retrying. If BlueZ or D-Bus restarts, check readiness and logs for recovery; recreate the container with `docker compose up -d --force-recreate` if it does not reconnect. Restarting the host system bus can affect other services.

For simulator-only use on a host without `/run/dbus`, create an empty directory accessible to the Docker daemon, set `SOMARIS_DBUS_DIR` to its absolute path in `.env`, and start Compose normally. Somaris will serve the simulator but report Bluetooth as degraded and retry adapter startup. The directory must be available to the daemon, including through Docker Desktop file sharing when applicable. The standalone `docker run` example above has no D-Bus mount and is likewise simulator-oriented.
