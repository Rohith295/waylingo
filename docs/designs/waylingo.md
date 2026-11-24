# Waylingo: Natural-Language Stop Planning Along Routes

*   **Author:** Rohith295
*   **Status:** Proposed
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

Waylingo is an open-source Rust routing engine. An app sends it one request: a natural-language query such as "drive to Berlin, find a 150 kW charger every 200 km, and a McDonald's 15 minutes before we arrive", plus context such as the current location and departure time. Waylingo returns one GeoJSON response with the route and the chosen stops.

It does this in three stages:

1. An LLM turns the query text into a structured plan. This is the only thing the LLM does.
2. Ordinary program code places each stop request on the route, at a time, a distance, a fraction of the route, or every N km.
3. The same code searches for places near each point and ranks them by real detour time, using one map provider: HERE, Google Maps Platform or TomTom, chosen by the developer.

The developer supplies their own LLM credentials (any OpenAI-compatible endpoint) and map provider credentials. Version 0.1 ships as a Rust crate. Its public API is designed so that iOS and Android bindings can be generated later with UniFFI without rewriting it.

## The Problem

Map provider APIs work with coordinates. None of them understand the way people describe stops on a trip:

*   "40 minutes into the drive"
*   "15 minutes before we arrive"
*   "a charger every 200 km"
*   "halfway there"

To answer one such request today, a developer has to write all of this themselves:

1.  Turn the user's words into exact parameters.
2.  Get a route and work out where along it "40 minutes in" falls. That needs the time and distance at each point of the route, and each provider reports these in a different format.
3.  Search for places near that point, but along the road, not in a circle around it. A circle search finds a fuel station 4 km away in a village that costs a 15-minute detour.
4.  Rank the results by how much time each one adds to the trip.
5.  Convert three different geometry formats (HERE Flexible Polyline, Google encoded polyline, TomTom coordinate objects) into one.

Letting an LLM agent do these steps by calling map tools itself does not work. LLMs cannot do route-slicing math or detour ranking reliably. It also sends provider data into the LLM, which HERE's terms forbid (Platform Terms §3c(vii)) and Google's terms allow only under conditions.

Google's MCP tools (Maps Grounding Lite) and the providers' own along-route search endpoints search a corridor along a route. None of them can place a stop at "40 minutes in" or "every 200 km". That anchor math, plus detour ranking, is the gap Waylingo fills.

**Who has this problem:**

*   Developers of AI agents and assistants that need to answer trip questions.
*   Mobile app developers building travel, fleet or field-service apps.
*   Developers of phone-based in-car assistants (projected to the car screen), where the driver speaks rather than types.

## Goals

1.  **One request, one GeoJSON response.** Given a query and context, Waylingo returns a single RFC 7946 `FeatureCollection`:
    *   the route as a `LineString`;
    *   each found stop as a `Point`;
    *   each unfound stop request as a feature with `geometry: null` and a reason.
    
    It never asks follow-up questions. Ambiguity is resolved by documented code rules and listed in `assumptions`.
2.  **Anchor types for v0.1:**
    *   **offset:** before or after the origin, destination or a waypoint, by time or distance;
    *   **every N:** frequency marks at N, 2N, 3N… measured from the start, each searched within a window;
    *   **fraction:** a share of the route, such as halfway.
3.  **Correct placement.** Each anchor is placed using the provider's own time and distance data along the route, not by spreading time evenly over the line. On the reference trip (Amsterdam → Berlin), a "40 minutes in" anchor lands within 1 minute of the provider's own travel time to that point.
4.  **Detour ranking.** Candidates are ranked by added travel time, measured with the same provider that made the route:
    *   TomTom returns detour time directly;
    *   Google is computed from routing summaries;
    *   HERE uses one Matrix call per stop request.
5.  **Three providers behind one interface.** HERE, Google and TomTom adapters, each passing a shared conformance test suite. Each app uses exactly one provider: no mixing, no fallback. v0.1 can ship once the core plus at least one adapter pass end to end.
6.  **Any LLM.** One OpenAI-format adapter with a configurable base URL, covering OpenAI, local servers (Ollama, vLLM, LM Studio), and other providers' OpenAI-compatible endpoints. The LLM is required: Waylingo is a natural-language engine, not a routing SDK, so the only public entry point takes a query.
7.  **Provider data never reaches an LLM.** This is enforced in code by four mechanisms:
    *   call order: parsing finishes before the first map call;
    *   a type barrier: the LLM interface accepts only the user's text;
    *   module boundaries checked in CI;
    *   a canary test that fails if any provider marker string appears in an LLM request.
8.  **Bounded cost.** The reference trip uses at most about 12 map calls. Callers can set a call budget and a deadline. The response reports the calls used.
9.  **Ready for bindings.** The public Rust API uses only shapes UniFFI accepts, and a CI job runs the binding generator from the first commit.
10. **Usable in under an hour.** A new developer goes from the README to a correct stop plan for the reference trip in under an hour, with one provider key. A mock provider lets the test suite run with no keys at all.

## Non-Goals

