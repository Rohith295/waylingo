# Bindings: Python, Swift, Kotlin

The same API in three languages, generated with [UniFFI](https://mozilla.github.io/uniffi-rs/)
from `crates/waylingo-ffi`. The guide in `crates/waylingo-ffi/README.md` has the build
commands; this chapter explains the shape of the API and the choices behind it.

## Status

* **Python:** built and tested live; an offline test runs in CI.
* **Swift (macOS):** built and tested live with `crates/waylingo-ffi/examples/swift/run.sh`.
* **Kotlin:** CI compiles the generated bindings and runs offline tests on the JVM
  (`crates/waylingo-ffi/tests/kotlin-jvm`).
* **iOS and Android:** the release workflow builds the iOS XCFramework and the Android
  library (AAR); neither has yet been run in an app on a device.
* **Packages:** version 0.1.2 is on PyPI (`waylingo`), Maven Central
  (`io.github.rohith295:waylingo-android`) and Swift Package Manager (this repository).
  On `main`, `Package.swift` holds placeholder URL and checksum values; the release
  workflow fills them in on each release tag, so depend on a version, not on `main`.

## The API

One object, created once per process:

```text
Waylingo(config, map_key, llm_key = None, signing_key = None)
  plan(trip)                                  -> TripOutput    async
  reroute(previous_geojson, choices, request_id = None) -> TripOutput    async
  parse(trip)                                 -> ParseOutput   async
  plan_blocking(trip), reroute_blocking(...), parse_blocking(trip)   blocking
is_retryable(code), needs_user(code), version()
```

```python
import waylingo as w

app = w.Waylingo(
    w.WaylingoConfig(map_provider=w.MapProvider.TOMTOM,
                     llm=w.LlmBackend.ANTHROPIC(model=None, base_url=None)),
    map_key=TOMTOM_KEY, llm_key=ANTHROPIC_KEY)

out = app.plan_blocking(w.TripInput(
    query="Drive to Utrecht, coffee halfway",
    current_location=w.LatLng(lat=52.3702, lng=4.8952)))
```

Rerouting after the end user picks the first alternative of the first stop:

```python
stop = out.stops[0]
choices = w.Choices(stops=[
    w.StopChoice(
        stop_request_id=stop.stop_request_id,
        occurrence=stop.occurrence,
        pick=w.StopPick.PLACE(
            provider_place_id=stop.alternatives[0].provider_place_id),
    ),
])
again = app.reroute_blocking(out.geojson, choices)   # no LLM call, one map call
```

The same plan call in Swift (inside an `async` context) and Kotlin (inside a
coroutine):

```swift
let app = try Waylingo(
    config: WaylingoConfig(mapProvider: .tomtom, llm: .anthropic(model: nil, baseUrl: nil)),
    mapKey: tomtomKey, llmKey: anthropicKey)
let out = try await app.plan(trip: TripInput(
    query: "Drive to Utrecht, coffee halfway",
    currentLocation: LatLng(lat: 52.3702, lng: 4.8952)))
```

```kotlin
val app = Waylingo(
    WaylingoConfig(mapProvider = MapProvider.TOMTOM, llm = LlmBackend.Anthropic(null, null)),
    mapKey = tomtomKey, llmKey = anthropicKey)
val out = app.plan(TripInput(
    query = "Drive to Utrecht, coffee halfway",
    currentLocation = LatLng(52.3702, 4.8952)))
```

## Parse only

`parse` runs only the LLM step: one LLM call (more if the reply needs a repair) and no
map call. It returns the plan the request compiled to, for example to show the end
user what was understood before spending map calls, or to test a model on your own
requests.

```python
p = app.parse_blocking(w.TripInput(
    query="Drive to Utrecht, coffee halfway",
    current_location=w.LatLng(lat=52.3702, lng=4.8952)))
if p.status == w.ParseStatus.PARSED:
    print(p.plan_json)   # the same "plan" a plan() call puts in its GeoJSON
else:
    print(p.error_code, p.error_message)
```

Nothing is looked up on the map, so the plan holds the place names from the request,
not coordinates; the only coordinates in it are the ones the app passed in, such as
`current_location` as the origin. The error codes are the same as for `plan`. A later
`plan` call parses the request again; there is no call that routes a plan you already
have. In Rust the same is `Engine::parse`, `waylingo_reqwest::drive_parse` and
`AsyncHost::parse`, which return a `ParseResult` document
(`crates/waylingo/src/result.rs`).

## Results

`TripOutput` (`crates/waylingo-ffi/src/result.rs`) has the full `geojson` plus the
same facts as typed fields: `status`, `error_code`, `error_message`, `stops` (pick,
alternatives, `eta_s`, `beyond_radius`, `user_choice`), `unresolved`, `locations` (with
`candidates`), route length and time, `warnings`, `assumptions`, `attribution`, and the
call counts. An app can build `Choices` from these fields and pass `out.geojson` back to
`reroute`.

`TripInput` (`crates/waylingo-ffi/src/trip.rs`) carries the query and context:
`current_location`, `heading_deg`, `locale`, `saved_places`, `active_destination`, `ev`
(minimum power and connectors), `avoid`, `departure_time`, `timezone`, `request_id`.

## Configuration

`WaylingoConfig` (`crates/waylingo-ffi/src/config.rs`):

| Field | Default | Notes |
| :--- | :--- | :--- |
| `map_provider` | required | `TOMTOM` or `HERE` |
| `llm` | required | see below |
| `map_base_url` | none | your map proxy |
| `settings` | Waylingo defaults | `anchor_radius_m`, `search_span_m`, `search_limit`, `max_detour_s`, `top_n`, `strict_radius` |
| `max_map_calls` | 25 | at least 3 |
| `map_requests_per_second` | 4.5 | pacing across every query on the instance |
| `timeout_ms` | 60,000 | whole query, including waiting for map slots; at least 1,000 |

LLM backends:

| Backend | Default endpoint | Output mode |
| :--- | :--- | :--- |
| `Anthropic { model, base_url }` | `https://api.anthropic.com/v1`; `model` defaults to `claude-sonnet-5-5` | `tool_call` |
| `OpenAi { model, base_url }` | `https://api.openai.com/v1` | `json_schema` |
| `Gemini { model, base_url }` | Google's OpenAI-compatible endpoint | `json_schema` |
| `OpenAiCompatible { base_url, model, fine_tuned }` | your server | `prompt_json`; compact prompt when `fine_tuned` |

Pin the Anthropic model in shipped apps: a default baked into an app may be retired by
the provider. The bindings guide notes that Gemini has not been tested live yet.

## Async and blocking

`plan` and `reroute` are async: `await` in Python's asyncio, `async` in Swift, `suspend`
functions in Kotlin. They are exported with UniFFI's async support
(`#[uniffi::export(async_runtime = "tokio")]`). `plan_blocking` and
`reroute_blocking` run the same futures on one process-wide tokio runtime with two
worker threads, created on first use (`RUNTIME` in
`crates/waylingo-ffi/src/lib.rs`). They block the calling thread for up to `timeout_ms`,
so never call them on a UI thread.

**One instance per process.** The instance holds the HTTP client (and its connections)
and the map pacing gate. Pacing works across queries only if they share the instance.
On servers that fork (for example gunicorn with `--preload`), create it after the fork.

## Keys in shipped apps

Keys are constructor arguments, never fields of a record, so they cannot appear in a
generated `toString()` or `repr()`. The constructor refuses an empty map key unless
`map_base_url` is set, and an empty LLM key for a hosted backend unless that backend has
a `base_url`.

A key compiled into an app can be extracted. For apps in end users' hands:

* set `map_base_url` to your proxy, which adds the provider key;
* give the LLM backend a `base_url` pointing at your proxy;
* if your server takes results back from clients, give it a `signing_key` (see
  [Signing](reroute.md#signing)).

Running the LLM step on the device with the [fine-tuned parser](fine-tuning.md) is
planned (a callback to an on-device runtime) and not built yet.

## Android and TLS roots

On Android, `waylingo-ffi` enables the `bundled-roots` feature of `waylingo-reqwest`
(`crates/waylingo-ffi/Cargo.toml`). The HTTP client then trusts only the root
certificates bundled from `webpki-root-certs` (Mozilla's root store), so no TLS set-up
is needed in the app. The trade-off: user-installed and enterprise certificate
authorities are not trusted, so networks that intercept TLS (some corporate proxies)
will fail with `PROVIDER_UNREACHABLE` or `LLM_UNREACHABLE`.

## Packaging

* Python: `maturin` builds wheels from `crates/waylingo-ffi/pyproject.toml`.
* Swift: an XCFramework of static libraries (iOS device, iOS simulator, macOS), plus the
  generated `Waylingo.swift`; `Package.swift` at the repository root points at the
  XCFramework attached to a release. The Swift module is `Waylingo`; the C module is
  `WaylingoFFI` (`crates/waylingo-ffi/uniffi.toml`).
* Kotlin: package `io.waylingo`, using JNA; an Android library (AAR) is built from
  `android/`.

See [Testing and releases](testing-and-releases.md) for how these are built.
