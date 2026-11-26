# Introduction

Waylingo is an open-source Rust library. An app gives it one trip request in plain
language, plus what the app already knows (where the user is, saved places, the car),
and gets back one GeoJSON document with the route and the stops.

```text
"Drive to Berlin, a fast charger every 200 km and a McDonald's 15 minutes before we arrive"
        │
        ▼  LLM: reads only the text, writes a fixed JSON plan
        ▼  map provider (TomTom or HERE): geocode, route, search along the route, rank by detour
        ▼
GeoJSON: the route, each stop with its pick and alternatives, and "did you mean" candidates
```

The end user can then pick another place for a stop, skip a stop, or choose another
match for an ambiguous place name. Waylingo builds the new route from the previous
result without calling the LLM again.

Waylingo runs in **Android apps (Kotlin), iOS and macOS apps (Swift), Python and
Rust**: the engine is written in Rust, and the same API is generated for the other
three languages. [Quick start](quick-start.md) shows each one.

Status: preview, version 0.1.1. The engine, the TomTom and HERE adapters and the
Python, Swift and Kotlin bindings work and have tests. Packages are on PyPI, Maven
Central, crates.io and Swift Package Manager; the API may still change before 1.0.

## The problem

Map provider APIs work with coordinates. People describe stops relative to the trip:

* "40 minutes into the drive"
* "15 minutes before we arrive"
* "a charger every 200 km"
* "halfway there"

Answering one such request by hand means: turning the words into parameters, getting a
route, finding where "40 minutes in" falls on it (which needs time and distance at every
point of the route, reported differently by each provider), searching for places along
the road near that point, and ranking them by how much time they add to the trip.

Giving an LLM agent map tools and letting it do these steps has two problems. LLMs are
not reliable at route arithmetic and detour ranking. And it sends map provider data into
the LLM, which providers' terms may restrict. Check your own agreement; Waylingo avoids
the question by never sending provider data to the LLM.

Waylingo splits the work: the LLM only translates language into a small, fixed plan.
Ordinary, tested code does everything with coordinates.

## What it is not

* **Not a routing SDK.** There is no public entry point that takes a plan instead of a
  sentence. `Engine::start` takes a `TripRequest` whose required field is the query text
  (`crates/waylingo/src/engine/mod.rs`). If you already have coordinates and categories,
  call the map provider directly.
* **Not a service.** Waylingo is a library that runs in your app or on your server. It
  has no hosted endpoint and keeps no state between calls: everything needed to continue
  a trip is inside the result document.
* **Not a navigation product.** No turn-by-turn guidance, no map rendering, no live
  tracking.
* **Not an EV range planner.** "Every 200 km" places stops by spacing; there is no
  battery or consumption model.

## The privacy boundary

Two rules shape the whole design:

1. **Map provider data never reaches an LLM.** The LLM sees the request text, the locale
   and the *names* of saved places. It never sees coordinates, routes, place results or
   addresses returned by the provider.
2. **Developers bring their own keys.** The engine never holds a key. It hands the host
   HTTP request descriptions with a credential slot (`Llm` or `Map`); the host adds the
   key when it sends the request (`crates/waylingo-http/src/lib.rs`).

The first rule is enforced in code, not by convention:

| Mechanism | Where |
| :--- | :--- |
| The LLM input type has no field for coordinates or provider data | `ParseInput` in `crates/waylingo-parse/src/input.rs` |
| The message history can only be extended by three crate-private methods: the input, the model's reply, and validation errors | `crates/waylingo-parse/src/conversation.rs` |
| The parse crate and the provider crate never depend on each other | `scripts/check_deps.sh`, run in CI |
| Parsing finishes before the first map request | phase order in `crates/waylingo/src/engine/mod.rs` |
| A test fails if an LLM request follows a map request, or carries a canary string from a provider fixture or the user's coordinates | `provider_data_never_reaches_the_llm` in `crates/waylingo/tests/e2e.rs` |

## Who this book is for

Developers deciding whether to use Waylingo, and engineers who want to know how it
works. Each chapter explains what the code does and why, with paths to the source so
you can check. Where something is not built yet, or has a known gap, the text says so.

## Crates

| Crate | Role |
| :--- | :--- |
| `waylingo` | The engine: query in, GeoJSON out. Performs no I/O |
| `waylingo-parse` | The LLM step: prompt, structured output, repairs, retries |
| `waylingo-plan` | `SurfaceIntent` (what the LLM writes), the compiler, the canonical `Plan`, validation limits |
| `waylingo-geo` | Route geometry: measured routes, anchors, windows, polylines, simplification |
| `waylingo-providers` | TomTom and HERE adapters: pure request builders and response parsers |
| `waylingo-http` | Request and response descriptions shared by the engine and hosts |
| `waylingo-reqwest` | Ready-made hosts (blocking and async) that send the engine's requests |
| `waylingo-ffi` | Python, Swift and Kotlin bindings generated with UniFFI |

Licensed under Apache-2.0 or MIT, at your option. The project does not accept pull
requests; issues and discussions are open.