*   **Chat or multi-turn conversation.** One request in, one response out.
*   **Mixing providers in one request, or falling back to another provider.** Never planned.
*   **Sending provider results to any AI.** This includes an output summary meant for a calling LLM agent; there is none.
*   **Browser or WASM builds.**
*   **Published Swift, Kotlin, Node or Python packages in v0.1.** iOS and Android bindings come next.
*   **Battery-aware EV range planning.** "Every N km" is spacing, not range planning, and the docs say so. The vehicle fields are reserved so this can be added later.
*   **Fuzzy time phrases** ("around lunchtime") and **city-boundary anchors** ("when we enter Berlin") in v0.1. "Before Berlin" means before the destination point.
*   **Rerouting after each chosen stop.** Stops are placed on the base route, and one final route call gives the totals.
*   **Live GPS tracking, sessions, or caching** of provider results.
*   **Built-in in-car (head unit) integration.** HERE's terms forbid it without a separate agreement; phone projection is fine.
*   **Turn-by-turn guidance, map rendering, or a hosted service.**
*   **Guaranteeing that a developer's use complies with their provider's terms.** The docs explain the known restrictions. Each developer's agreement with their provider is theirs.

## Open Questions

*   **Search window for offset anchors:** the working value is ±10% of the offset, at least 2 km or 2 minutes. It needs testing on real routes.
*   **Dwell time:** v0.1 does not shift later anchors by earlier stops' detour and dwell time (see Technical Plan, step 7). Confirm that this is acceptable for v0.1.
*   **Window size for "every N":** ±15% is the working value. The known risk: the real gap between stops can reach N + 2 × window.
*   **Default call budget and deadline**, and what happens when they run out (D8).
*   **Tested LLM list:** which models are tested, and the minimum parser pass rate for a model to be listed (D4).
*   **TomTom terms:** confirm on the live page that results may be shown on other maps and that there is no AI clause. Only an archived copy was readable.
*   **Google display rules:** how the output and docs present Google's ban on non-Google maps outside the EEA, and the stricter EEA rule for place details.
*   **License** for the project, and fixture policy: synthetic only, or private recordings. HERE's terms forbid committing its data to an open-source repo.

## Critical User Journeys (CUJs)

### CUJ 1: Plan stops on a road trip

*   **Given**:
    *   an app configured with TomTom and an OpenAI-compatible LLM endpoint;
    *   current location Amsterdam (52.37, 4.90);
    *   departure time now, Europe/Amsterdam;
    *   vehicle EV with CCS2 connectors.
*   **When**: the app sends "Drive to Berlin, find a charger of 150 kW or more every 200 km, and a McDonald's about 15 minutes before we arrive".
*   **Then**: the response is a `FeatureCollection` with:
    *   status `complete`;
    *   one route `LineString`;
    *   charger `Point`s near the 200 km, 400 km and 600 km marks, each within its window and each with `detour_s` and an ETA;
    *   one McDonald's `Point` about 15 minutes of driving before the destination point.
    
    `usage` shows 12 or fewer map calls, and `assumptions` lists any choices made. The LLM received only the query text.

### CUJ 2: Find a stop ahead on the route already being driven

*   **Given**: the user is already navigating to Berlin. The app passes `active_route` (destination plus geometry), `current_location` and `heading`.
*   **When**: the app sends "Coffee in about 20 minutes".
*   **Then**: the engine reuses or recomputes the route from the current location toward the active destination. It returns one coffee `Point` about 20 minutes ahead in the direction of travel, never behind the car.

### CUJ 3: Reproduce a result

*   **Given**: a developer has a response that looks wrong. They have the request, the recorded LLM response, and the recorded provider responses.
*   **When**: they replay all three through the engine with the `ReplayDriver`.
*   **Then**: the output is byte-identical to the original. The compiled `plan` in the output shows exactly how the query was understood.

### CUJ 4: A stretch with nothing to find

*   **Given**: a route with no matching charger within the 400 km window.
*   **When**: the request asks for a charger every 200 km.
*   **Then**: the status is `partial`. The 400 km stop request appears as a feature with `geometry: null` and `result: NoCandidates`, including the window searched. The other stops are still returned. Nothing is invented.

### CUJ 5: Switch provider

*   **Given**: the CUJ 1 app.
*   **When**: the developer changes the configured provider from TomTom to Google and supplies a Google key.
*   **Then**:
    *   no code changes are needed;
    *   the response has the same shape, though the stops may differ;
    *   `usage` reflects Google's call pattern;
    *   the metadata carries Google's attribution and `display_terms`.

## Technical Plan (Architecture)

### Key decision: the engine never touches the network

**Chosen by author: Option A (sans-IO).** The engine is a synchronous state machine. It never opens a connection, never holds a key, and never reads a clock. It tells the host app which HTTP requests to make, and the host passes the responses back. The engine's only output is GeoJSON. What the developer does with it is up to them, within their own provider agreement.

Why this fits:

*   **Bindings.** Everything is synchronous and uses plain owned types, so UniFFI can generate Swift and Kotlin bindings without bridging async code.
*   **Credentials.** Keys stay in the app or on the developer's proxy server.
*   **Network stack.** On mobile the app uses its own network client (URLSession, OkHttp) and its own certificate pinning.
*   **Testing.** Tests replay recorded responses without any HTTP mocking, and the same responses always give the same output.
*   **Dry run.** A dry run can list every planned call before anything is sent.
*   **Cancellation.** The host simply stops calling `step`.

