# Somaris

[![Publish Docker image](https://github.com/CyanAutomation/somaris/actions/workflows/publish-image.yml/badge.svg?branch=main)](https://github.com/CyanAutomation/somaris/actions/workflows/publish-image.yml)

Somaris is a Rust service and API for Wazombi Labs smart blind motors. The initial implementation focuses on telemetry, simulator-backed control, command tracking, and API reliability. The built-in dashboard provides a browser interface for enrollment, blind control, telemetry, and service status.

## Run

```sh
cargo run
```

The service listens on `127.0.0.1:8080` by default. Set `SOMARIS_LISTEN` to choose another address. Two built-in simulators are enabled by default: `Somaris Simulator 1` and `Somaris Simulator 2`, each starting closed at 100% and with independent position, scenario, wireless, and battery settings. Their names are synthetic and chosen to avoid likely collisions with physical blinds. Both use the hallway profile by default, with the observed 80 ms connection and 72 ms write-response delays, movement of about one percentage point per second, capture-informed RSSI variation, and a synthetic 4210 mV battery sample when a later service-resolution read fails. Physical discovery and control use the host Bluetooth adapter through `btleplug`; Linux deployments need a working BlueZ adapter and permission to access it. If Bluetooth is unavailable, the HTTP API stays up in a degraded state and retries adapter startup.

Persistent service state is stored in `data/state.json` by default. Set `SOMARIS_DATA_DIR` to change its directory. The latest 30 recent-activity entries are stored with this state automatically; no separate path is needed. They cover enrollment changes, command and batch outcomes, and simulator changes. Telemetry samples are kept in a separate append-only journal at `data/state.history.jsonl` by default, so they do not enlarge every state snapshot. Set `SOMARIS_HISTORY_PATH` to choose another journal path; setting it also enables the history API. Existing snapshots with embedded telemetry are migrated to the journal on startup. On Unix, these files are restricted to the service account. BLE discovery retains at most 1,024 recent physical devices. At capacity, it evicts the oldest safe entry or removes an un-enrolled ambiguous identity group together, while keeping enrolled candidates. A different peripheral ID cannot create a second fresh candidate for an already observed enrolled identity; it can refresh the identity once the retained observation is stale. Physical devices must be freshly observed and explicitly enrolled. The controller supports open, close, stop, toggle, direct percentage positioning, motor-speed settings, and supervised endpoint calibration. Percentage targets use the V2 motor target characteristic and are expressed as percent closed (0 is open, 100 is closed). Motor speed uses a 0% to 100% setting and requires SOMA firmware 2.2.0 or later; the current setting is not read back by Somaris. Endpoint calibration is a supervised start, nudge, mark, stop workflow that blocks ordinary movement and survives service restarts. Arbitrary GATT writes are not exposed. Battery voltage is cached from background BA71 reads for enrolled blinds. Successful reads are refreshed hourly; unavailable reads are retried after five minutes. Battery percentages are estimates derived from voltage. Control endpoints do not provide authentication, so keep the default loopback binding or place the service behind a trusted access boundary before exposing it to other machines.

Set `SOMARIS_ALLOWED_ORIGINS` to a comma-separated list of exact HTTP or HTTPS origins for browser access. The default is an empty list. Queue defaults are 1,000 outstanding commands, 32 per device, one adapter operation at a time, and a 120-second queue wait. BLE scanning yields the adapter between bounded 1-second scan windows so scans do not overlap GATT reads or writes; set `SOMARIS_ADAPTER_SCAN_WINDOW_MS` to tune the window from 100 to 10,000 milliseconds. Shorter windows reduce command wait time while increasing scan restarts. After a successful acknowledged motor write, the BLE adapter may reuse that same connection for up to five seconds; it checks connection state and re-verifies the SOMA identity before each subsequent operation, then disconnects on expiry. The queue settings are `SOMARIS_COMMAND_MAX_OUTSTANDING`, `SOMARIS_COMMAND_MAX_QUEUED_PER_DEVICE`, `SOMARIS_ADAPTER_CONCURRENCY`, `SOMARIS_ADAPTER_SCAN_WINDOW_MS`, and `SOMARIS_COMMAND_QUEUE_WAIT_TIMEOUT_SECONDS`.

## Dashboard

Open `http://127.0.0.1:8080/` after starting Somaris (or use your configured address). Add a freshly discovered blind to control it; both built-in simulators are available by default, and up to two simulators can be enrolled at a time. The dashboard explains the limit and directs users to remove an existing simulator before adding another. The dashboard is included in the binary and container and uses the same-origin API. Individual controls include Open, Stop, and Close; the dashboard can also open or close all currently ready enrolled blinds. All-blinds batches are queued sequentially, and the dashboard reports unavailable blinds that were skipped. Panel view keeps per-blind controls to Open, Stop, and Close; percentage positioning remains available in the other views. Position is last-reported telemetry, and a command acknowledgement does not confirm movement. Choose Auto, Panel, Mobile, or Desktop from the display menu; the selection is saved in this browser. Auto selects Panel for small, near-square viewports. Add `?view=panel` to the URL to pin Panel mode on a kiosk. Use Node.js 26 for UI development and tests; `.nvmrc` selects that major version in local tools and CI. Install the UI tooling and browser with `npm ci` and `npx playwright install chromium`, then run `npm run check:ui` to type-check, compile the browser modules, and run the unit tests. The compiled `.mjs` files are checked in because the Rust service embeds them at compile time.

The panel viewport and accessibility behavior covered by dashboard regression tests is specified in [the test contracts](docs/test-contracts.md).

The dashboard and control API have no authentication. Keep the default loopback binding or put the service behind a trusted access boundary before exposing it.

## API

The HTML API reference is available at `/docs`; the machine-readable contract is served at `/openapi.json` and `/openapi.yaml`. `GET /api/control/v1/audit-log` returns up to 30 recent enrollment, command, batch, and simulator changes without extra storage configuration. `GET /api/dev/simulators` lists both simulators. Enrolling a third simulator while two are already enrolled returns HTTP 409 with error code `simulator_limit_reached`; remove one enrollment before retrying. Use `/api/dev/simulators/{id}/scenario` and `/api/dev/simulators/{id}/reset` to configure or reset one simulator; the older `/api/dev/simulator` endpoints continue to target Simulator 1. Control requests require an `Idempotency-Key`; command acceptance is asynchronous, so clients poll the returned command resource. A write acknowledgement does not confirm that a physical blind moved.

Physical managed-device entries include `gatt_readiness`, a summary of the most recent retained command outcome and last acknowledged GATT write. Command records are retained for up to 24 hours, subject to the 1,000-record journal cap. This is historical evidence with timestamps, not a live connection probe: `availability: "ready"` still means only that a unique recent advertisement was observed. `/api/v1/ready` reports adapter availability. Command resources retain each execution attempt's phase, duration, write state, and stable failure code. Somaris allows one automatic retry after a confirmed connection or service-discovery failure before a write. Between attempts it yields the adapter to scanning for up to one bounded scan window (about 1.3 seconds at the default), then applies a 250 ms backoff; the next attempt rechecks advertisement freshness and identity. A 45-second execution deadline bounds physical attempts, scan refresh, and backoff; queue wait has its own timeout. Durable state logs include the operation name, serialization time, durable-write time, and total time; writes taking at least 500 ms are logged as warnings. After Stop or deadline cancellation, Somaris gives adapter cleanup up to 10 additional seconds before finalizing the command. If the GATT write may have started, timeout is `outcome_unknown` and must be reconciled against telemetry before another command. Stop cancels queued movements and can preempt an in-flight movement only until its GATT write starts; after that it remains serialized behind the movement.

Live API responses are not stored by shared caches. Conditional reads use private revalidation with ETags, and the built-in HTTP and MCP clients reuse an ETag response when the data has not changed. The static API reference is cacheable for one hour and returns an ETag. The recent-activity endpoint returns up to 30 entries newest first. When the optional telemetry history is enabled, new samples are appended and synced in batches; the retained history is compacted periodically.

### MCP

Somaris also provides a local stdio MCP server. Keep the HTTP service running, then configure an MCP client to launch `somaris mcp`. The MCP process forwards tool calls to the HTTP service, so it shares its device observations, queue, and persistent journal without starting a second Bluetooth scanner. `SOMARIS_API_URL` selects the API base URL and defaults to `http://127.0.0.1:8080`; it can point to another Somaris service on a trusted LAN.

The MCP tools expose health, readiness, device discovery, recent activity, optional telemetry history, enrollment, open/close/stop/percentage-position/toggle commands, batch commands, and command status. The `somaris://api/openapi.json` resource provides the live machine-readable HTTP contract. Mutating control tools require a caller-provided idempotency key. Commands return asynchronously; use the returned command or batch ID with the corresponding status tool. A write reported as possibly sent must be checked against telemetry before retrying. Simulator configuration is intentionally not exposed through MCP.

The control API and MCP tools do not authenticate callers. This deployment is intended for a trusted LAN; keep the service behind the network boundary you trust. CORS does not authenticate MCP or HTTP clients.

## Container image

The GitHub Actions workflow tests pull requests targeting `main` and audits `Cargo.lock`. When image inputs change, it builds the Docker image for `linux/amd64` on the pull request without publishing. Pushes to `main` continue to run tests and the dependency audit, and publish both architectures only when image inputs change. Published images use `cyanautomation/somaris:latest` and a commit tag with SBOM and provenance attestations. Docker selects the matching image automatically, so a Raspberry Pi running 64-bit Raspberry Pi OS can use the regular `docker pull cyanautomation/somaris` command. 32-bit Raspberry Pi OS (`linux/arm/v7`) is not currently supported.

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

The API is available at `http://127.0.0.1:8080` by default, and a named volume retains its state and recent-activity log across container restarts. Edit `.env` to change the host address or port, simulator, logging, browser origins, queue settings, or optional telemetry-history journal; then run `docker compose up -d` to apply the changes.

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

Check `docker compose logs somaris`, `/api/v1/health`, and `/api/v1/ready` when diagnosing startup. A healthy scanner shows adapter availability, not proof that a blind can connect or accept commands. Use the managed-device `gatt_readiness` summary and command attempt history to distinguish fresh advertisements from recent GATT outcomes. Verify discovery, enrollment, GATT reads, and a supervised movement and stop with a physical blind. A write acknowledgement confirms the GATT response only; compare position telemetry to verify movement. If a write's outcome is uncertain, check telemetry before retrying. If BlueZ or D-Bus restarts, check readiness and logs for recovery; recreate the container with `docker compose up -d --force-recreate` if it does not reconnect. Restarting the host system bus can affect other services.

For simulator-only use on a host without `/run/dbus`, create an empty directory accessible to the Docker daemon, set `SOMARIS_DBUS_DIR` to its absolute path in `.env`, and start Compose normally. Somaris will serve the simulator but report Bluetooth as degraded and retry adapter startup. The directory must be available to the daemon, including through Docker Desktop file sharing when applicable. The standalone `docker run` example above has no D-Bus mount and is likewise simulator-oriented.
