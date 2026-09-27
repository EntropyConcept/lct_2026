# Контур — SAT-based engineer dispatch

Runnable Rust implementation of `task.pdf`: assignment, ordered routes, a web map,
explanations, baseline comparison, **cancellation and urgent-job replanning**,
configurable Excel service norms, real street routing for car/walk/bicycle, and
departure-aware public transport through a configured MOTIS/GTFS router.
Uses the real **CaDiCaL CDCL SAT solver**, compiled and linked into the executable.
No external solver process, Python backend, database, Node build, or API key.
City preparation uses external OSM services; SAT itself remains local/offline.
Public transport requires a service date and a MOTIS instance with suitable GTFS
and OSM coverage. It is never substituted with a speed estimate or walking-only route.

## Russian paper and road benchmark

- [Compiled report (PDF)](reports/dispatch-report.pdf) · [Typst source](reports/dispatch-report.typ)
- [All 45 measurements (CSV)](reports/road-benchmark/results.csv) · [reproduction and artifacts](reports/road-benchmark/README.md)

The report gives the full fast/exact mathematics and comparisons with greedy,
OR-Tools Routing GLS and CP-SAT at 1/5/30/120 seconds on all three synthetic CSVs.
All methods share frozen **real OSRM car/bike/foot road costs**, not straight-line
estimates; coordinates and workforce remain explicitly synthetic. Python and
OR-Tools are benchmark-only dependencies; the application remains Rust-only.

## Run

Install current stable Rust and a C++ compiler (CaDiCaL is C++ internally):

- macOS: Xcode Command Line Tools (`xcode-select --install`).
- Debian/Ubuntu: `sudo apt install build-essential`.

From this directory:

```sh
cargo build --release --locked
./target/release/dispatch-sat serve
# Open http://127.0.0.1:8080
```

Use **release**, not debug, for timing. Rust provides safe input handling and a
small native application; CaDiCaL does the intensive search in optimized C++.
The first build downloads Cargo dependencies and compiles the bundled solver.
Subsequent runs require no solver installation.

## Docker and Compose

Docker Engine/Desktop with Compose v2 or newer is sufficient; Rust, C++, Python
and Node are not required on the deployment host. The dispatcher image includes
the UI, service norms and three synthetic example datasets. Both runtime images
run as UID 10001. Large map inputs, generated indexes, local credentials and
reports are excluded from the build context.

Build and start the dispatcher demo from source:

```sh
cp .env.example .env
docker compose build app
docker compose up -d --no-build --pull never app
# Open http://localhost:8080
docker compose logs -f app
```

The offline demo works immediately. Real route preparation requires the local
MOTIS service below or an explicitly configured external router. The `routing`
profile adds MOTIS; it does not download or invent map/timetable data.

To deploy images published by CI instead of compiling locally:

```sh
# Run from a checkout, or a directory containing compose.yaml and .env.
docker compose pull app
docker compose up -d --no-build app
```

Images are `ghcr.io/entropyconcept/lct_2026/dispatch-sat` and
`ghcr.io/entropyconcept/lct_2026/motis`, published for Linux AMD64 and ARM64.
`IMAGE_PREFIX` changes the registry/repository prefix (use lowercase);
`IMAGE_TAG=main` follows main, `sha-<full-commit-sha>` pins a build, and
`pr-<number>` selects a same-repository PR preview. Images become available after
the corresponding workflow succeeds. If packages are private, authenticate with
`docker login ghcr.io` using a token with package read access; package visibility
is managed in GitHub Packages settings.

### Local street and public-transport router

The MOTIS image builds the same pinned 2.11.3 source and strict OSR patch as the
local macOS launcher. It serves the routing API; the dispatcher's embedded UI is
the web frontend. Its first source build downloads the upstream cross toolchain
and C++ dependencies and can take a long time. `MOTIS_BUILD_JOBS` defaults to 2 to
limit memory pressure. Using the published image avoids compiling the router.
On ARM hosts, source builds need AMD64 emulation for the upstream compiler stage;
the published ARM64 runtime image runs natively.

1. Put a suitable OSM PBF and licensed GTFS ZIP in `.transit/docker-inputs/`, named
   `moscow.osm.pbf` and `moscow.gtfs.zip`. See the Moscow data sources and coverage
   caveats below. Set `MOTIS_INPUT_DIR` to use another directory.
2. Copy `docker/motis-config.example.yml` to that directory as `config.yml`.
   Set `timetable.first_day` and `num_days` to the service dates covered by your
   feed. Keep `extend_calendar: false`. For street-only routing, remove the
   `timetable` section and set `osr_footpath: false`; no GTFS is then needed.
3. Build (or pull), import, then start:

```sh
docker compose --profile routing build app motis
# For prebuilt images, replace the build command with:
# docker compose --profile routing pull app motis

docker compose --profile tools run --rm --no-deps motis-import
docker compose --profile routing up -d --no-build --pull never --wait --wait-timeout 300
```

Import reads `/inputs/config.yml` and writes indexes plus the effective server
configuration into the `motis-data` named volume. The server needs only that
volume; source inputs are mounted read-only in the import container. MOTIS listens
on the Compose network at `http://motis:8081`; its API port is not published on the
host. The app can start independently; wait for MOTIS's health check before
preparing roads or public transport.

To change map data or dates, stop MOTIS, update the inputs/config, repeat import,
and start it again:

```sh
docker compose --profile routing stop motis
docker compose --profile tools run --rm --no-deps motis-import
docker compose --profile routing up -d --no-build --pull never --wait --wait-timeout 300
```

Do not run import concurrently with a server using the same data volume. Exported
city scenarios depend on snapshots in the separate `routing-cache` volume; back
up both volumes as appropriate. `docker compose --profile routing down` preserves
them; adding `--volumes` deletes them. Existing native `.routing-cache/` and
`.transit/moscow/data/` are not automatically migrated.

### Ports, proxy deployment and configuration