### Data flow

```text
 Host app (Rust driver, later Swift/Kotlin)            Waylingo engine (pure, sync)
 ───────────────────────────────────────────           ──────────────────────────────
 Request {query, current_location, ...} ──start()──▶  Phase 1: PARSE
                                                        build LLM request from ParseInput only
       ◀── NeedRequests[LLM call] ──────────────────
 run HTTP (adds LLM key) ────────────step()────────▶  validate + compile plan (≤2 repairs)
                                                        ── parse phase closes; no more LLM calls ──
                                                       Phase 2: RESOLVE PLACES (geocode if needed)
       ◀── NeedRequests[geocode...] ────────────────
 run HTTP (adds map key) ───────────step()─────────▶  Phase 3: BASE ROUTE
       ◀── NeedRequests[route] ─────────────────────
 run HTTP ──────────────────────────step()─────────▶  build MeasuredRoute; resolve anchors locally
                                                       Phase 4: SEARCH (one batch, parallel)
       ◀── NeedRequests[search × windows] ──────────
 run HTTP ──────────────────────────step()─────────▶  Phase 5: DETOUR (HERE only: matrix batch)
       ◀── NeedRequests[matrix × stops] ────────────
 run HTTP ──────────────────────────step()─────────▶  select best per stop request
                                                       Phase 6: FINAL ROUTE (stops as waypoints)
       ◀── NeedRequests[route] ─────────────────────
 run HTTP ──────────────────────────step()─────────▶  build GeoJSON
       ◀── Done { geojson } ────────────────────────
```

Every request goes through Phase 1. There is no public way to skip the LLM. Tests replay a recorded LLM response like any other response.

### Public API (binding-ready)

Pseudocode, not final code:

```text
Engine::new(config: EngineConfig) -> Engine
engine.start(request: TripRequest) -> (Session, Step)
session.step(responses: Vec<HttpResponse>) -> Step
session.dry_run() -> Vec<PlannedCall>            // calls the engine expects to make, with estimates

enum Step {
    NeedRequests { requests: Vec<HttpRequest> },  // host runs all of them, in any order or in parallel
    Done { geojson: String },                     // RFC 7946 FeatureCollection, serialized JSON
}

struct HttpRequest {
    id: String,                 // echoed back in HttpResponse
    method: String,             // "GET" | "POST"
    url: String,                // full URL with no credential
    headers: Vec<Header>,       // non-secret headers, e.g. X-Goog-FieldMask
    body: Option<Vec<u8>>,
    credential: CredentialSlot, // Llm | Map
    auth: AuthPlacement,        // QueryParam{name} | Header{name, prefix}
    timeout_ms: u32,            // hint; host enforces
    not_before_ms: u32,         // backoff hint for retries
}

struct HttpResponse { id: String, status: u16, headers: Vec<Header>, body: Vec<u8>, transport_error: Option<TransportError> }
```

Rules:

*   **Types.** Only owned types cross the public boundary: `String`, `Vec`, records, enums. No generics, lifetimes, `anyhow`, or `tokio` types.
*   **Errors.** There is one public error enum, `WaylingoError`. It is used only for misuse, such as an invalid request or a response for an unknown `id`. Provider and LLM failures are not errors: they become per-stop results inside the GeoJSON.
*   **Session.** `Session` is an opaque object holding the state between steps, and it is single-use.
*   **Rust driver.** The `waylingo-reqwest` crate provides `drive(engine, request, keys) -> geojson` for Rust users. It runs the loop with `reqwest`, adds the keys, honours `timeout_ms` and `not_before_ms`, and runs batches in parallel with a per-provider concurrency limit (5 for TomTom).

### Crate layout and the "no provider data to the LLM" rule

```text
waylingo-plan       Plan types, JSON Schema export (schemars), compiler (surface intent → canonical plan)
waylingo-parse      ParseInput, prompt building, LLM request/response handling   depends on: plan
waylingo-geo        MeasuredRoute, polyline decoders, anchor math, window slicing  depends on: (none internal)
waylingo-providers  HERE / Google / TomTom adapters (pure request builders + response parsers)
                                                                                   depends on: plan, geo
waylingo            Engine, Session state machine, ranking, GeoJSON output         depends on: all above
waylingo-reqwest    Rust driver                                                    depends on: waylingo
```

The parse/provider separation is enforced in five layers:

1.  **Crate dependencies.** `waylingo-parse` depends only on `waylingo-plan`, and `waylingo-providers` does not depend on `waylingo-parse`. CI fails if `cargo tree` shows a path between the two in either direction.
2.  **Type barrier.** The only way to build an LLM request is `waylingo_parse::build_request(&ParseInput)`. `ParseInput` holds `query`, `locale` and the saved-place names, and no provider type converts into it.
3.  **Phase order.** The session's phase is an enum that moves only forward. The LLM request builder is called only in `Phase::Parse`. A test asserts that no `CredentialSlot::Llm` request is ever emitted after a `CredentialSlot::Map` request.
4.  **Repair loop limits.** Repair messages contain only schema-validation errors about the plan the LLM wrote. They are built before any map call exists.
5.  **Canary test.** Every fixture's place names and addresses contain the marker `WAYLINGO_CANARY_<n>`. A test runs every scenario and fails if any emitted LLM request body contains `WAYLINGO_CANARY`.

