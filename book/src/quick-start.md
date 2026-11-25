# Quick start

Two minimal programs that plan one trip: one in Rust, one in Python. Both need a
TomTom key and an Anthropic key in the environment. No package is published yet, so
both build Waylingo from the repository.

The output contains map provider content (place names, addresses). Show it to the end
user; do not send it to an LLM.

## Rust

`Cargo.toml`:

```toml
[dependencies]
waylingo = { git = "https://github.com/Rohith295/waylingo" }
waylingo-reqwest = { git = "https://github.com/Rohith295/waylingo" }
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

## Python

Build and install the bindings from a checkout (needs Rust):

```bash
cd crates/waylingo-ffi
pip install maturin
maturin develop --release
```

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
In async code use `await app.plan(...)`. The full API, rerouting, and Swift and Kotlin
are in [Bindings](bindings.md); what to do with each `error_code` is in
[Errors](errors.md).
