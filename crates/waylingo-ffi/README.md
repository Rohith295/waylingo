# Waylingo bindings: Python, Swift, Kotlin

The same API in all three languages, generated with
[UniFFI](https://mozilla.github.io/uniffi-rs/) from this crate.

```text
plan(trip)                          end user's request  ->  result
reroute(previous_geojson, choices)  end user's choices  ->  new result (no LLM call)
```

You bring two keys: your map provider's (TomTom or HERE) and your LLM provider's
(Anthropic, OpenAI, Gemini, or any OpenAI-compatible server). The LLM only reads
the request text; map data never reaches it.

Status: Python and Swift (macOS) are built and tested live. Kotlin is compiled and
tested on the JVM in CI. iOS and Android device builds are documented below but not
yet verified on a device.

## The loop in an app

1. The end user types or says a trip: "drive to Utrecht, coffee halfway".
2. `plan` returns a `TripOutput`:
   * `status` (`COMPLETE`, `PARTIAL`, `FAILED`), `error_code`, `error_message`.
     See "Error codes" below. A trip may have no stops ("drive to Utrecht").
   * `stops`: for each requested stop, the `pick` and its `alternatives`.
   * `unresolved`: stops without a place, with a `reason`.
   * `locations`: origin, waypoints and destination, with `candidates` when the
     text matched several places ("did you mean ...?").
   * `geojson`: the same as a GeoJSON `FeatureCollection` to draw on a map.
   * `attribution`: show it with the map, as the provider's terms require.
3. Show the options. The end user picks another stop, skips one, or picks another
   candidate for the destination.
4. `reroute(out.geojson, choices)` returns the new result. A stop change costs one
   map call; a new destination is searched again. Earlier choices carry over.
   Keep the previous result to undo.

## Python

```bash
cd crates/waylingo-ffi
pip install maturin
maturin develop --release        # or: maturin build --release (a wheel in target/wheels)
```

```python
import waylingo as w

app = w.Waylingo(
    w.WaylingoConfig(
        map_provider=w.MapProvider.TOMTOM,
        llm=w.LlmBackend.ANTHROPIC(model=None, base_url=None),
        settings=w.SearchSettings(anchor_radius_m=5000, top_n=3),
    ),
    map_key=TOMTOM_KEY,
    llm_key=ANTHROPIC_KEY,
)
out = app.plan_blocking(w.TripInput(
    query="Drive to Utrecht, coffee halfway",
    current_location=w.LatLng(lat=52.3702, lng=4.8952),
))
if out.status == w.TripStatus.FAILED:
    print(out.error_code, out.error_message)

stop = out.stops[0]
again = app.reroute_blocking(out.geojson, w.Choices(stops=[
    w.StopChoice(stop_request_id=stop.stop_request_id, occurrence=stop.occurrence,
                 pick=w.StopPick.PLACE(provider_place_id=stop.alternatives[0].provider_place_id)),
]))
```

`await app.plan(trip)` and `await app.reroute(...)` work under asyncio.
Live check: `python examples/python/smoke.py`.

## Swift (iOS, macOS)

On macOS, `examples/swift/run.sh` builds the library, generates the bindings and
runs a live check. For an iOS app, build an XCFramework (needs `rustup`; not yet
verified end to end):

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo build -p waylingo-ffi --release --target aarch64-apple-ios
cargo build -p waylingo-ffi --release --target aarch64-apple-ios-sim
cargo build -p waylingo-ffi --release --target x86_64-apple-ios
cargo run -p waylingo-ffi --features bindgen --bin uniffi-bindgen -- generate \
  --library target/aarch64-apple-ios/release/libwaylingo_ffi.a --language swift --out-dir bindings/swift

# Headers in their own folder; the module map must be named module.modulemap.
mkdir -p bindings/headers
cp bindings/swift/WaylingoFFI.h bindings/headers/
cp bindings/swift/WaylingoFFI.modulemap bindings/headers/module.modulemap
# One simulator library for both architectures.
mkdir -p target/ios-sim
lipo -create target/aarch64-apple-ios-sim/release/libwaylingo_ffi.a \
  target/x86_64-apple-ios/release/libwaylingo_ffi.a -output target/ios-sim/libwaylingo_ffi.a
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libwaylingo_ffi.a -headers bindings/headers \
  -library target/ios-sim/libwaylingo_ffi.a -headers bindings/headers \
  -output Waylingo.xcframework
```

Add `Waylingo.xcframework` and `bindings/swift/Waylingo.swift` to the app.

```swift
let app = try Waylingo(
    config: WaylingoConfig(mapProvider: .tomtom, llm: .anthropic(model: nil, baseUrl: nil)),
    mapKey: tomtomKey, llmKey: anthropicKey)
let out = try await app.plan(trip: TripInput(
    query: "Drive to Utrecht, coffee halfway",
    currentLocation: LatLng(lat: 52.3702, lng: 4.8952)))
```

## Kotlin (Android, JVM)

Compiled and tested on the JVM in CI; not yet run on an Android device. The source is generated into
`bindings/kotlin/io/waylingo/waylingo.kt`. Build the native library per ABI (needs
`rustup` and the Android NDK); on Android the library trusts bundled Mozilla root
certificates, so no TLS set-up is needed. Those do not include user-installed or
enterprise certificate authorities, so networks that intercept TLS (some corporate
proxies) will fail:

```bash
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk
cargo ndk -t arm64-v8a -t x86_64 -o app/src/main/jniLibs build -p waylingo-ffi --release
cargo run -p waylingo-ffi --features bindgen --bin uniffi-bindgen -- generate \
  --library target/aarch64-linux-android/release/libwaylingo_ffi.so --language kotlin --out-dir bindings/kotlin
```

```kotlin
// build.gradle.kts
dependencies {
    implementation("net.java.dev.jna:jna:5.14.0@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.1")
}
```

```kotlin
val app = Waylingo(
    WaylingoConfig(mapProvider = MapProvider.TOMTOM, llm = LlmBackend.Anthropic(null, null)),
    mapKey = tomtomKey, llmKey = anthropicKey)
val out = app.plan(TripInput(query = "Drive to Utrecht, coffee halfway",
    currentLocation = LatLng(52.3702, 4.8952)))   // a suspend function
```

## Errors

A call either **throws** (the request or configuration is unusable, or it ran out of
time) or **returns** a `TripOutput` whose `status` and `error_code` say how it went.

### Thrown

| Exception | When | What to do |
| :--- | :--- | :--- |
| `InvalidConfig` | Bad configuration: an empty key, a bad `base_url`, a timeout under 1 s | Fix the configuration |
| `InvalidRequest` | Empty or over-long query; more than 50 saved places, or a name over 60 characters; coordinates out of range; settings out of range | Fix the input |
| `IncompatibleResult` | `reroute` got something that is not a result from this server: another version, provider, or signing key; no plan | Plan again |
| `UnknownChoice` | A choice names a stop, place or candidate the result did not offer | Rebuild the choices from the latest result |
| `Timeout` | The query took longer than `timeout_ms` | Retry; raise `timeout_ms` if it happens often |
| `Internal` | A bug in Waylingo | Report it with the message |

### Returned: `error_code`

When `status` is `FAILED`, `error_code` says why; `error_message` has detail for your
logs. Codes are only ever added. In Swift and Kotlin the enum is exhaustive, so give
`switch`/`when` a default branch, or a new Waylingo version will not compile against
it. `UNKNOWN` stands in for a code from a newer version. `is_retryable(code)` and
`needs_user(code)` group the codes; no code is in both groups.

| Code | What happened | What the app should do | Retryable | Needs user |
| :--- | :--- | :--- | :---: | :---: |
| `NOT_UNDERSTOOD` | Not a trip request | Ask the end user to rephrase, with an example | | yes |
| `NEEDS_DESTINATION` | No destination named and no active route passed. If the model left out a destination, it is asked once and may only use a place named in the request | Ask "where to?" | | yes |
| `NEEDS_CURRENT_LOCATION` | The trip starts here, but `current_location` was not passed | Pass the location, or ask where to start | | yes |
| `INVALID_PLAN` | The model's plan stayed invalid after the repairs | Ask to rephrase; if frequent, use a stronger model | | yes |
| `NO_GEOCODE` | A place in the request was not found | Show the text and ask for a more precise one | | yes |
| `NO_ROUTE` | No route between the places | Tell the end user; suggest other places | | yes |
| `LLM_REFUSED` | The model declined the request | Ask to rephrase | | yes |
| `LLM_AUTH` | LLM key wrong, expired, or without access to the model | Fix the key | | |
| `LLM_QUOTA_EXCEEDED` | LLM account out of quota or credit | Add credit or raise the limit | | |
| `LLM_RATE_LIMITED` | LLM still rate-limited after the retries | Retry later; raise your limit | yes | |
| `LLM_BAD_REQUEST` | LLM rejected the request: often a wrong or retired model name, or a wrong `base_url` | Fix the configuration | | |
| `LLM_BAD_RESPONSE` | The LLM answered in a format Waylingo cannot read | Check the endpoint is OpenAI-compatible; update Waylingo | | |
| `LLM_UNAVAILABLE` | LLM 5xx or timeouts after the retries | Retry later | yes | |
| `LLM_UNREACHABLE` | No connection to the LLM (network, DNS, TLS) | Check connectivity; on a phone, often no signal | yes | |
| `PROVIDER_AUTH` | Map key wrong, expired, or missing a product. TomTom also uses 403 for some over-limit cases | Check the key and the plan's limits | | |
| `PROVIDER_QUOTA_EXCEEDED` | The map provider said the quota is used up | Upgrade the plan or wait for the reset | | |
| `PROVIDER_RATE_LIMITED` | Map provider still rate-limited after the retries | Retry later; lower `map_requests_per_second` | yes | |
| `PROVIDER_BAD_REQUEST` | The map provider rejected a request | Report it with `error_message` | | |
| `PROVIDER_BAD_RESPONSE` | The map provider answered in a format Waylingo cannot read | Update Waylingo | | |
| `PROVIDER_UNAVAILABLE` | Map provider 5xx after the retries | Retry later | yes | |
| `PROVIDER_UNREACHABLE` | No connection to the map provider (network, DNS, TLS) | Check connectivity, and proxies that intercept TLS | yes | |
| `INTERNAL` | A bug in Waylingo | Report it with `error_message` | | |
| `UNKNOWN` | A code from a newer Waylingo version | Treat as a generic failure | | |

### A stop that was not found

It does not fail the trip: `status` is `PARTIAL` and the stop is in `unresolved`
with a `reason` (`NO_CANDIDATES`, `SKIPPED_BY_USER`, `SKIPPED_BUDGET`, `UNSUPPORTED`,
`PROVIDER_ERROR`, `NOT_SEARCHED`). For `PROVIDER_ERROR`, `error_code` says what
failed, with the same codes as above.

## Using it well

* **One instance per process.** Create `Waylingo` once and reuse it: it holds the
  HTTP connections and the map request pacing. On servers that fork (for example
  gunicorn with `--preload`), create it after the fork.
* **Never call the blocking methods on a UI thread.** They wait for the whole
  query (up to `timeout_ms`). Use the async methods in apps.
* **Map request pacing.** `map_requests_per_second` (default 4.5, under TomTom's
  default 5 per second) applies across every query on the instance. Raise it to
  match your plan; queries waiting for a slot count against `timeout_ms`.

## Keys in shipped apps

A key compiled into an app can be extracted. Keep provider keys on your server:

* **Map proxy:** set `map_base_url` to your proxy, which adds the provider key.
  `map_key` may then be empty, or carry your proxy's own token.
* **LLM proxy:** give the backend a `base_url` (for example
  `LlmBackend.ANTHROPIC(model=None, base_url="https://llm.myapp.example/v1")`);
  `llm_key` may then be left out.

Running the LLM step on the device with Waylingo's fine-tuned model is planned (a
callback to an on-device runtime), not yet available.

Servers that take results back from clients should pass a `signing_key` (32
random bytes): results are then signed, and `reroute` only accepts results this
server produced. Clients may empty the route line's coordinates (or set its
geometry to null) before sending a result back; the signature does not cover them.

## LLM backends

| Backend | Default endpoint | Notes |
| :--- | :--- | :--- |
| `Anthropic` | `api.anthropic.com/v1` | `model` defaults to `claude-sonnet-5-5`; pin it in apps |
| `OpenAi` | `api.openai.com/v1` | JSON schema output |
| `Gemini` | Google's OpenAI-compatible endpoint | not yet tested live |
| `OpenAiCompatible` | your `base_url` | local servers; set `fine_tuned` for Waylingo's model |

## Before publishing

`.github/workflows/release.yml` builds the wheels (manylinux, musllinux, macOS
universal2, Windows) and sdist, the XCFramework for the root `Package.swift`, and
the AAR from `android/`. It is a dry run unless started with `publish` on. CI
compiles and tests the Kotlin bindings on the JVM.

Not done yet: a first run of these workflows on GitHub (the Android module and the
Kotlin test have not been built locally), and the PyPI trusted publisher, Maven
Central namespace and signing key.