### Parsing: two tiers

1.  **Surface intent.** The LLM fills a loose `SurfaceIntent` JSON. It is close to how people talk: units as written, place names as text, and placeholders such as `current_location` or `saved:home` instead of coordinates.
2.  **Compiler.** Deterministic Rust code in `waylingo-plan`:
    *   converts units to metres and seconds;
    *   fills in placeholders from the request context;
    *   checks that every reference exists;
    *   rejects impossible anchors, such as an interval longer than any plausible route, or a negative offset;
    *   outputs the canonical `Plan` with `schema_version` and a content hash.
3.  **Repairs.** If validation fails, the engine sends one repair request listing the errors (JSON Pointer, reason, allowed values). At most 2 repairs. After that the status is `failed` with `INVALID_PLAN`.

**LLM modes.** The OpenAI-format adapter has three settings, set in `LlmConfig.mode`:

*   `json_schema`: `response_format` with `json_schema`;
*   `tool_call`: a single function whose parameters are the schema;
*   `prompt_json`: schema in the prompt, JSON parsed from the reply.

### Plan schema (canonical)

```text
Plan {
  schema_version: "0.1.0"
  origin: Location                      // Coords{lat,lng} | PlaceQuery{text} | Saved{name}
  destination: Option<Location>         // None only when active_route provides it
  waypoints: Vec<Location>
  departure: { time: RFC3339, timezone: IANA }
  vehicle: { mode: Car, ev: Option<{ connector_types, min_power_kw }> }
  avoid: Vec<Tolls | Highways | Ferries>
  max_detour_s: u32                     // default 600
  stop_requests: Vec<StopRequest>
}
StopRequest {
  id: String
  what: { category: Category | text: String, brand: Option<String>, ev_filters: Option<...> }
  anchor: Anchor
  top_n: u8                             // default 1, max 5
}
Anchor =
  | Offset   { from: Origin | Destination | Waypoint(u8), direction: After | Before, distance_m | duration_s }
  | Every    { interval_m | interval_s, window_pct: u8 }    // marks at N, 2N, 3N... from the start
  | Fraction { value: 0.0..1.0 }
Category = a fixed Waylingo list (ev_charging, fuel, restaurant, fast_food, coffee, rest_area, hotel, parking,
           supermarket, pharmacy). Each adapter maps it to provider category IDs; TomTom IDs are fetched at
           runtime from its categories endpoint.
```

### Routes and anchors

**MeasuredRoute.** Each adapter converts its route response into a `MeasuredRoute`, in `waylingo-geo`:

```text
MeasuredRoute { coords: Vec<[lon, lat]>, cum_m: Vec<f64>, cum_s: Vec<f64>, total_m, total_s }
```

*   **HERE:** decode the Flexible Polyline per section. Map each `spans` entry (`length`, `duration`) onto its polyline offset range.
*   **Google:** request `ENCODED_POLYLINE` (search along route needs it). Use the step `distanceMeters` and `staticDuration`, then scale the step times so they add up to the leg's traffic-aware `duration`.
*   **TomTom v1:** use `legs[].points`. Use instruction `routeOffsetInMeters` and `travelTimeInSeconds` at their point indexes, and interpolate between instructions.
*   **All providers:** distance between points is haversine, scaled so the total matches the provider's reported length.

**Anchor resolution** is local, with no API calls. It is a binary search on `cum_s` or `cum_m`, then interpolation:

*   **Offset after X:** X's position plus the offset.
*   **Offset before X:** X's position minus the offset.
*   **Fraction:** `total × value`.
*   **Every N:** positions `k × N` for k = 1, 2, … while `k × N < total`. A mark is skipped when the distance or time left after it is smaller than its window half-width, because a stop that close to the destination is not useful.

**Windows.**

*   **Every N:** `±window_pct` of N, default 15.
*   **Offset:** ±10% of the offset, at least 2 km or 2 minutes.
*   **Fraction:** ±5% of the route.

A window is a slice of the route. It is simplified (Douglas–Peucker) to at most 250 points before it goes to the provider, which keeps HERE's GET URL within its guideline.

### Search, detour and selection

**Search.** There is one search request per window, all in one batch. Each uses the provider's along-route search with the category or brand, plus EV filters where supported:

*   **TomTom:** Search v2 `searchAlongRoute`, with `maxDetourTime = max_detour_s`, `sortBy=detourTime` and `limit=20`.
*   **Google:** Text Search (New) with `searchAlongRouteParameters`, `routingParameters.origin` set to the window's start point, `routingSummaries` in the field mask, and `evOptions` when it is an EV request.
*   **HERE:** Browse with `route=<window flexpolyline>;w=2000`, `ranking=excursionDistance`, and `evStation[minPower]` when it is an EV request.

**Detour.**