Compose publishes the app on host loopback by default. To use another port, set
both `DISPATCH_PORT=9090` and `DISPATCH_PUBLIC_ORIGIN=http://localhost:9090` in
`.env`. `DISPATCH_PUBLIC_ORIGIN` is the browser-facing HTTP(S) origin, with no
path or query. A TLS reverse proxy should preserve the original `Host` header;
set the origin to e.g. `https://dispatch.example.com`. The application has no
authentication: put access control at the proxy before exposing it publicly.
`DISPATCH_LISTEN_ADDRESS` controls the host-side published interface.

`DISPATCH_BIND` controls the binary's listening interface: native runs default to
`127.0.0.1`, images use `0.0.0.0`. Configured origins and local health checks are
allowed; unrelated Host/Origin headers are rejected. Forwarded headers do not
implicitly authorize new origins.

For an external MOTIS instance set `DISPATCH_MOTIS_URL` and omit the `routing`
profile. For Valhalla set `DISPATCH_ROAD_BACKEND=valhalla` and
`DISPATCH_VALHALLA_URL`. A router on the Docker host is not container localhost;
use a reachable address (e.g. `host.docker.internal` on Docker Desktop).
The remaining routing settings are listed below and exposed in `compose.yaml`.
Custom norms can be mounted read-only with a Compose override and selected via
`DISPATCH_NORMS=/path/in/container/custom.xlsx` (or `.json`).

The same image provides the CLI:

```sh
docker compose run --rm --no-deps app solve demo 1 > plan.json
docker compose run --rm --no-deps -v "$PWD:/inputs:ro" app solve /inputs/demo.json 1
```

### Image CI and GitHub Packages

[`.github/workflows/images.yml`](.github/workflows/images.yml) runs on every push
to `main`, every PR targeting `main`, and manual dispatch. It validates Compose,
runs the Rust test suite, builds both images, and smoke-tests the app's UI/API,
origin checks, CLI and persistent-volume permissions, plus MOTIS's executable.
BuildKit caches are separate for each image. Main pushes publish `main` and
`sha-...` tags; same-repository PRs publish `pr-...` and `sha-...` tags. Fork and
Dependabot PRs build/test without publishing. Manual runs publish only from main.
No `pull_request_target` workflow executes PR code.

