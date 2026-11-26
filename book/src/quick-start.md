# Quick start

Waylingo is a Rust library with the same API in Python, Swift and Kotlin, so it runs
in Android and iOS apps, on servers and in scripts:

| Where | Language | Install |
| :--- | :--- | :--- |
| Android | Kotlin | Gradle: `io.github.rohith295:waylingo-android` ([below](#android-kotlin)) |
| iOS, macOS | Swift | Swift Package: `https://github.com/Rohith295/waylingo` ([below](#ios-and-macos-swift)) |
| Servers, scripts | Python | `pip install waylingo` ([below](#python)) |
| Anything in Rust | Rust | crates `waylingo` and `waylingo-reqwest` ([below](#rust)) |

The current version is 0.1.0. Every example plans one trip and needs a map key (TomTom here) and
an LLM key (Anthropic here). See [Keys in shipped apps](bindings.md#keys-in-shipped-apps)
before putting keys in an app.

To try requests in a browser before writing code, run the
[demo](https://github.com/Rohith295/waylingo/tree/main/demo): a local page where you
paste your own keys (or use the built-in model) and see the route, the stops and
their coordinates on a map.

The output contains map provider content (place names). Show it to the end user; do
not send it to an LLM.

## Python

```bash
pip install waylingo
```

Wheels are published for Linux (x86_64, aarch64), macOS and Windows, Python 3.9 and
later.

```python
import os
import waylingo as w

app = w.Waylingo(
    w.WaylingoConfig(
        map_provider=w.MapProvider.TOMTOM,
        llm=w.LlmBackend.ANTHROPIC(model=None, base_url=None),
    ),
    map_key=os.environ["TOMTOM_API_KEY"],
    llm_key=os.environ["LLM_API_KEY"],
)

out = app.plan_blocking(w.TripInput(
    query="Drive to Utrecht, coffee halfway",
    current_location=w.LatLng(lat=52.3702, lng=4.8952),
))

if out.status == w.TripStatus.FAILED:
    print("failed:", out.error_code, out.error_message)
else:
    print(out.status, len(out.stops), "stops,", out.map_calls, "map calls")
    print(out.attribution)
```

`model=None` uses the default Anthropic model; set it explicitly in shipped apps.
In async code use `await app.plan(...)`.

## iOS and macOS (Swift)

Add the package in Xcode (File > Add Package Dependencies) with the URL `https://github.com/Rohith295/waylingo` and a version, or in
`Package.swift`:

```swift
.package(url: "https://github.com/Rohith295/waylingo", from: "0.1.0")
// and in the target: .product(name: "Waylingo", package: "waylingo")
```

The package downloads a prebuilt XCFramework from the GitHub Release; no Rust is
needed.

```swift
import Waylingo

let app = try Waylingo(
    config: WaylingoConfig(mapProvider: .tomtom, llm: .anthropic(model: nil, baseUrl: nil)),
    mapKey: tomtomKey, llmKey: anthropicKey)

// Inside an async context, never on the main thread's blocking path:
let out = try await app.plan(trip: TripInput(
    query: "Drive to Utrecht, coffee halfway",
    currentLocation: LatLng(lat: 52.3702, lng: 4.8952)))

if out.status == .failed {
    print("failed:", String(describing: out.errorCode))
} else {
    print(out.status, out.stops.count, "stops")
}
```

The minimum versions are iOS 15 and macOS 12.

## Android (Kotlin)

Add the library from Maven Central in `build.gradle.kts`:

```kotlin
dependencies {
    implementation("io.github.rohith295:waylingo-android:0.1.0")
}
```

It brings JNA and kotlinx-coroutines with it.

The app needs the internet permission in `AndroidManifest.xml`:

```xml
<uses-permission android:name="android.permission.INTERNET" />
```

```kotlin
import io.waylingo.*

val app = Waylingo(
    WaylingoConfig(mapProvider = MapProvider.TOMTOM, llm = LlmBackend.Anthropic(null, null)),
    mapKey = tomtomKey, llmKey = anthropicKey)

// plan is a suspend function: call it from a coroutine, for example in a ViewModel.
viewModelScope.launch {
    val out = app.plan(TripInput(
        query = "Drive to Utrecht, coffee halfway",
        currentLocation = LatLng(52.3702, 4.8952)))
    if (out.status == TripStatus.FAILED) {
        Log.w("trip", "failed: ${out.errorCode}")
    } else {
        Log.i("trip", "${out.stops.size} stops")
    }
}
```

The minimum Android version is 7.0 (API 24).

## Rust

`Cargo.toml`:

```toml
[dependencies]
waylingo = "0.1"
waylingo-reqwest = "0.1"
```

`src/main.rs`:

```rust
use waylingo::{
    Coords, Engine, EngineConfig, LlmConfig, LlmMode, ProviderChoice, TripContext, TripRequest,
    TripResult,
};
use waylingo_reqwest::{Keys, drive};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::new(EngineConfig::new(
        LlmConfig::new("https://api.anthropic.com/v1", "claude-sonnet-5-5", LlmMode::ToolCall),
        ProviderChoice::TomTom { base_url: None },
    ));
    let request = TripRequest {
        query: "Drive to Utrecht, coffee halfway".into(),
        context: TripContext {
            current_location: Some(Coords { lat: 52.3702, lng: 4.8952 }),
            ..TripContext::default()
        },
        request_id: None,
    };
    let keys = Keys {
        llm: std::env::var("LLM_API_KEY")?,
        map: std::env::var("TOMTOM_API_KEY")?,
    };
    let geojson = drive(&engine, request, &keys)?;
    let result = TripResult::parse(&geojson)?;
    println!(
        "status {:?}, {} features, {} map calls",
        result.status,
        result.features.len(),
        result.usage.map_calls
    );
    std::fs::write("trip.geojson", geojson)?;
    Ok(())
}
```

```bash
LLM_API_KEY=... TOMTOM_API_KEY=... cargo run
```

`drive` is the blocking host: it sends one request at a time and spaces map requests
220 ms apart. For apps and servers, use `waylingo_reqwest::AsyncHost`, which sends
each batch in parallel, paces map requests across all queries, and has a deadline. To
continue a result after the end user's choices, use `drive_resume` or
`AsyncHost::resume` (see [Reroute](reroute.md)).

`LlmMode::ToolCall` is the mode the bindings use for Anthropic; see
[Output modes](llm-step.md#output-modes) for the others. Pin the model name in
anything you ship: a provider may retire it.

A fuller command-line example, with EV and search settings, is
`crates/waylingo-reqwest/examples/plan.rs`.

## Next

* [Bindings](bindings.md): the full API, rerouting after the end user's choices, and
  async versus blocking calls.
* [Errors](errors.md): what to do with each `error_code`.
* [The fine-tuned parser](fine-tuning.md): running without an LLM key.