*   **TomTom:** `detourTime` from the response.
*   **Google:** `routingSummaries.legs[0].duration + legs[1].duration − (window start → window end duration on the base route)`.
*   **HERE:** take the top 5 by `excursionDistance`. Send one Matrix call per stop request: origins A (window start), destinations B (window end), candidates as both. Then `detour = t(A→P) + t(P→B) − t(A→B)`.

**Selection.** For each stop request:

1.  Drop candidates whose detour is more than `max_detour_s`, or that fall outside the window.
2.  Rank by detour time, then by distance from the anchor point.
3.  Keep `top_n`.

The top pick becomes the stop. The others are listed in the stop's properties as `alternatives`.

**No anchor shifting in v0.1.** Anchors are placed on the base route and are not moved by earlier stops' detour or dwell time. The final route call (Phase 6) reports the true ETAs. This keeps search to one parallel batch. It is listed as an open question.

### Budget, timeouts and retries

*   **Budget.** `limits.max_calls` (default 20) and `limits.deadline_ms` (default 20,000). The engine counts every request it emits. If the next phase would exceed the budget, it skips that phase's remaining stop requests (`SkippedBudget`) and goes to output. The deadline is checked using timestamps the host passes with each `step`, because the engine reads no clock.
*   **Retries.** If a response has status 429, 5xx, or a transport error, the engine re-emits that request once or twice, with `not_before_ms` set from `Retry-After` or a jittered backoff. Other errors become per-stop results.
*   **Timeouts.** `timeout_ms` defaults to 8,000 per request, as a hint the host enforces.

### Output

A single RFC 7946 `FeatureCollection`:

*   **Features:**
    *   one route `LineString` with `total_m` and `total_s` (detours included);
    *   one `Point` per found stop, with `stop_request_id`, `occurrence_index`, `name`, `provider_place_id`, `detour_s`, `eta`, `anchor` and `alternatives`;
    *   one feature with `geometry: null` per unfound stop request, with `result` and `window`.
*   **Foreign members:** `status`, `plan`, `assumptions`, `warnings`, `usage` (calls by kind), `provider`, `attribution`, `display_terms`, `schema_version` and `request_id`.
*   **Determinism.** Serialization uses a fixed key order and rounds coordinates to 6 decimals. With the same plan and the same responses, the output is byte-identical.

### Configuration

```text
EngineConfig {
  provider: Here | Google | TomTom
  provider_options: { base_url_override: Option<String>, here_auth: ApiKey | OAuthBearer, ... }
  llm: { base_url: String, model: String, mode: JsonSchema | ToolCall | PromptJson, max_repairs: u8 }
  limits: { max_calls, deadline_ms, per_request_timeout_ms }
  defaults: { max_detour_s, every_window_pct, top_n }
}
```

No keys appear in the config. Keys are added by the host at the moment it sends each request.

### Testing

1.  **Unit and golden tests, on every PR:**
    *   polyline decoders checked against the providers' official test vectors;
    *   anchor math;
    *   window slicing;
    *   the plan compiler;
    *   ranking;
    *   GeoJSON snapshots.
2.  **Replay tests, on every PR.** The CUJ scenarios run against recorded or synthetic responses for each adapter, using a `ReplayDriver`. A missing fixture fails the test; it never makes a live call. HERE fixtures are synthetic only, because its terms forbid putting HERE data in an open-source repo.
3.  **Conformance suite.** The same scenarios run for every adapter. An adapter passes, or returns `Unsupported` exactly where its capability table says it should.
4.  **Live checks, manual or nightly.** These run against real APIs with capped call counts and dedicated keys. They are never run on PRs from forks. They check response shapes, not exact values.
5.  **Parser evaluation, separate from the core tests.** 60–100 phrases with their expected canonical plans, scored for each model on the tested-models list.
6.  **Rule tests.** The canary test, the phase-order test, and the crate dependency check.
7.  **Binding check.** `uniffi-bindgen` runs on the public API in CI from the first commit.

## Alternatives Considered

### Alternative A: Engine calls a transport supplied by the app (network Option B)

*   **Pros**: Straight-line core code that is easy to read. Apps can still use their own network client through a UniFFI callback interface.
*   **Cons**:
    *   Engine calls block a thread while waiting on the network, so mobile apps must call from a background thread.
    *   Cancellation needs a separate flag.
    *   It depends on UniFFI callback interfaces working well under concurrent requests, which is not yet tested.
*   **Reason for Rejection**: Rejected by author: the sans-IO design (Option A) gives the same benefits, plus free replay tests, dry runs and cancellation, without callbacks crossing the language boundary.

### Alternative B: Engine makes its own HTTP calls with async Rust (network Option C)

*   **Pros**: Fastest to build, and simplest for Rust users.
*   **Cons**:
    *   `tokio` and `reqwest` end up inside every mobile app, which makes the binary bigger.
    *   Apps cannot use their own network stack or certificate pinning, only a base-URL proxy.
    *   Async Rust maps poorly to Swift and Kotlin.
    *   Tests need a mock HTTP server.
*   **Reason for Rejection**: Rejected by author: it conflicts with the goal of generating iOS and Android bindings without a rewrite.

### Alternative C: Parser only, with no engine