Publishing uses the repository's `GITHUB_TOKEN` with `packages: write`; no Docker
Hub credentials are needed. Enable Actions/package publishing in repository or
organization settings if restricted. Existing packages must grant this repository
write access. This follows GitHub's
[container publishing workflow](https://docs.github.com/en/actions/tutorials/publish-packages/publish-docker-images).
CI publishes packages; deployment on a host is done with the Compose commands
above. To reproduce the application checks locally:

```sh
docker build --target test -t dispatch-sat:test .
python3 scripts/container-smoke.py http://localhost:8080
```

## CLI

```sh
# Built-in demo, 1-second search budget, machine-readable output
./target/release/dispatch-sat solve demo 1 > plan.json

# All 56 jobs from a supplied CSV, with explicitly synthetic enrichment
./target/release/dispatch-sat solve \
  'dataset/Югоцентр Синтетические данные.csv.new.csv' 5 > plan.json

# Convert CSV to editable, self-contained input JSON
./target/release/dispatch-sat import \
  'dataset/Восток Синтетические данные.csv.new.csv' > scenario.json
./target/release/dispatch-sat solve scenario.json 5 fast > plan.json

# Optional full search, retaining global-optimality proofs
./target/release/dispatch-sat solve scenario.json 5 exact > plan.json

# Optional alternate local port
./target/release/dispatch-sat serve 8081
```

Default mode: **fast**. Default maximum budget: **5 seconds**; supported range:
0.01–300. Fast mode uses a candidate-route SAT model and can use the configured
budget for stronger route generation and proof attempts; there is no fixed
500-conflict cutoff. Choose **Полный SAT** / CLI `exact` when exhaustive search
matters. Road-only/offline instances of up to 16 jobs use the full encoding even
in fast mode; small timetable instances attempt exhaustive route enumeration.

The deadline is cooperative, not a hard real-time guarantee. Baseline computation,
validation, serialization, solver termination latency and memory cleanup add
some overhead. On time/effort exhaustion the program returns the best **independently
validated** incumbent. `stats.optimal: false` is not UNSAT. An incumbent may
still be entirely heuristic when SAT has not completed an improvement query.

## City routes and service norms

Select **Город / авто · пешком · велосипед** for `city-example.json`, or import
complete real input. Open **Настройки**:

1. For a CSV, optionally run **Найти координаты адресов**. Nominatim returns its
   best match, displayed per job; failed lookups retain the old point and are
   explicitly marked. A match may be only a street/area: **review every point**.
2. Enter actual engineer bases (individually or with the common-base fields),
   transport and service durations. CSV engineers/skills/shifts remain synthetic;
   use your own JSON for real workforce data. Check the coordinate confirmation.
3. For public-transport engineers, select the service date (Moscow, UTC+03:00)
   and configure MOTIS as described below; alternatively explicitly exclude them.
   Prepare **реальные дороги**. Transport/resources are never silently changed.
4. Run SAT. Lines now follow the cached street geometry, including one-way/profile
   differences. Editing inputs invalidates the snapshot; the UI blocks a silent
   switch back to straight-line travel. The demo fallback requires an explicit click.

CLI equivalent (verify the points before `--confirmed`):

```sh
./target/release/dispatch-sat geocode scenario.json > matched.json
# Review/correct matches and engineer bases in matched.json.
./target/release/dispatch-sat route matched.json --confirmed > city.json
./target/release/dispatch-sat solve city.json 1 exact > city-plan.json
# Small road-profile example, not actual customer/engineer data:
./target/release/dispatch-sat route city-example.json --confirmed > city.json
```

Street preparation uses the explicitly selected **MOTIS or Valhalla** backend.
The installed local launcher uses MOTIS's OSM street graph for car, walking and
bicycle routes, with four bounded concurrent requests. Each directed leg's time,
distance and polyline6 geometry come from the **same routed response**. MOTIS
street queries disable transit and search up to six hours with a 250-metre street
matching radius; its street graph must be enabled and support that time limit.
Only an explicit empty direct-route result means no path within those limits.
Direct street legs must retain the requested mode and reference an OSM way or
node in their detailed steps. Synthetic near-point legs are rejected, including
when the requested mode is walking; see the local router patch below.

Valhalla remains available through its directed matrix API, also with geometry
in the same response. Small walking/cycling matrices use duplicate-point padding
to select its shape-capable CostMatrix; dummy rows/columns are discarded. Only
explicit null matrix entries mean unreachable. Unsupported geometry, upstream
failures and missing snapshots are errors, never straight-line fallbacks or an
automatic switch to another provider.

Preparation is a separate network phase and may take minutes for a whole CSV;
it is **outside the SAT budget**. MOTIS preparations allow up to twenty minutes
because they compute individual directed paths; Valhalla's batched preparation
retains its four-minute limit. Both preserve completed cache entries on failure.
MOTIS street requests, including capability checks and response bodies, use the
remaining preparation deadline rather than the ordinary 45-second HTTP cap.
DNS resolution and connection setup each have a five-second limit; requests
are not retried automatically and the preparation deadline is never extended.
Directed road legs and transit pair queries are
reused for 24 hours. Road cache keys include backend/settings, provider, transport
and both rounded coordinates: point reordering, shared bases and unrelated settings edits do not
rebuild unchanged pairs. Changing one point fetches only its incoming/outgoing
arcs, not the entire matrix. Diagonal road legs are local zero-cost journeys. Saved
snapshots never expire automatically because dispatched history must remain
replayable; keep `.routing-cache/` alongside exported city plans. Only missing
pairs/departure intervals are fetched on urgent arrival. Cache files are trusted local data, with
bounded reads, four hot snapshots and a 32 MB aggregate geometry limit. Transit
departures share identical geometry in memory and in saved snapshots: the limit
counts each distinct transit path once, not once per departure. Preparation excludes
only transitions and departure intervals that cannot fit the current job windows
and engineer shifts. Transit pair caches store parsed journeys rather than verbose
MOTIS responses; fresh full-day caches supply bounded intervals locally without
refetching or renewing their 24-hour lifetime. Older frozen snapshots remain readable.

Server configuration:
- `DISPATCH_ROAD_BACKEND`: `motis` or `valhalla`. The standalone binary defaults
  to `valhalla`; `scripts/city.sh app` defaults to local MOTIS unless a backend
  or `DISPATCH_VALHALLA_URL` was explicitly configured.
- `DISPATCH_VALHALLA_URL`: default `https://valhalla1.openstreetmap.de`.
  Use a [self-hosted Valhalla](https://valhalla.github.io/valhalla/) with suitable
  matrix distance limits for operational volumes; the public demo has no SLA.
- `DISPATCH_NOMINATIM_URL`: default `https://nominatim.openstreetmap.org`.
  Explicit, cached lookups only, one at a time and at least 1.1 seconds apart.
  Respect the provider's usage policy; no background/bulk polling.
- `DISPATCH_ROUTING_CACHE`: default `.routing-cache` under the working directory.
- `DISPATCH_MOTIS_URL`: no public default. Base URL of your MOTIS 2.11.x server
  for transit and, when selected, street routing; e.g. `http://127.0.0.1:8081`
  (not the `/api/v6/plan` URL). Car/walking/bicycle routes do not require a GTFS date.
- `DISPATCH_TRANSIT_WORKERS`: concurrent MOTIS pair requests, `1`–`8`; default
  up to `4`, capped by available CPU parallelism. Requests and completed results
  are bounded by this setting. Lower it for a shared or resource-constrained
  MOTIS server. Valhalla requests remain sequential; MOTIS street requests use four workers.

### Installed local Moscow instance

This workstation has a graph-only build of MOTIS **2.11.3** under `.transit/`, with
the archived Moscow GTFS below and the September 26, 2026
[BBBike Moscow OSM extract](https://download.bbbike.org/osm/bbbike/Moscow/).
Large inputs, binaries and generated indexes are ignored by Git; source URLs and
download checksums are recorded in `.transit/installation.json`.

- Dispatcher: **http://127.0.0.1:8080**
- MOTIS API: **http://127.0.0.1:8081**
- Router configuration: `.transit/moscow/config.yml`
- Imported dates: **2026-09-27 through 2026-10-26**, Moscow UTC+03:00.
- Ready-to-open scenario: `.transit/moscow/example.json`; import it through
  **Ваш JSON или CSV** and build a plan. It has one explicitly synthetic service
  visit using real GTFS stop coordinates and bus 327's archived timetable.
- Verified output: `.transit/moscow/verified-plan.json`.

The stock OSR dependency substitutes a one-minute straight-line `WALK` segment
for endpoints less than eight metres apart, even for `CAR`/`BIKE` requests.
`scripts/motis-strict-streets.patch` removes that shortcut from all four search
paths; distinct endpoints use the requested profile's graph search instead.
The dispatcher does not relabel walking legs, merge nearby coordinates, or
silently mark these pairs unreachable.

To rebuild the patched router on macOS, install Clang, CMake, Ninja and Git, then:

```sh
sh scripts/build-motis.sh
```

The script checks pinned MOTIS/OSR revisions, applies the patch, and builds with
two compiler jobs by default (`MOTIS_BUILD_JOBS` overrides that limit). The first
build downloads upstream dependencies. The resulting executable is
`.transit/motis-strict/motis`; the original distribution remains intact for UI
assets. The launcher requires the patched executable and never falls back to the
stock one.

Both services bind only to loopback. To start them again, use separate terminals:

```sh
sh scripts/city.sh router
sh scripts/city.sh app
```

The app launcher sets `DISPATCH_MOTIS_URL` to `http://127.0.0.1:8081` and selects
MOTIS for street routing, removing the public Valhalla/DNS dependency. Explicit
`DISPATCH_ROAD_BACKEND` takes precedence; an existing `DISPATCH_VALHALLA_URL`
selects Valhalla if no backend was specified. There is no automatic failover.
Existing frozen snapshots keep their original paths and costs; newly prepared
MOTIS routes have separate cache entries and can differ from Valhalla estimates.
The graph-only street cache namespace also excludes old MOTIS per-leg entries;
new preparations rebuild those road costs without changing frozen snapshots.
No shell-profile edits or system-wide installation are needed. These commands
do not install an automatic macOS login/reboot service.

To change the imported date range or replace a feed, stop the router first,
edit `.transit/moscow/config.yml` / replace the relevant input, then run
`sh scripts/city.sh import` and restart the router. Keep `extend_calendar: false`.
Reprepare scenarios for the new dates/data; old dispatched snapshots remain frozen.
The downloaded archive's coverage, freshness and licensing caveats below still apply.

### Public transport and Moscow data

Set an engineer's `transport` to `"public"` and the scenario's `transit_date` to
`"YYYY-MM-DD"`, or select the date in **Настройки**. All dispatch clock values use
**Moscow UTC+03:00**, including responses returned by the router in UTC.

Preparation requests MOTIS profiles only for compatible public-engineer base/job
pairs that can occur in chronological order. Each departure interval runs from the
earliest possible completion of its source job (or the engineer's shift start) to
the latest feasible service start at its destination. Bounds are combined across
engineers and colocated jobs, so no feasible route is excluded. Walking access,
transfers and egress are included; walking-only alternatives are disabled. For every
actual ready time, scheduling chooses the available journey with earliest arrival
(distance breaks ties), includes waiting, and rejects missed departures, expired
connections and arrivals after midnight. Departures round down and arrivals round
up to integer minutes. The solver, plan validator, geometry and event replay all
use the same immutable snapshot; there are no network calls during SAT search.
Snapshots record their departure coverage. Expanding job windows, shifts or skill
eligibility requires preparation again, which queries only uncovered interval edges
and retains existing departures, geometries and published history. Incomplete
coverage is an explicit error, not an unreachable route or an optimality claim.
Urgent arrivals use the same expansion rules. Changing the date requires a new snapshot.

Timetable scenarios use route-column SAT rather than the static street-time
encoding. Exact mode enumerates feasible ordered routes within its generation
budget and memory cap; global optimality is claimed only after complete enumeration
and all objective proofs. Otherwise bounds/proofs are explicitly pool-local.
These guarantees concern the frozen, earliest-arrival, integer-minute model—not
live operations or every possible deliberately delayed transit itinerary.

**Moscow GTFS exists, but coverage and freshness need care.** The
[Mobility Database catalog](https://mobilitydatabase.org/feeds/gtfs/mdb-3226)
provides a reachable
[June 30, 2026 archive](https://files.mobilitydatabase.org/mdb-3226/mdb-3226-202606301640/mdb-3226-202606301640.zip).
The producer URL was returning HTTP 404 when checked. The downloaded archive has
877 routes, 224,851 trips, and route types 0/3/5; **no subway/Metro type 1**,
no `shapes.txt`, and no calendar exceptions. Its calendars activate 52,135 trips
on 395 routes for September 27, 2026, but long calendar end dates do not establish
current operational accuracy. The catalog does not establish a redistribution
license; confirm usage rights and schedule freshness before operational use.
Transit geometry without GTFS shapes can connect successive stops rather than
follow the actual vehicle path; distance is measured along the returned geometry.
No live Moscow service accuracy or complete Metro coverage is claimed.

[OsmAnd](https://osmand.net/docs/user/navigation/routing/public-transport-navigation/)
instead uses OSM PTv2 routes and configured vehicle speeds; it is not evidence of
departure timetable availability. A newer processed
[BusMaps Moscow dataset](https://busmaps.com/en/russia/open-data-portal-moscow/moscow-official)
is another acquisition option, subject to that provider's delivery terms.

Self-host with a [MOTIS release](https://github.com/motis-project/motis/releases)
and a matching Moscow-area OSM PBF plus your licensed GTFS ZIP:

```sh
# Run in a separate MOTIS data directory containing these two downloaded inputs.
motis config moscow.osm.pbf moscow.gtfs.zip
# Edit config.yml: server.port: 8081; timetable.first_day: your service date;
# timetable.num_days: sufficient coverage; street_routing: true; osr_footpath: true.
# Keep timetable.datasets.*.extend_calendar: false; never invent service dates.
motis import
motis server
# In another terminal, from this repository:
DISPATCH_MOTIS_URL=http://127.0.0.1:8081 ./target/release/dispatch-sat serve
```

See the [MOTIS setup reference](https://github.com/motis-project/motis/blob/v2.11.3/docs/setup.md).
The public [Transitous API](https://transitous.org/api/) restricts commercial and
resource-intensive routing; it is deliberately **not** an automatic backend for
batch dispatch. Existing offline `public` demo data remains explicitly synthetic.

### `norms.xlsx`: travel is not counted twice

| Work / norm key | Technical | Documents | On site (`duration`) | Workbook total including 20-min travel |
|---|---:|---:|---:|---:|
| Connection / `connection` | 60 | 10 | **70** | 90 |
| Emergency / `emergency` | 80 | 0 | **80** | 100 |
| Equipment / `equipment` | 10 | 10 | **20** | 40 |
| Local / `local` | 30 | 0 | **30** | 50 |

CSV imports apply these norms automatically; equipment retains the connection
skill but has its own duration. Explicit JSON durations are overrides (including
the original demo). The UI offers per-type service overrides, an explicit apply
button, and per-job duration edits. Urgent-job type selection prefills the norm.

The reader uses local `norms.xlsx` when present, otherwise the bundled copy.
`DISPATCH_NORMS=/path/custom.xlsx` or `/path/custom.json` selects another file;
no rebuild is needed. JSON is the `Vec<Norm>` returned by `GET /api/norms` or:

```sh
./target/release/dispatch-sat norms > my-norms.json
DISPATCH_NORMS=./my-norms.json ./target/release/dispatch-sat serve
```

Each row has `key`, `name`, `travel`, `technical`, `documents`, `total`, `service`.
All four keys are required; times must be integers, `service = technical + documents`
(1–1440), and `total = travel + service`. Missing/duplicate/inconsistent rows fail
instead of silently using old constants. Normative travel is informational only;
scheduled travel always comes from the selected travel model.

## Demonstration

1. Open the built-in 12-job/5-engineer scenario and click **Построить SAT-план**.
2. Inspect a route or map marker: skills, transport, window, arrival, service
   start/end, and a constraint-based explanation are shown.
3. Toggle **Базовый план** to inspect the baseline's routes and individual
   engineer distances. The comparison table always shows both plans.
4. At **12:00**, select a still-pending job and click **Отменить и пересчитать**.
   The application preserves dispatched visits and shows assignment/order/time
   changes. Already-dispatched jobs are excluded from the cancellation selector.
5. Download the full result using **↓ JSON**, or load a supplied CSV/your JSON.

The demo intentionally lets the baseline assign its versatile engineer to J01,
losing urgent J02. The optimized plan serves J02 and uses more staff to complete
more work. J11 cannot finish within any shift. Minimizing staff never takes
precedence over completing urgent jobs or maximizing total coverage.

The server is a localhost-only, single-user prototype with host/origin checks:
one solve/preparation at a time to avoid CPU contention. Reload an input dataset to start a new day simulation.

## Files / architecture

```text
Browser: web/index.html (plain HTML/CSS/JS, Leaflet + OSM)
                   │ JSON / local HTTP
src/main.rs        │ CLI, small HTTP server, bounded uploads
src/import.rs      │ JSON validation / supplied semicolon-separated CSV adapter
src/norms.rs       │ XLSX/JSON norms, service = technical + documents
src/routing.rs     │ frozen street snapshots, geometry, Valhalla/geocoding/cache
src/street.rs      │ local MOTIS car/walk/bicycle routes, bounded preparation
src/app.rs         │ cancellation/urgent arrival, frozen history, daily metrics
src/model.rs       │ travel matrices, baseline, insertion seed, independent replay
src/sat.rs         │ full assignment/time CNF, deferred distance arcs, incremental CaDiCaL
src/fast.rs        │ prevalidated route pool → much smaller exact-cover SAT
src/tests.rs       │ exhaustive oracle, input, timeout and replanning checks
```

Leaflet 1.9.4 JS/CSS are **bundled locally** in `web/vendor/` (BSD-2-Clause license
included), so CDN outages cannot hide the routes. OpenStreetMap supplies background
tiles and receives the viewed tile area. **Only explicit geocoding/preparation**
sends addresses to Nominatim and coordinates to the selected road router and,
for public transport, MOTIS. The local launcher routes against loopback MOTIS.
No names/skills are sent.
If tiles or Leaflet fail, cached street geometry is still drawn (SVG fallback).
Straight connectors are used only in the explicitly labeled offline demo.

The map's SVG sizing rule intentionally targets only `#map > svg` (the standalone
fallback). Applying it to all nested SVGs collapses Leaflet's overlay against its
zero-sized positioning pane. The browser regression test checks actual rendered
SVG dimensions, not just whether route elements exist in the DOM.

## Input and output

`demo.json` is a complete, editable schema example. A scenario contains `name`,
`jobs`, `engineers`, optional `notes` and optional prepared `routing` reference.
Limits: 100 jobs, 15 engineers, one day; IDs ≤128 bytes, addresses ≤2048 bytes.
IDs must be unique within each entity type; zero-duration jobs are rejected.

| Entity | Fields / units |
|---|---|
| Job | `id`, `address` (display), `point: {lat, lon}`, positive `duration` in minutes, `window_start`, `window_end`, `skill`, optional `transport`, `urgent` (default false), optional `work_type` (norm key) and `geocode_match` (reviewable match label) |
| Engineer | `id`, `name`, `start: {lat, lon}`, `shift_start`, `shift_end`, `skills` (1–3), `transport`, optional `already_used` (default false) |
| Time | Integer minutes after midnight, 0–1440. UI formats HH:MM. Only **start** must be inside the job window; completion must be within the shift. |
| Skill | `local`, `connection`, `emergency` |
| Transport | `car`, `walk`, `bicycle`, `public`; job `transport: null` means unrestricted |
| Coordinates | WGS84 latitude/longitude in degrees; finite and range-checked |

With `routing: null`, the offline demo uses Haversine metres and constant speeds:
car 30, walk 5, bicycle 15, public 20 km/h. This is explicitly **not city routing**.
With a prepared `routing` reference, travel comes from directed, transport-specific
street paths. Their geometry and costs come from the **same matrix response**.
Kilometres are rounded up to metres; reported seconds plus a one-second allowance
for provider truncation are rounded up to minutes (zero-length legs stay zero).
These are static road-time estimates, not live traffic or measured actual arrivals.
Every route starts at the engineer's location; no return-to-base leg is required.
Public city routing uses the frozen timetables described above. Lunch breaks and
equipment inventories are not modeled.

Output includes:

- `scenario`: normalized input (cancelled jobs removed after an event);
- `plan`, `baseline`: ordered `routes` including each stop's departure, arrival,
  service start/end, leg distance and explanation; explicit `unassigned` reasons;
- `metrics`: urgent/total unassigned, daily unique engineers, total metres;
- `stats`: elapsed/generation/encoding milliseconds, variables/clauses, SAT calls,
  criterion bounds, candidate count, `scope` (`global` or `candidate_routes`),
  `search_complete` (within that scope), and `optimal` (global proof only);
  **bounds in candidate-route mode are not global lower bounds**;
- `changes`, `last_event_time`: replanning history comparison and event clock.

“Cannot serve even alone” reasons are checked directly. A feasible standalone
job omitted from the incumbent is described as competing for capacity, **not**
proven impossible. We do not fabricate individual causal/optimality explanations
or compute expensive per-job UNSAT cores.

### Supplied CSVs are incomplete planning inputs

The supplied files provide addresses, job categories and date/time windows, but
**no coordinates, service durations, engineer start points, shifts, skills or
transport types**. The adapter explicitly labels its enrichment as fictional where data is missing (service norms now come from the workbook):

- Approximate district-centre coordinates plus a deterministic address-based
  offset (not geocoding; plotted locations do not identify the real buildings).
- 12 generated engineers, mixed skills and all four transport types; common
  synthetic base at the centroid of jobs; shifts 09:00–23:59.
- Service comes from `norms.xlsx`: local 30, connection 70, additional equipment
  20, emergency 80 minutes. Emergency work is urgent; no transport requirement
  is inferred. These are supplied norms, **not fabricated service durations**.
- Blank rows and the `Адрес офиса` footer are ignored, not treated as jobs.
- The three synthetic exports contain **66 / 83 / 56 jobs** (East / Southeast /
  South-centre). All are read without sampling or truncation.
- Control exports are reference data, not solver constraints or a fair baseline:
  their brigade/status fields are ignored. The Southeast control export has
  **two contradictory rows for ID 305898293** and is rejected rather than
  silently choosing a window/status or inventing a second job.

To obtain meaningful real-world distances or comparisons, supply complete JSON
with verified coordinates and actual engineer/service data. Distances in the
CSV demonstration must not be presented as actual road mileage.

## Fast dataset mode (default)

The original full arc/time model spent hundreds of milliseconds constructing
roughly half a million variables. The unrestricted full mode has since been
optimized too (see below). The fast path instead generates feasible route **columns**:

1. Keep baseline and insertion-plan routes, their one-job-deleted variants, and
   every feasible singleton. Larger problems also seed the pool with a bounded
   unrestricted static-SAT solution. For mixed fleets this seed uses road engineers
   only, then maps the plan back into the full fleet; public engineers remain
   eligible in the final search. Subproblem proofs are not transferred.
2. Generate up to 256 deterministic multistart insertion plans within a generation
   deadline of 60% of the overall budget. Recheck useful sequences on other eligible
   engineers using their actual travel and timetable. Whole-route elimination uses
   constrained-first insertion and bounded backtracking before distance polishing.
   Deduplicate by engineer/job set, retaining the shortest feasible order.
3. One Boolean variable selects each candidate route. Choose at most one route per
   engineer and exactly one covering route or unassigned status per job. No time
   arithmetic is needed in SAT: **each whole route was already scheduled and checked**.
4. Optimize the same urgent → total coverage → staff → distance hierarchy using
   incremental CaDiCaL until proof or the caller's deadline. Unknown is never
   interpreted as UNSAT. Revalidate every decoded plan independently.

The incumbent remains representable in the pool. Incomplete route enumeration
uses `stats.scope: "candidate_routes"` and cannot claim global optimality; even
`search_complete: true` proves only the pool optimum. Increasing the time budget
can improve generation and SAT search, but does not make fast mode exhaustive.
Timetable `exact` mode shares enumeration time and remaining column capacity across
engineers, so one factorial subtree cannot consume the entire pool. Global proof
still requires complete enumeration and every objective proof.

## Full SAT model (`exact`)

For offline and road-only scenarios this mode retains **every feasible route**,
every integer minute, and global lexicographic proof scope, without a restricted
route pool. Timetable scenarios instead use the enumeration-based search described
above. The static model is based on `solution.md`, with a smaller exact formulation.
For city snapshots, directed travel need not satisfy triangle inequality.
Coverage/staffing first use the shortest-path timing closure as a **relaxation**.
Every SAT candidate is replayed with actual directed street times before acceptance.
An infeasible route adds a clause excluding only that engineer's **exact job set
and order**. Reordering it or inserting another stop remains legal—including
intermediate stops that repair nonmetric travel. Invalid candidates never update
objective bounds; UNSAT over this relaxation still proves a global lower bound.
All actual successor arcs are added before distance optimization.

City models also receive necessary capacity constraints in up to eight dense
intervals: if each of several visits consumes at least `w` service minutes inside
an interval, an engineer with `c` available minutes can handle at most `floor(c/w)`
of them. These strengthen propagation without coarsening time or excluding any
feasible route. Encoding and refinement remain within the solve budget.

1. Exactly one compatible assignment `x[e,j]` or unassigned flag `u[j]` per job;
   `y[e]` is equivalent to at least one assignment. Each assignment enforces
   base-reachability lower bounds and completion within the engineer's shift.
2. **Order-encoded time:** `q[j,t]` means `start[j] >= t`. Create thresholds only
   inside the job's feasible window and add their monotone implication chain.
   A fixed-time job needs no time variables. This replaces binary time adders
   with directly propagating temporal clauses, without coarsening the minute grid.
3. If one engineer receives jobs `i,j`, require either
   `start[j] >= start[i] + duration[i] + travel_lower[e,i,j]` or the reverse.
   Each direction is a shared implication between time thresholds. Skip only
   constraints already guaranteed by the windows; reject impossible directions.
   Before distance optimization, decode each engineer's jobs by start time.
4. **Why this is exact:** offline great-circle travel times are metric. Along any real route,
   triangle inequality and nonnegative intervening work imply every pairwise
   separation. Conversely, sorting by start time yields a route whose adjacent
   legs, base departure, windows and shift completion all satisfy the model.
   City mode uses the replay/refinement procedure described above until the
   full arc formulation is installed.
   Time-dependent transit would require a separate timetable-aware formulation.
5. **Defer all successor arcs until distance.** Once earlier objectives are
   proven, add the complete feasible arc set, source/sink and degree constraints.
   Positive service time rules out disconnected cycles. Adding these variables
   preserves every real feasible route, not a restricted route pool. Remaining
   invalid city relaxations are eliminated; the incumbent is checked again.
6. Equal-cost incoming arcs across engineers share one distance-objective literal.
   This is exact because a job has at most one selected incoming arc globally.
   Distances use carry-preserving balanced binary adders; cardinality objectives
   use truncated unary thresholds for stronger bound propagation.
7. Minimize **lexicographically**: urgent unassigned → all unassigned → daily
   engineers → metres. An interval-energy relaxation also bounds staffing: sum
   each job's minimum service overlap in an interval, allow the best possible
   choice of omitted jobs, and cover the remainder with the largest available
   shift overlaps. Windows constrain latest **start**, so completion is
   `window_end + duration`, not `window_end`.
8. CaDiCaL's SAT preset, incremental assumptions and learned clauses are reused.
   Trial bounds are never permanently asserted after UNSAT/UNKNOWN. Proven
   criteria are fixed before proceeding. Independently replay every decoded plan.

Multistart insertion and route compaction supply better feasible upper bounds
only; they do **not** restrict SAT choices. The solver keeps searching until proof
or the requested deadline.
Full optimality remains NP-hard: faster construction/propagation is not a promise
that a 100-job global distance optimum can be proved in milliseconds.

The baseline is exactly the task's sequential rule: input order, first feasible
engineer in input order, append only, no global optimization. Both baseline and
SAT use identical travel/feasibility rules. The returned incumbent is never
lexicographically worse than the baseline.

## Replanning API

```text
GET  /api/demo
GET  /api/city-demo
GET  /api/norms
POST /api/geocode  {"scenario":{...}}
POST /api/routing  {"scenario":{...}, "confirm_coordinates":true}
GET  /api/datasets
GET  /api/dataset/0
POST /api/import   {"name":"input.csv", "text":"..."}
POST /api/plan     {"scenario":{...}, "seconds":5, "mode":"fast"}
# mode can also be "exact"; omitted means fast
POST /api/plan     {"scenario":{...}, "seconds":5, "mode":"fast",
                   "previous":{...plan from previous response...},
                   "last_event_time":0,
                   "event":{"kind":"cancel","time":720,"job_id":"J06"}}
POST /api/plan     {"scenario":{...}, "seconds":5, "mode":"exact",
                   "previous":{...}, "last_event_time":720,
                   "event":{"kind":"add_urgent","time":750,"job":{
                     "id":"URG-1", "address":"Verified customer address",
                     "point":{"lat":55.748,"lon":37.612},
                     "work_type":"emergency", "skill":"emergency", "duration":80,
                     "window_start":750, "window_end":900, "transport":null}}}
```

City event clients may omit `shape` from each previous stop (the UI does this),
keeping requests under 2 MB. The server recovers it from the original snapshot
before freezing history. A supplied nonempty shape must match the saved geometry.

The service validates the submitted previous plan. It assumes engineers follow
that published plan and depart immediately after the preceding work/shift start,
possibly waiting at the next site. A visit whose **departure < event time** is
committed, including travel/waiting; cancelling it is rejected. At the exact
departure instant it is still cancellable. Completed/committed prefixes are
immutable, remaining starts move to the resulting locations/availability, and
already-used engineers still count toward daily staff. Distance bounds include
the constant frozen-prefix distance in the response. Later events cannot move
the clock backwards when the returned `last_event_time` is supplied.

The baseline after an event plans the *same remaining problem and same actual
frozen history*, rather than pretending the old baseline was executed.
Optimality after an event is conditional on that frozen history. The UI disables
start-of-day planning after an event until the dataset is reloaded, preventing
accidental rewriting of completed work. The API is stateless and trusts the
caller to carry returned state forward; it is not an authenticated audit log.

Implemented events: cancellation and **`add_urgent`**. Added jobs are forced urgent,
validated for duplicate IDs/capacity/duration, and their windows cannot precede
arrival. Urgency prioritizes coverage, not immediate preemption; use `window_end`
as the response deadline. Both events freeze exactly the same dispatched history. City snapshots
are extended only for new coordinates, preserving every old arc and geometry.
Engineer absence and a plan-stability objective for unfrozen work are not included.

## Verification and performance

```sh
cargo test --release --locked
cargo clippy --all-targets -- -D warnings
python3 scripts/routing-smoke.py  # offline mock HTTP: new-point expansion and history
python3 scripts/benchmark.py --seconds 1 --runs 3 --mode fast
python3 scripts/benchmark.py --seconds 1 --runs 3 --mode exact

# Optional real-browser regression test (Node 22+, running Rust server)
CHROME_BIN=/path/to/chromium node scripts/ui-smoke.mjs
```

Tests cover all 4-bit adder/comparator inputs, weighted carry overflow, temporary
UNSAT assumptions, 120 small randomized instances against exhaustive assignment/
permutation enumeration, order-time implications, truncated cardinality counters,
deferred-route handoff and distance aggregation, dataset model-size regressions,
late-day times, disconnected-cycle prevention, empty
inputs, all supplied CSVs (including the conflicting-ID rejection), invalid
inputs, workbook parsing/overrides, directed nonmetric routes against enumeration,
urgent addition/cancellation, omitted-geometry recovery, timeout fallback,
mid-day history preservation, and the fast route pool
on all three datasets (including zero-distance routes and honest proof scope).
The browser test covers actual route rendering, editable norms, the urgent form,
history locks, baseline toggle, dataset load, cancellation, mobile, and offline SVG.
Optional `CITY_RESULT=/path/to/city-result.json` also verifies live street shapes
and prevents silent fallback after city coordinates change.

Initial fast-mode comparison (before the subsequent full-mode optimization):
**Apple M2, macOS arm64**, release build, three runs, **1-second maximum budget**.
**Historical: these timings/coverage figures predate Excel norms and city routing.** Wall time includes process startup/output/cleanup.
Fast mode stops early on its work limit. Metrics are from the final repetition
and may vary with available CPU time. This compares different search spaces/
effort policies, not an equally exhaustive solver speedup.

| Input | Jobs | Previous wall | Fast wall | Fast CNF build | Unassigned | Engineers / baseline | km / baseline | Global optimum proven |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| Built-in demo | 12 | 97 ms | 100 ms | 2 ms | 1 | 3 / 2 | 7.86 / 10.91 | Yes (full model) |
| East CSV | 66 | 1097 ms | 26 ms | 2 ms | 0 | 6 / 12 | 133.75 / 247.11 | No |
| Southeast CSV | 83 | 1077 ms | 30 ms | 3 ms | 0 | 9 / 12 | 513.47 / 1023.83 | No |
| South-centre CSV | 56 | 1033 ms | 25 ms | 2 ms | 0 | 4 / 11 | 148.89 / 242.86 | No |

### Unrestricted full-mode optimization

Historical measurements before the norms/routing change: same machine, three
repetitions and the same **one-second `exact` budget** on
both versions. The route search space and global-proof meaning are unchanged.
Dataset variable counts are for the initial coverage/staffing model; distance
arcs are now added only when that phase is reached.

| Input | CNF build before → after | Variables before → after | Staff before → after | km before → after |
|---|---:|---:|---:|---:|
| East | 287 → 29 ms | 625,576 → 13,799 | 7 → 6 | 154.76 → 133.75 |
| Southeast | 351 → 63 ms | 749,173 → 24,790 | 9 → 8 | 627.48 → 404.00 |
| South-centre | 217 → 22 ms | 491,739 → 11,319 | 5 → 4 | 159.28 → 148.89 |

All dataset jobs are assigned in both versions. The demo reaches the **same
proven global optimum in 40 ms instead of 100 ms** (median wall time).
The dataset runs **still use approximately the whole one-second budget and do
not finish a global proof**. These are construction-size and incumbent-quality
improvements, not a claim of 10× faster completion of the full optimization.
Use the `exact` budget to choose how long to search; progress is reported per
criterion and timeout remains explicitly unproven. For an interactive full-model
run, for example:

```sh
./target/release/dispatch-sat solve scenario.json 0.15 exact
```

With that 150 ms budget the tested datasets returned in roughly **150–180 ms**,
serving all jobs with 6/8/4 engineers respectively; these are feasible incumbents,
not completed global proofs. The full model was built and queried in each run.

### City staffing regression: the exported 66-job East scenario

The supplied `dispatch-plan.json` reached **6 engineers / 257.38 km in 300 seconds**.
With lazy street refinement and interval capacity cuts, three completed cold-start
**30-second** runs reached **6 engineers / 277.27 km**, all 66 jobs served. The
staff minimum of six is proven; the distance optimum is **not**. This recovers
staffing much sooner, not an equivalent-distance solution or a guarantee for all
budgets: at one/five seconds this scenario still returned seven engineers.

Reproduce locally (requires the same `.routing-cache` snapshot):

```sh
python3 scripts/staffing-benchmark.py dispatch-plan.json --seconds 30 --runs 3
```

The script passes **only the scenario**, never the saved six-engineer plan, to
SAT. It checks coverage/staffing against the export. Results depend on CPU load;
use this as a performance regression check, not a hard real-time guarantee.

### Earlier exact-mode incumbent polishing measurements (with workbook norms)

Staffing proofs can consume the entire budget before the distance objective is
reached. Exact mode now polishes feasible SAT incumbents with job relocation,
swaps, segment reversal, and whole-route elimination. Up to **10% of the budget,
capped at 100 ms**, is reserved for final polishing. Complete affected routes
are checked against the same directed matrices, windows, skills and shifts.
These moves change **only the incumbent**, never restrict SAT's route choices;
assignment coverage cannot worsen, daily paid staff remain counted, and stage
upper bounds are refreshed. Local optimality is **not** global optimality.

Same M2, release, medians of three **one-second exact** runs before/after this
polishing change. No changes to durations, coordinates, matrices or workforce
between each pair. All jobs are assigned; none of these global proofs finishes.

| Fixture | Engineers before / after | km before → after |
|---|---:|---:|
| Cached East street matrix, 66 jobs | 7 / 7 | **338.92 → 186.53** |
| East, offline travel | 6 / 6 | 206.53 → 141.75 |
| Southeast, offline travel | 9 / 9 | 539.10 → 491.87 |
| South-centre, offline travel | 5 / 5 | 140.26 → 116.32 |

The street fixture uses the cached matrix with imported CSV job coordinates,
synthetic workforce/common base, and public engineers explicitly excluded;
it is a reproducible local comparison, **not independently verified field data**.
Wall times after the change were about 0.92–0.98 seconds. Improvements vary with
input and budget; fewer engineers still outrank fewer kilometres. Longer Excel
service norms and real street travel must not be compared as though they were
the old, shorter synthetic problem.

The demo baseline misses one additional job, including an urgent job; its smaller
staff count is therefore not a better solution. CSV metrics reflect the explicit
synthetic assumptions above, not real dispatch performance. In full mode a short
budget can still be dominated by proving staff count; incumbent polishing now
improves mileage without waiting for that proof. Fast mode also uses the requested
deadline, but its proof remains limited to generated routes.
Choose full mode and increase the budget when proof matters; do not mistake a
quick valid incumbent for a demonstrated global optimum.

Next useful extensions: timetable-backed public transit, engineer-absence events,
and state-preserving asynchronous multi-user serving.
Not included: authentication, persistent storage, enterprise FSM integration,
public deployment or a slide deck; this README provides a live-demo outline.
