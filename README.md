# Waylingo

One natural-language trip request in, one GeoJSON trip out.

```text
"Drive to Berlin, a fast charger every 200 km and a McDonald's 15 minutes before we arrive"
        │
        ▼  LLM: reads only the text, writes a fixed JSON plan
        ▼  map provider (TomTom or HERE): geocode, route, search along the route, rank by detour
        ▼
GeoJSON: the route, each stop with its pick and alternatives, and "did you mean" candidates
```

The end user can then pick another place, skip a stop or choose another match for the
destination; `reroute` builds the new route without calling the LLM again.

Waylingo is a library, not a service: you bring your own keys for the map provider
and the LLM (Anthropic, OpenAI, Gemini, or any OpenAI-compatible server, including
one serving Waylingo's own fine-tuned parser model). Map data never reaches the LLM.

## Status

Preview, version 0.1.0. The Rust engine, the TomTom and HERE adapters and the
Python, Swift and Kotlin bindings work and are tested. The API may still change
before 1.0.

## Where it runs

| Where | Language | Install |
| :--- | :--- | :--- |
| Android | Kotlin | `io.github.rohith295:waylingo-android` on Maven Central |
| iOS, macOS | Swift | Swift Package `https://github.com/Rohith295/waylingo` |
| Servers, scripts | Python | `pip install waylingo` |
| Rust | Rust | crates `waylingo` and `waylingo-reqwest` on crates.io |

Examples for each: the
[quick start](https://rohith295.github.io/waylingo/quick-start.html). To try it in a
browser with your own keys, run the [demo](demo/README.md). Full docs:
**https://rohith295.github.io/waylingo/**

## Crates

| Crate | What it does |
| :--- | :--- |
| `waylingo` | The engine: query in, GeoJSON out. Sans-IO: it returns the HTTP requests to send and never does I/O itself |
| `waylingo-parse` | The LLM step: prompt, structured output, repairs. Never sees map data |
| `waylingo-plan` | The plan: schema, compilation from the LLM's JSON, validation |
| `waylingo-providers` | TomTom and HERE adapters: request builders and response parsers |
| `waylingo-geo` | Route geometry: anchors, windows, polylines |
| `waylingo-http` | The request and response types shared by the engine and hosts |
| `waylingo-reqwest` | Ready-made hosts (blocking and async) that send the engine's requests |
| `waylingo-ffi` | Python, Swift and Kotlin bindings ([guide](crates/waylingo-ffi/README.md)) |

## Quick look (Python)

```python
import waylingo as w

app = w.Waylingo(
    w.WaylingoConfig(map_provider=w.MapProvider.TOMTOM,
                     llm=w.LlmBackend.ANTHROPIC(model=None, base_url=None)),
    map_key=TOMTOM_KEY, llm_key=ANTHROPIC_KEY)

out = app.plan_blocking(w.TripInput(
    query="Drive to Utrecht, coffee halfway",
    current_location=w.LatLng(lat=52.3702, lng=4.8952)))

if out.status == w.TripStatus.FAILED:
    print(out.error_code)          # a typed code, for example NEEDS_DESTINATION
```

The [bindings guide](crates/waylingo-ffi/README.md) covers Swift, Kotlin, the
interaction loop, every error code, and keeping keys out of shipped apps.

## Contributing

Waylingo is maintained by one person, so pull requests aren't accepted. Bug reports,
questions and ideas are very welcome in issues and discussions.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option.