*   **Pros**: Small scope, and no map provider integration to maintain.
*   **Cons**: The parser alone is a prompt plus a schema that any developer can write. The plan it produces needs an engine to run it.
*   **Reason for Rejection**: Rejected by author: the anchor math and detour ranking are the parts no existing tool provides.

### Alternative D: Let an LLM agent call map tools directly

*   **Pros**: No engine to build. Existing MCP tools, such as Google Maps Grounding Lite, already exist.
*   **Cons**:
    *   LLMs cannot reliably slice routes or rank by detour.
    *   Provider results would flow into the LLM, which HERE's terms forbid (§3c(vii)) and Google's terms allow only under conditions.
*   **Reason for Rejection**: Rejected by author: unreliable results, and it breaks the rule that provider data never reaches an AI.

### Alternative E: LLM optional (separate parser package, or a public `run(plan)` that skips it)

*   **Pros**: Callers that already run an LLM avoid a second LLM call. The engine could also be used as a plain routing library.
*   **Cons**: Waylingo would become a routing SDK on top of the providers, which is not the product.
*   **Reason for Rejection**: Rejected by author: the LLM is required. The only public entry point takes a natural-language query. The accepted cost is that agent developers with their own LLM pay for a second LLM call.

### Alternative F: Mixing providers in one request (for example HERE route + TomTom chargers + Google places)

*   **Pros**: Each task could use the provider that is strongest at it.
*   **Cons**:
    *   It risks breaking provider terms; for example, Google forbids using its content with non-Google routing outside the EEA.
    *   Detour numbers would subtract times from different traffic models, so they mean nothing.
    *   It needs several keys per app.
*   **Reason for Rejection**: Rejected by author: one app uses one provider, chosen at setup.

### Alternative G: One provider first, others later

*   **Pros**: Something working end to end sooner. Less fixture and key management at the start.
*   **Cons**: The common interface would be shaped around the first vendor, and adapters added later might force breaking changes.
*   **Reason for Rejection**: Rejected by author: all three adapters are built in parallel, against an interface designed after reading all three providers' docs. The cost is limited by the shared conformance suite and by the rule that one passing adapter is enough to release.

### Alternative H: Browser / WASM build

*   **Pros**: Would reach TypeScript developers and web apps directly.
*   **Cons**:
    *   Keys in a browser are public.
    *   `tokio` and `std::time::Instant` do not work in WASM.
    *   It means one more package to release and maintain.
*   **Reason for Rejection**: Rejected by author: the target platforms are Rust, then iOS and Android.

### Alternative I: Measure "every N km" from the previous chosen stop

*   **Pros**: The real gap between stops never exceeds about N, which matters for EV range.
*   **Cons**: Stops can only be searched one at a time, because each window depends on the previous pick. That means more round trips and more latency. The results also depend on which stop was picked earlier.
*   **Reason for Rejection**: Rejected by author: "every N" is frequency-based, measured from the start. The docs state the worst-case gap (N + 2 × window).

### Alternative J: Rerouting after every chosen stop

*   **Pros**: Later anchors account for earlier detours and charging time.
*   **Cons**: Cost grows with every stop. Searches must run one after another. Results depend on the order of the stops.
*   **Reason for Rejection**: Rejected for v0.1, as the reviewers recommended: anchors are placed on the base route, and one final route call gives the true arrival times. Anchor shifting is an open question.

### Alternative K: Plain-coordinate maths for anchors (time spread evenly, or flat-plane interpolation)

*   **Pros**: Simple, and needs no provider timing data.
*   **Cons**: Highway and city speeds differ, so "40 minutes in" lands in the wrong place. Flat-plane maths on longitude and latitude also distorts distances at European latitudes.
*   **Reason for Rejection**: Rejected: anchors use the provider's own segment times and distances, stored in a `MeasuredRoute`.

### Alternative L: Radius search around a point, or ranking by straight-line distance

*   **Pros**: Every provider supports it, and it is simple.
*   **Cons**: It finds places that look near but cost a long detour off the highway.
*   **Reason for Rejection**: Rejected: Waylingo searches a slice of the route and ranks by detour time.

### Alternative M: A summary of results for a calling LLM agent

*   **Pros**: Fewer tokens for an agent than the full GeoJSON.
*   **Cons**: Its only reader would be an AI, and it would carry provider data into it.
*   **Reason for Rejection**: Rejected by author: provider data never reaches an AI. The output is GeoJSON for the app.

### Alternative N: Existing tools instead of Waylingo

*   **Google Maps Grounding Lite, or provider along-route search on its own:** these search a corridor, but cannot place "40 minutes in" or "every 200 km" anchors.
*   **OSRM, Valhalla, openrouteservice:** good open-source routers, but their place and EV charger data in Europe is weak.
*   **Reason for Rejection**: Rejected: none of them provide anchor placement plus detour ranking. A routing adapter for these routers could be a future addition.

## Non-Functional Requirements

*   **Security:**
    *   The engine never holds a key: requests carry a credential slot, and the host adds the key.
    *   Errors and logs never include URLs with `key=` parameters.
    *   The docs recommend a proxy for all LLM calls on mobile, and for Google on mobile.
*   **Privacy:**
    *   The LLM receives only the query text, the locale, and the names of saved places. It never receives coordinates or provider results.
    *   Location data goes only to the developer's chosen map provider.
*   **Performance:** engine CPU time is small next to network time. Target: under 5 ms of engine work for the reference trip on a laptop, excluding JSON parsing of provider responses. Call counts are bounded by the budget.
*   **Compatibility and rollout:**
    *   Releases follow 0.x semver, and every release is labelled unstable.
    *   The `Plan` and output schemas carry `schema_version`. Minor versions only add optional fields. An unknown major version is rejected with `INVALID_PLAN`.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

**Language and toolchain:** Rust (edition 2024, stable toolchain). **License:** MIT OR Apache-2.0, the Rust ecosystem convention.

**Crate layout:** a Cargo workspace under `crates/`. One extra crate is added beyond the Technical Plan: `waylingo-http`. It holds the request and response types, so that both `waylingo-parse` and `waylingo-providers` can use them without depending on each other.

**Build order (milestones):**

*   **M1: `waylingo-http`, `waylingo-geo` and `waylingo-plan`.** Pure maths and types, with no network code.
*   **M2: `waylingo-parse`.** Prompt building and the OpenAI-format adapter.
*   **M3: `waylingo-providers`.** The TomTom adapter first, then Google and HERE in parallel.
*   **M4: `waylingo`.** The engine, session state machine, selection and GeoJSON output.
*   **M5: `waylingo-reqwest`, the conformance suite, CI and the README.**

### Target Files to Create/Modify

#### 1. `Cargo.toml` (Create)

*   **Type / Scope**: Workspace manifest.
*   **Proposed Changes**:
    *   Lists the members under `crates/*`.
    *   Shared `[workspace.dependencies]`: `serde`, `serde_json`, `thiserror`, `flexpolyline`, `geojson`, `schemars`, `jsonschema`, `sha2`, `insta` (dev), `proptest` (dev).
    *   Shared lints: `unsafe_code = "forbid"`, plus clippy pedantic as warnings.

#### 2. `crates/waylingo-http/src/lib.rs` (Create)

*   **Type / Scope**: Rust library with types only.
*   **Proposed Changes**: `HttpRequest`, `HttpResponse`, `Header`, `CredentialSlot { Llm, Map }`, `AuthPlacement { QueryParam{name}, Header{name, prefix} }` and `TransportError`. These are the request and response types from the public API. There are no secrets fields.

#### 3. `crates/waylingo-geo/src/` (Create)

*   **Type / Scope**: Rust library for route geometry and anchor maths.
*   **Proposed Changes**:
    *   `lib.rs`: module exports.
    *   `latlng.rs`: `LatLng { lat, lng }`, plus haversine distance.
    *   `polyline.rs`: decoders.
        *   Google encoded polyline (precision 5).
        *   HERE Flexible Polyline (wrapping the `flexpolyline` crate).
        *   Both return `Vec<LatLng>`.
    *   `measured.rs`: `MeasuredRoute { coords, cum_m, cum_s, total_m, total_s }`.
        *   Constructors from coordinates plus timing checkpoints `(point_index, cum_m, cum_s)`, which every adapter produces.
        *   Haversine distances are scaled so they match the provider's total.
        *   Time between checkpoints is interpolated by distance.
        *   `position_at_distance(m)` and `position_at_time(s)` use binary search plus interpolation.
    *   `anchor.rs`: resolves offset, fraction and every-N anchors to `AnchorPoint { cum_m, cum_s, point }`, following the rules in the Technical Plan. The last-10% rule applies to every-N.
    *   `window.rs`: `Window { start_m, end_m }` per anchor kind, clamped to the route. It cuts a sub-line with interpolated end points.
    *   `simplify.rs`: Douglas–Peucker simplification down to a maximum number of points (the default is 250). It uses a tolerance search, and always keeps the first and last point.

#### 4. `crates/waylingo-plan/src/` (Create)

*   **Type / Scope**: Rust library for the plan types and the compiler.
*   **Proposed Changes**:
    *   `surface.rs`: `SurfaceIntent`, the loose structure the LLM fills. It keeps units and place text as written, and uses placeholders.
    *   `plan.rs`: the canonical `Plan`, `StopRequest`, `Anchor`, `Category` (the fixed list of 10), `Location`, `Vehicle` and `Avoid` types. All units are metres and seconds. It carries `schema_version`.
    *   `compile.rs`: `compile(surface, &TripContext) -> Result<Plan, Vec<PlanError>>`.
        *   Converts units: km, m, mi, min and h.
        *   Resolves placeholders: `current_location` and `saved:<name>`.
        *   Applies defaults.
        *   Validates the plan.
        *   Each `PlanError` carries a JSON Pointer, a reason and the allowed values, which feed the repair loop.
    *   `schema.rs`: exports the JSON Schema of `SurfaceIntent`, which goes to the LLM, using `schemars`. It also computes the plan's content hash (SHA-256 of canonical JSON).
    *   `context.rs`: `TripContext`, holding the request fields from the contract (current location, heading, departure, active route, vehicle, preferences, saved places, locale, region bias and limits).

#### 5. `crates/waylingo-parse/src/` (Create)

*   **Type / Scope**: Rust library for the LLM step. It depends only on `waylingo-plan` and `waylingo-http`.
*   **Proposed Changes**:
    *   `input.rs`: `ParseInput { query, locale, saved_place_names }`. This is the only input type.
    *   `prompt.rs`: the system prompt, which holds the intent rules and few-shot examples, and the repair prompt, which lists the `PlanError`s.
    *   `openai.rs`:
        *   Builds a chat-completions `HttpRequest` for each `LlmMode` (`json_schema`, `tool_call`, `prompt_json`).
        *   Parses the reply into a `SurfaceIntent`.
        *   Its credential slot is `Llm`, with a `Bearer` header.

#### 6. `crates/waylingo-providers/src/` (Create)

*   **Type / Scope**: Rust library of pure request builders and response parsers. It depends on `waylingo-plan`, `waylingo-geo` and `waylingo-http`.
*   **Proposed Changes**:
    *   `capability.rs`:
        *   A `ProviderAdapter` trait with `geocode_request`, `parse_geocode`, `route_request`, `parse_route -> MeasuredRoute`, `search_request(window, stop)`, `parse_search -> Vec<Candidate>`, `detour_strategy()` and `matrix_request` / `parse_matrix`.
        *   A `Capabilities` table.
    *   `category.rs`: maps each Waylingo category to the matching provider category for each provider.
    *   `tomtom.rs`:
        *   Routing v1 and Search v2 `searchAlongRoute`, with native detour.
        *   Category IDs are fetched at runtime and cached for each session.
    *   `google.rs`:
        *   Routes API `computeRoutes`, with field masks.
        *   Text Search (New) with `searchAlongRouteParameters` and `routingSummaries`, and detour computed from them.
    *   `here.rs`: Routing v8 with `spans`, Browse with a `route=` corridor, and Matrix v8 with `async=false`.

#### 7. `crates/waylingo/src/` (Create)

*   **Type / Scope**: Rust library containing the engine. This is the public crate.
*   **Proposed Changes**:
    *   `engine.rs`: `Engine::new(EngineConfig)` and `start(TripRequest) -> (Session, Step)`.
    *   `session.rs`: the phase state machine (Parse → ResolvePlaces → BaseRoute → Search → Detour → FinalRoute → Done), plus `step(responses)` and `dry_run()`.
    *   `budget.rs`: call counting, the deadline (from host timestamps), and the retry and backoff logic.
    *   `select.rs`: filtering by window and maximum detour, ranking, `top_n` and alternatives.
    *   `output.rs`: the GeoJSON `FeatureCollection` with its extra top-level fields, in a fixed key order with 6-decimal rounding.
    *   `error.rs`: `WaylingoError`.

#### 8. `crates/waylingo-reqwest/src/lib.rs` (Create)

*   **Type / Scope**: Rust library, the driver for Rust users.
*   **Proposed Changes**:
    *   `drive(engine, request, Keys) -> Result<String>`.
    *   Runs each batch in parallel, with a concurrency limit for each provider.
    *   Adds keys according to `AuthPlacement`.
    *   Honours `timeout_ms` and `not_before_ms`.

#### 9. `tests/fixtures/` and `crates/waylingo/tests/` (Create)

*   **Type / Scope**: Replay fixtures and integration tests.
*   **Proposed Changes**:
    *   Synthetic fixtures for each provider, covering the CUJ scenarios, with `WAYLINGO_CANARY_<n>` markers.
    *   `conformance.rs`: every scenario runs against every adapter.
    *   `canary.rs`: the rule test.
    *   `phase_order.rs`: checks that no LLM request is emitted after a map request.
    *   `replay.rs`: checks that the output is byte-identical on replay.

#### 10. `scripts/check_deps.sh` and `.github/workflows/ci.yml` (Create)

*   **Type / Scope**: CI.
*   **Proposed Changes**:
    *   `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test --workspace`.
    *   The dependency check, which fails if `waylingo-parse` and `waylingo-providers` depend on each other in either direction.
    *   `uniffi-bindgen` on the public API, added in M4.

#### 11. `README.md`, `LICENSE-MIT`, `LICENSE-APACHE`, `.gitignore` (Create)

*   **Type / Scope**: Project documentation.
*   **Proposed Changes**: the README includes:
    *   the quickstart for the reference trip;
    *   the "provider data never reaches an AI" rule;
    *   each provider's terms restrictions (HERE AI and vehicle clauses, Google's display rules, attribution);
    *   the note that "every N" is spacing, not EV range planning.

#### 12. `docs/designs/waylingo.md` (Create)

*   **Type / Scope**: The finalized copy of this design.

### Verification Plan

*   **Compilation/Build**: `cargo build --workspace`
*   **Unit Tests**: `cargo test --workspace`
*   **Static Analysis & Linting**: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `scripts/check_deps.sh`
*   **Test Coverage Requirement**: every anchor kind, window rule, unit conversion and plan error has a test. Polyline decoders pass the providers' published examples.
*   **E2E/Integration**:
    *   Replay tests for CUJs 1–5 for each adapter, plus the canary, phase-order and replay-determinism tests.
    *   Live checks run by hand, using the developer's own keys.

--------------------------------------------------------------------------------

*Drafted with EGM*
