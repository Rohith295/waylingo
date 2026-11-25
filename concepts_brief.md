# Waylingo: Route-Anchored Stop Planning for AI Agents

* **Author:** rohithrangaraju
* **Date:** 2026-11-24
* **Status:** Brainstorm brief (pre-RFC). Source use case: `~/Downloads/usecase.txt`.

## Overview

Waylingo is an open-source Rust routing engine, with packages for iOS (Swift) and Android/JVM (Kotlin/Java), plus server-side packages for agent developers. It does not target the browser (no WASM). The engine calls both the LLM and the map provider. It answers trip requests such as:

> "Navigate from Amsterdam to Berlin, find a fast EV charger every 200 km, and stop at a McDonald's about 15 minutes before we arrive."

It works in three stages:

1. **Intent parser.** An LLM turns the plain-language request into a structured trip plan: a route, plus "stop requests" anchored by time or distance from the origin, the destination, a waypoint, or "every N km".
2. **Orchestrator.** It runs the plan against a map provider (Google Maps Platform, HERE or TomTom):
   - gets the base route;
   - finds each anchor point along the route;
   - searches for places near that point along the route;
   - ranks the candidates by real detour time.
3. **Output.** A GeoJSON FeatureCollection, with the route as LineStrings and the stops as Points, for the developer's app. Provider results never go to any LLM (see "Hard Rule" below).

Developers plug in their own map provider and LLM credentials in the apps they build. The library handles one request at a time from a fixed start point and does not track a moving car.

**Target users:** developers building AI agents (MCP, LangChain and similar), fleet and logistics apps, and voice-first car or travel assistants.

**Core value:** map APIs search by coordinates. None of them understand "40 minutes into the drive", "15 minutes before the destination" or "every 200 km along my route". That anchor math, plus ranking by detour, is the part no existing tool provides.

## Request and Response Contract

The engine is **not a chat interface**. It takes one request and returns one GeoJSON response. It never asks the user a follow-up question. When something is ambiguous, it makes a documented choice or reports the problem in the response, and the host app decides what to do.

### Request

| Field | Required | Purpose |
| :--- | :--- | :--- |
| `query` | Yes | The natural-language request, for example "charger every 200 km, McDonald's 15 min before Berlin". |
| `current_location` | Yes, unless the query names an origin | `{lat, lng}`, plus optional `accuracy_m` and `heading_deg`. Heading tells the engine which way the car faces, which matters for "ahead of me" on a highway. |
| `departure_time` + `timezone` | No (default: now, in the timezone of `current_location`) | Needed for duration anchors, traffic, opening hours at arrival, and later "around lunchtime". |
| `active_route` | No | The route the user is already driving: destination plus optional geometry. Without it, "coffee 20 minutes ahead on my route" has no route to work on, because the engine is stateless. |
| `vehicle` | No (default: car) | Travel mode, plus EV details: `connector_types`, `min_power_kw`. Truck size and weight come later. |
| `preferences` | No | `avoid` (tolls, highways, ferries) and `max_detour_s`. |
| `saved_places` | No | Host-supplied names such as `home` and `work` with coordinates, so "drive home" resolves without geocoding. |
| `locale` | No (default: `en`, metric) | Language of the query and of returned names; km or miles in display fields. |
| `region_bias` | No | Country or bounding box used to rank geocoding results. Stops "Paris" from resolving to Paris, Texas. |
| `limits` | No | `top_n` candidates per stop, `max_calls`, `deadline_ms`. |
| `request_id` | No | Echoed in the response and logs for tracing. |

**Privacy rule:** the LLM receives only `query`, `locale` and the names of `saved_places`, never coordinates. It writes placeholders such as `current_location` or `saved:home`, and the engine fills in real coordinates after parsing. This keeps the user's location away from the LLM vendor and makes the plan easier to cache and test.

### Response

A single GeoJSON `FeatureCollection` (RFC 7946). Engine metadata goes in top-level "foreign members", which RFC 7946 allows:

- `status`: `complete`, `partial` or `failed`.
- `plan`: the canonical plan the engine ran, so the host can show or log what the request was understood to mean.
- `assumptions`: the choices the engine made where the query was ambiguous, for example "‘before Berlin’ read as 15 min before the destination point".
- `warnings`, `usage` (map and LLM calls made), `attribution` (required provider text), `schema_version`, `request_id`.
- There is no summary for an LLM agent (see "Hard Rule" below).

Features:

- **Route:** one `LineString` with total distance and duration (including detours) in `properties`.
- **Stops:** one `Point` per chosen stop. Its properties hold `stop_request_id`, `occurrence_index`, name, provider ID, `detour_s`, ETA, and anchor position.
- **Unresolved stop requests:** a Feature with `geometry: null` (allowed by RFC 7946) and a `result` property such as `NoCandidates` or `Unsupported`. This keeps every stop request visible in one place, found or not.
- **Ambiguous places** (for example two "Springfield"s): the engine picks the top match by a code rule using `current_location` and `region_bias`, records the pick in `assumptions`, and lists the other candidates as properties. It does not stop to ask, and it never sends the candidates to the LLM.

## Hard Rule: Provider Data Never Reaches an LLM

**Author decision:** no AI system ever receives results from HERE, Google or TomTom. The LLM only reads the end user's text. Ordinary program code makes every map call and handles every result. This applies to all three providers, not just HERE.

The engine enforces this in five ways, so it is not left to discipline:

1. **One-way order.** Parsing finishes before the first map call. Once any provider call has been made, the engine makes no further LLM calls for that request. Two consequences:
   - The repair loop runs only on schema-validation errors about the plan itself, never on provider responses.
   - Ambiguity found later, such as two matching "Springfield"s from geocoding, is resolved by code rules and reported in `assumptions`. It is never sent back to the LLM.
2. **Type barrier.** The `LlmClient` interface accepts one input type, `ParseInput`, with only `query`, `locale` and the *names* of `saved_places`. No provider response type can be converted into it, so passing provider data to the LLM would not compile.
3. **Separate modules.** The parser module does not depend on the provider adapter modules, and the provider modules do not depend on the parser. Only the orchestrator sees both, and it passes the parser only a `ParseInput`. A CI check fails the build if these dependencies change.
4. **Canary test.** Every recorded or synthetic provider fixture includes a unique marker string in its place names and addresses. A test runs every scenario and fails if any request sent to the LLM contains a marker.
5. **No agent summary.** The earlier idea of a short `summary` for a calling LLM agent is dropped. Even engine-derived numbers come from provider data, and the summary's only purpose was to be read by an AI. The output is GeoJSON for the developer's app.

**What the library cannot control:** once the developer's app has the GeoJSON, nothing stops the developer from sending it to their own LLM. The README and each adapter's docs say clearly that doing so may break their provider agreement: HERE §3c(vii) forbids it outright, and Google allows it only under conditions.

## Analysis

Seven independent reviewers looked at the concept. They agreed on one conclusion: **the part worth building is small and mostly pure computation.** It consists of:

- placing a time or distance anchor on a route;
- searching near that anchor;
- ranking by detour time measured with one provider;
- returning GeoJSON that reports failures honestly.

Most of what is around that core either adds risk or can be added later. The main findings follow, most severe first. Claims about provider APIs and terms come from the reviewers' general knowledge and are marked *(verify)* where they must be checked against current vendor documentation.

### 1. Where network calls happen decides almost everything (Blocker)

WASM is dropped (author decision), which removes the browser-specific problems (tokio and `std::time::Instant` not working there). The core problem remains for Swift and Kotlin: if the Rust core makes its own HTTP calls, it needs an async runtime, and Rust async code does not cross into Swift or Kotlin cleanly. Proxy, header, retry and cancellation logic would also end up written again for every platform. For server-side use only (a plain Rust crate, or Node/Python bindings), a built-in tokio + reqwest client is workable.

**Recommendation: a core that makes no network calls itself ("sans-IO").** The core is a synchronous state machine:

- `start(plan)` returns either a list of HTTP requests to make or a finished result.
- The host app makes those requests with its own HTTP client and passes the responses to `step(responses)`.
- Each request carries a *credential slot name* (for example `"google"`), never the key itself, along with a cost estimate.
- The platform driver is about 50 lines per platform: URLSession, OkHttp, or reqwest.

This one choice also gives you, without extra work:

- proxying through the developer's server;
- safe credential handling;
- cancellation (the host stops calling `step`);
- offline replay tests;
- a dry run that lists every planned API call and its cost before anything is spent.

One reviewer argued for a lighter start: pure functions plus a single `HttpTransport` trait, moving to the full state machine only when mobile bindings arrive. Either way, the shape of this interface must be fixed before any bindings are generated, because changing it later breaks every package.

### 2. Credentials: the library must never be where a secret lives (Blocker)

The author has decided that developers pass their own credentials, as with the Google Maps and Mapbox SDKs. That decision stands. The library has to make it safe:

- **Exposed keys.** Keys inside mobile apps can be extracted. Server-side use avoids this.
- **Google keys:** the iOS and Android restriction headers (`X-Ios-Bundle-Identifier`, `X-Android-Package` / `X-Android-Cert`) are documented in general, but not confirmed for the Routes and Places (New) APIs. Google recommends IP restrictions or a proxy for these APIs, so Google on mobile means proxy only.
- **HERE and TomTom keys** can only be restricted by domain, not by mobile app. HERE also supports OAuth 2.0 tokens (up to 24 hours) minted by the developer's backend, which is the recommended path for HERE on mobile.
- **LLM keys** cannot be restricted to an app at all.
- **Production setup.** The documented default for production is: map calls and all LLM calls go through the developer's own server, using a base-URL override or the host's own transport. Keys placed directly in the app are labelled "prototype only".
- **Handling rules.** Keys are never logged. `key=` query parameters (used by TomTom and HERE) are removed from error messages and traces.

### 3. One provider per app, and the terms that still apply (Major)

**Author decision:** providers are never mixed. The developer picks one provider (HERE, Google or TomTom) when they set up the library, and every call (routing, search, detour) goes to that provider. This removes the biggest terms-of-service risk and keeps detour numbers consistent, because every time comes from one traffic model. The use case's multi-provider example and the `default_router` / `default_search` / fallback config must be removed from the spec.

What still needs handling (checked against each provider's terms on 2026-11-24; details and sources are in [provider_capabilities.md](provider_capabilities.md)):

- **HERE Platform Terms §3c** (read directly from here.com, effective 14 Nov 2026) list things a customer may not do "except as otherwise permitted in your Subscription Plan or a separate written agreement with HERE". Three affect this project:
  - **(vii) AI:** "Use HERE Materials in connection with a machine learning or artificial intelligence ('AI') system", including natural-language processing and generative AI. **Design answer:** the LLM only parses the end user's text. Ordinary program code calls HERE, and HERE results never go to any LLM. This is the strongest position available under this wording, and the engine enforces it for all providers (see "Hard Rule: Provider Data Never Reaches an LLM"). "In connection with" is broad wording, so the docs tell developers their HERE use falls under their own HERE agreement. Asking HERE to confirm is optional, not a blocker.
  - **(i) Vehicles:** HERE Materials may not be integrated into a vehicle system, including in-car display screens. Phone projection onto the car screen is expressly allowed. The "in-car assistant" persona therefore means phone-projected apps (such as CarPlay or Android Auto style) for HERE, unless the developer has a separate HERE agreement.
  - **(iii) Open source:** HERE Materials may not be exposed to open-source projects or databases. Recorded HERE responses must never be committed to the public repo. HERE test fixtures are synthetic, or kept private.
- **Google bans showing its content on non-Google maps outside the EEA** (ToS §3.2.3(e), Service Specific Terms §15.2 and §20.2). Inside the EEA, the route line and points may go on any map, but place details (names, ratings, EV info) may not be shown on any map. Caching: `place_id` indefinitely, coordinates for at most 30 days, nothing else. LLM use at request time is allowed only if the model does not train on the input. The library cannot enforce these rules, so the Google output carries a `display_terms` note and the docs spell the rules out.
- **TomTom** allows its results on other companies' maps and has no AI clause (based on an archived copy of its terms; the live page could not be read). Caching is limited to the HTTP cache headers, with no shared cache.
- Every output feature carries its provider, source ID and the required attribution text.
- **The same request behaves differently per provider.** Each provider has different features:
  - TomTom returns detour time natively; HERE needs a matrix call.
  - Google has no truck mode.
  - EV filters differ between providers.
  
  Developers switching providers will see different stops, different call counts and some `Unsupported` results. Publish a support matrix per provider, and make the result report which strategy was used (section 10).
- **Switching providers is a config change, not a code change.** That is the promise of "provider-agnostic". It means one schema and one output format, not identical results.

### 4. The LLM parser lives inside the engine, behind a plan boundary

**Author decision:** the engine calls the LLM itself. It takes plain text, turns it into a plan, and runs the plan. The reviewers had recommended a separate package. Their concerns still apply, so the design has to answer them:

- **Duplicate LLM call.** The caller is often already an LLM agent, and a second LLM inside the engine adds cost and non-determinism. **Answer:** expose two entry points, `plan_and_run(text)` and `run(plan)`. Agents that already have an LLM call `run(plan)` and skip the built-in one.
- **Key exposure.** An LLM key on a phone cannot be protected (see section 2). **Answer:** LLM calls go through the same transport and base-URL override as map calls, and the mobile docs say to proxy them.
- **Determinism.** The use case calls the plan "deterministic", but an LLM produces it. Only the step from plan to API calls can be deterministic. **Answer:** say exactly that in the docs.
- **Testability.** Rules such as "halfway" meaning 50% sit in a prompt in the use case. Rules in a prompt cannot be tested. **Answer:** move those rules into Rust code (the two-tier design below).
- **Scope.** The author wants any LLM to work: Claude, OpenAI, Gemini, or local models behind an OpenAI-compatible server (Ollama, vLLM, LM Studio and similar). **Answer:** a small `LlmClient` interface, plus one adapter for the OpenAI chat-completions format in v0.1, with a configurable base URL. OpenAI, local servers, and the OpenAI-compatible endpoints offered by Gemini and Anthropic all speak this format *(verify each one's limits)*. Native Claude and Gemini adapters come later, and only if the compatibility endpoints fall short.
- **Uneven structured output.** Not every model or server supports JSON-schema output or tool calling, and small local models often return plans that don't validate. **Answer:** the adapter declares which mode it uses: strict JSON schema, tool calling, or "JSON in the prompt" as the last resort. Every reply goes through schema validation and the repair loop. Publish a list of tested models with their pass rate on the parser test set, instead of promising that every model works.

**Design:**

- The plan is **Plan JSON that validates against a published, versioned JSON Schema**, generated from the Rust types. The built-in parser and outside agents both produce it.
- **Two-tier parsing.** The LLM fills a loose "surface intent". A deterministic Rust compiler then turns it into the canonical plan. It converts units to metres and seconds, checks references and rejects impossible anchors. Validation errors go back to the LLM as repair hints, and it retries a limited number of times (for example 2).

### 5. The plan schema needs fixing before code (Blocker)

The two examples in the use case disagree with each other:

- `origin` is an object in one and a string in the other;
- `avoid` sits in different places;
- some fields (`intent`, `resolution_strategy`, `mode`) disappear in the second example;
- category names follow different styles (`fuel` vs `ev_charging_station`).

The schema also has gaps:

- `at_percentage` is not in the relation list;
- there is no `departure_time`, which "lunchtime", traffic and "open when I arrive" all need;
- there is no version field;
- `return_top_n` has no `n`.

The examples are also not valid JSON, because they contain `//` comments.

**Recommendation:**

- **Typed anchors.** An anchor is exactly one of these shapes, so invalid combinations cannot be written:
  - `Offset {from: origin | destination | waypoint(i), direction, distance_m or duration_s}`;
  - `Every {interval_m or interval_s, window_pct}`;
  - `Fraction {0..1}`.
- **Units and time.** Store metres and seconds only; human units exist only in the parser. Require `departure_time` with a timezone.
- **Locations.** The host resolves "current location" before calling, and passes coordinates.
- **Versioning.** Add a required `schema_version`. Minor versions only add optional fields.
- **Examples.** Every example in the docs must validate against the schema in CI.

### 6. Anchors need time and distance along the route, not just a line

A line on its own cannot answer "40 minutes in", because highway and city speeds differ. Plain-coordinate interpolation in the `geo` crate also gives wrong distances at European latitudes.

**Recommendation:**

- Each provider adapter produces a **measured route**: coordinates plus cumulative distance and duration at each point.
- Fill these from the provider's own segment data: HERE `spans`, Google per-step durations, TomTom guidance offsets.
- Finding an anchor is then a binary search on that route, with no extra API calls.
- GeoJSON is only the export format.
- Reuse existing crates: `polyline` for Google, HERE's official `flexpolyline`, and `geojson`.

### 7. What the anchors mean must be defined

- **"Before Berlin".** "15 minutes before we enter Berlin" (the city boundary) and "15 minutes before the destination point" (Berlin Mitte) can be 20+ minutes apart. v0.1 should use the destination point and drop the "enter Berlin" claim.
- **"Every 200 km".** The use case doesn't say whether the 200 km is measured from the origin or from the previous stop, so it must be defined. The reviewers recommend measuring from the previous chosen stop, with a ±15% search window.
- **Rerouting.** The use case's "reroute after every chosen stop" gets more expensive with each stop and makes results depend on the order of the stops. Instead:
  - resolve all stops against the base route;
  - shift later time anchors forward by each earlier detour plus dwell time;
  - make one final route call for totals and arrival times.
- **Opening hours** should be checked at arrival time, not at request time.

### 8. Detour ranking can get expensive

Ranking every candidate with its own route call, plus rerouting after each stop, comes to about 31 paid map calls for the Amsterdam to Berlin example. A staged approach brings this to about 12:

1. Use detour data the provider already returns (confirmed in each provider's docs):
   - TomTom `searchAlongRoute` returns `detourTime` and `detourDistance` directly;
   - Google Text Search with `searchAlongRouteParameters` + `routingSummaries` returns origin→place and place→destination legs, so detour = leg0 + leg1 − base duration. These calls are billed at Google's $40 per 1,000 SKU after the free cap;
   - HERE Browse returns only `excursionDistance` (a distance), so HERE always needs step 3.
2. Pre-filter locally by distance from the route, and keep the top 5–8.
3. Make one matrix call per stop request: detour = time(A→stop) + time(stop→B) − time(A→B), where A and B are route points just before and after the stop.

Also:

- **Search a window, not the whole route.** Always search a window around the anchor. Providers cap results at about 20, so a whole-route search clusters or cuts off results.
- **Call budget.** Let callers set a budget (maximum calls, deadline). Report the calls actually used, and mark results as `estimated` or `exact` when the budget runs out.

### 9. Partial results and errors must be explicit (Blocker)

The use case only defines `"status": "success"`. If the result is all-or-nothing, one rural stretch with no charger fails the whole trip. If failures are silent, the calling LLM invents stops to fill the gaps.

**Recommendation:**

- **Top-level status:** `complete`, `partial` or `failed`. It is `failed` only when the plan is invalid or the base route fails.
- **Per-stop result:** `Found`, `NoCandidates` (with the window searched), `ProviderError` (with whether it can be retried), `SkippedBudget` or `Unsupported`. There is no "needs clarification" state, because the engine never asks follow-up questions. Ambiguity is resolved by a documented rule and reported in `assumptions`.
- **Error codes:** a fixed list (`waylingo::ErrorCode`), for example `PROVIDER_AUTH`, `PROVIDER_QUOTA_EXCEEDED`, `PROVIDER_RATE_LIMITED`, `NO_ROUTE`; the full table is in `crates/waylingo-ffi/README.md`.
- **Defaults:** 8-second timeout per call, at most 2 retries, and retries only on 429, 5xx or a dropped connection.

### 10. A capability-based provider interface, not one big trait

The providers differ in ways a single interface would hide:

- TomTom has native detour data and HERE does not;
- Google Routes has no truck mode;
- waypoint limits differ;
- the providers name their "avoid" options differently.

**Recommendation:**

- Split the interface into capabilities: `Geocoder`, `Router`, `CorridorSearch`, `MatrixRouter`, `EvChargerSearch`.
- The orchestrator uses a provider's native feature when it has one and a documented fallback otherwise, and the output reports which strategy was used.
- A constraint a provider can't support returns `Unsupported`. It is never silently dropped.
- "Provider-agnostic" means one schema and one geometry format, not identical results from every provider.

### 11. EV: spacing, not range planning

"A charger every 200 km" stands in for range planning, which HERE (`ev[makeReachable]`) and TomTom (Long Distance EV Routing, v1 only) already do natively from battery and consumption data. Google has no documented EV range routing.

EV charger search works on all three, with power filters in the request: HERE `evStation[minPower]`, Google `evOptions.minimumChargingRateKw`, TomTom `minPowerKW` on `searchAlongRoute`. TomTom's dedicated EV Search API is sales-only, so the TomTom adapter uses along-route search with the EV category instead. Category IDs come from TomTom's categories endpoint at runtime, not hard-coded.

**Recommendation:**

- In v0.1, EV is a place category with filters (`min_power_kw`, `connector_types`) on an interval anchor, and the docs say plainly that this is spacing, not range planning.
- Battery-aware planning is a non-goal. Reserve the vehicle fields now so it can be added later without breaking the schema.

### 12. Output contract

- **Coordinate order.** The use case's example output uses `[lat, lon]` while GeoJSON uses `[lon, lat]`. Use GeoJSON order inside geometry, and named `{lat, lng}` objects in the request and in any properties.
- **Totals.** The example totals don't add up: 654 vs 661 km, 380 vs 398 min. Document whether totals include detours.
- **One source.** Build the GeoJSON from one internal result model, with stable IDs.
- **Untrusted text.** Place names and reviews from providers flow straight into the calling LLM, which is a prompt-injection path. Cap and clean the strings, and mark them as untrusted in the docs.
- **Agent summary dropped.** The reviewers backed a summary-plus-GeoJSON split. The author has since dropped the summary, because its only reader would be an AI, and provider data never goes to an AI.

### 13. Testing without burning free quota

Free tiers are fine for trying the library out, but not for CI. Use four tiers:

1. **Pure unit tests on every PR:** polyline decoding checked against the official test vectors, anchor math and ranking.
2. **Replay tests on every PR:** recorded responses are played back. A missing recording fails the test; it never falls back to a live call.
3. **Live contract tests, nightly or by hand only:** a capped number of calls, never run on PRs from forks, dedicated keys with quota limits set.
4. **Separate parser evaluation:** 60–100 example phrases with their expected plans.

Only commit fixtures that are synthetic or scrubbed. Raw provider responses in a public repo may break redistribution terms and may leak keys.

### 14. Packaging cost

- **Separate releases.** Each platform package (crates.io, npm or PyPI, Maven Central, Swift Package) is its own release pipeline.
- **Irreversible publishing.** Maven Central releases can never be deleted, and npm only allows unpublishing within 72 hours.
- **Public API.** The code generated by UniFFI and wasm-bindgen becomes your public API.

Use one tag per release, the same version number in every package, publishing to staging first, and pinned binding-generator versions.

### Inventive contradictions and how they resolve

| Trade-off accepted as unavoidable | Resolution |
| :--- | :--- |
| Many platforms vs a small maintainer team | The core makes no network calls, so each platform needs only a tiny HTTP driver |
| Keys inside the library for convenience vs security | Requests carry credential slot names; the host or its server fills them in |
| LLM flexibility vs a reproducible plan | The built-in LLM proposes, a Rust compiler decides; `run(plan)` skips the LLM entirely; the same plan plus the same responses gives identical output |
| Exact detour ranking vs API cost | Native detour data, then a free local filter, then one matrix call per stop request, under a declared budget |
| Correct stop order vs speed | Resolve stops on the base route, shift later anchors, one final route call |
| Best provider per task vs terms and consistent numbers | Author decision: one provider per app, chosen at setup; no mixing |
| Realistic tests vs quota | Replay recorded responses; live checks on a schedule only |

**Ideal Final Result** (reviewers' version; the author has since chosen to keep the LLM in the engine, so read "runs no LLM" as "the LLM is behind the same transport and can be skipped"). The core holds no keys, makes no network calls, keeps no state, runs no LLM and has no cache. It takes a validated plan, emits batches of API requests with cost estimates, takes back the responses, and returns GeoJSON that reports partial results honestly. Each provider contributes its native strengths. The same plan with the same responses always gives the same output, CI uses no quota, and the secure setup is the default path.

## Goals vs. Non-Goals

* **In-Scope Goals (v0.1):**
  * A published, versioned Plan JSON Schema and Output Schema. The Plan schema is also exported as an LLM tool definition, and every doc example validates in CI.
  * A Rust core that makes no network calls, or at minimum puts all network access behind one injected transport (decision D2). The core contains:
    * the measured-route anchor engine;
    * `before`/`after` anchors from the origin or destination, `fraction` anchors, and `every N` anchors, by distance or duration;
    * detour ranking using native detour data or a matrix call;
    * a call budget;
    * the partial-result and error contract.
  * Three provider adapters built in parallel: HERE, Google and TomTom (author decision). Each app still uses only one of them.
    * **Interface first.** Write the common capability interface (`Geocoder`, `Router`, `CorridorSearch`, `MatrixRouter`, `EvChargerSearch`) only after reading all three providers' docs, so it isn't shaped around one vendor.
    * **Capability matrix.** A published per-provider support table, generated from each adapter's declared capabilities, so the docs can't drift from the code.
    * **Shared conformance suite.** The same scenarios (Amsterdam → Berlin, an "every N" trip, an empty rural stretch, a bad key, a rate-limit response) run against every adapter's recorded fixtures. An adapter is done when it passes the suite or returns `Unsupported` where the matrix says it should.
    * **Release gate.** v0.1 ships when the core plus at least one adapter pass end to end. An adapter that isn't ready is marked experimental rather than holding the release.
  * The built-in intent parser: `plan_and_run(text)`, with one OpenAI-format LLM adapter (configurable base URL, so it works with OpenAI, local servers, and other providers' OpenAI-compatible endpoints), the two-tier compiler, and repair retries. Plus `run(plan)` for agents that bring their own plan.
  * The native Rust crate only (author decision). Its public API follows the binding-ready rules below, so the later iOS and Android bindings are generated, not rewritten.
  * **Binding-ready public API rules** (UniFFI, the usual Rust-to-Swift/Kotlin generator, accepts only a limited set of shapes):
    * no generics, lifetimes or borrowed types in public functions;
    * owned types only (`String`, `Vec`, records, enums);
    * one error enum, with no `anyhow` or boxed errors crossing the boundary;
    * HTTP and LLM access behind an interface the host can implement (Swift and Kotlin will supply their own network clients);
    * no tokio types in the public API, and synchronous calls (or the step-machine shape from section 1), with async added by the wrapper on each platform.
  * A CI job that runs `uniffi-bindgen` on the public API from day one, even before bindings ship, so a change that breaks binding generation fails the build.
  * A replay test for Amsterdam → Berlin that passes offline: a 150 kW+ charger about every 200 km, plus McDonald's about 15 minutes before the destination point. A mock provider so the library runs with no keys.
  * Credential guidance: proxy or base-URL override as the production default, Google app-restriction headers, a "prototype only" label on direct keys, and a note on terms and attribution.
  * Success test: a new developer goes from the README to a correct stop plan in under an hour with one provider key. Each stop is inside its anchor window and its detour limit, or comes back as `NoCandidates`. The reference trip uses at most about 12 map calls.
* **Out-of-Scope Non-Goals (v0.1):**
  * WASM and browser use (author decision).
  * Native (non-OpenAI-format) Claude and Gemini adapters in v0.1.
  * Mixing providers or falling back from one provider to another. Never planned: one app uses one provider.
  * Published Swift, Kotlin, Node or Python packages. iOS and Android bindings come next, generated with UniFFI. Server packages (napi-rs, PyO3) are undecided.
  * Battery-aware EV range planning.
  * The plan dependency graph, and rerouting after each stop.
  * Fuzzy phrases ("around lunchtime", "near the border") and city-boundary anchors ("when we enter Berlin").
  * Walk, bike and truck modes.
  * The YAML config file. Configuration is a typed object passed in code.
  * Live GPS tracking, sessions and caching.
  * Ratings and "open now" filters.
  * Turn-by-turn guidance, map rendering, and a hosted service.

## Alternatives Weighed

* **Parser as a separate optional package (reviewer recommendation).** Rejected by author: the engine calls the LLM itself. The reviewers' concerns (a second LLM call, LLM keys on devices, non-determinism) are handled by a `run(plan)` entry point that skips the LLM, routing LLM calls through the same transport, and the two-tier compiler.
* **Parser only, with no engine.** Rejected: the parser alone is a prompt plus a schema that any developer can write. The anchor math and detour ranking are the parts nothing else provides.
* **WASM / browser package.** Rejected by author. This removes browser key exposure and the tokio-in-WASM problem. The cost is that TypeScript agent developers need a Node (napi-rs) package instead.
* **Mixing providers (for example HERE route + TomTom chargers + Google places), and falling back to another provider.** Rejected by author: one app uses one provider for everything. This also avoids breaking provider terms and avoids detour numbers that subtract times from different traffic models.
* **Core library making its own HTTP calls with tokio and reqwest.** Rejected: it does not run in WASM, does not cross into Swift, Kotlin or JS cleanly, and forces proxy and retry logic to be written again for each platform.
* **Spreading time evenly along plain GeoJSON, or using flat-plane interpolation.** Rejected: both give wrong anchor positions. Use the provider's own segment times and distances.
* **Searching a straight-line radius around a point, or ranking by straight-line distance.** Rejected: this is the off-highway detour trap the use case itself describes.
* **Searching the whole route in one call.** Rejected: providers cap results at about 20, so results cluster or get cut off.
* **Rerouting after every chosen stop.** Rejected: cost grows with each stop and results depend on stop order, all to correct a drift of a few minutes.
* **One provider first, others later (reviewer recommendation, TomTom favoured).** Rejected by author: all three adapters are built in parallel against one common interface designed from all three providers' docs. Benefit: the interface isn't shaped around one vendor. Costs: about three times the adapter work and fixtures before anything ships, three sets of keys and quotas for testing, and Google's terms and billing setup in scope from day one. The conformance suite and the "one adapter passing is enough to release" gate limit these costs.
* **One large `Provider` trait covering every vendor.** Rejected: it reduces every provider to the features they all share.
* **YAML config with `${ENV}` keys.** Rejected: a server pattern that WASM and mobile apps cannot use.
* **Committing raw recorded provider responses to the public repo.** Rejected: risk under provider terms and risk of leaking keys.
* **Plain agent tool-calling, or existing map MCP servers.** Not a replacement: an LLM cannot reliably do route-slicing math or detour ranking. The reference Google Maps MCP server was archived *(verify)*. Wrapping the core as an MCP server later is cheap and worth planning.
* **Open-source routers (OSRM, Valhalla, openrouteservice).** Not a replacement for v0.1: their place and EV charger data in Europe is weak.
* **Provider along-route search on its own (Google or TomTom).** Not a replacement: it searches a corridor but cannot place an anchor such as "40 minutes in".
* **Gemini "Grounding with Google Maps".** A possible competitor for the language-to-places step. Check whether it supports along-route search *(verify)*.
* **Python or TypeScript as the core language.** Rejected by author in favour of Rust, to reach mobile and web from one core. Reviewers note this choice depends on decision D1. If the first users are TypeScript or Python agent developers, Rust's benefit only arrives once the mobile packages ship.
* **Proxy-only design with no direct keys.** Rejected by author: developers pass their own credentials in their apps. Kept as the documented production setup, with direct keys labelled "prototype only".

## Key Decisions & Staging for Next Steps

**Settled by the author:**

* Library, not a hosted service. Open source.
* Rust core, packaged for iOS and Android/JVM, plus server-side packages. No WASM or browser target.
* Build order: the complete Rust engine first (v0.1 ships as a Rust crate), then generated iOS and Android bindings.
* It is a routing engine that calls both the LLM (built-in intent parser) and the map provider.
* Any LLM can be plugged in: Claude, OpenAI, Gemini, or local models behind an OpenAI-compatible server.
* Developers pass their own map provider and LLM credentials in the apps they build.
* One provider per app. The developer picks HERE, Google or TomTom at setup, and every call goes to that provider. No mixing, no fallback to another provider.
* All three provider adapters are built in parallel, behind one common interface designed after reading all three providers' docs.
* Handles one request at a time from a fixed start point. No live tracking.
* Not a chat interface: one request (natural-language query plus context such as current location) in, one GeoJSON response out. No follow-up questions.
* Output is GeoJSON (route LineStrings + stop Points) for the developer's app. No summary for an LLM agent.
* Hard rule: provider results never reach any LLM. The LLM only parses the user's text, all parsing finishes before the first map call, and this is enforced by types, module boundaries and a canary test.
* Provider free tiers are used for trying the library out and for manual testing.

**Open decisions to settle before `/egm-draft`:**

* **D1. First users of the Rust-only v0.1.** Settled: v0.1 is the Rust crate, then iOS and Android bindings. Still open: until the bindings ship, the only users are Rust developers, a small part of the agent-developer audience. Decide whether that is acceptable for v0.1, or whether a server package (napi-rs or PyO3) should come before mobile to reach TypeScript and Python agent developers.
* **D2. Network boundary.** The full sans-IO state machine now, or pure functions plus one `HttpTransport` trait for now. This must be fixed before any bindings are generated.
* **D3. Providers.** Settled: HERE, Google and TomTom adapters built in parallel behind one capability interface. The research settled the detour fallbacks: TomTom native, Google computed from routing summaries, HERE via Matrix. It also settled the API versions: TomTom v1 Routing + Search v2 (Orbis lacks truck and EV), Google Routes + Places (New), HERE Routing v8 + Search v7. Still open:
  * **HERE's AI clause:** handled by design (the LLM never sees HERE results). Optional: ask HERE to confirm that an app where an LLM only parses the user's text is fine on the standard plan.
  * **Google display rules:** how the docs and output present them, given the non-Google-map ban outside the EEA.
* **D4. LLM support.** Settled: any LLM, through a pluggable `LlmClient`. v0.1 ships one OpenAI-format adapter with a configurable base URL. Still open: which models go on the tested list, and the minimum pass rate a model needs to be listed.
* **D5. Anchor rules.**
  * Settled by author: "every N" is frequency-based and measured from the starting point (current location or origin). Marks sit at N, 2N, 3N… along the route, each searched within a window (for example ±15%). Known trade-off: if a stop is picked early in one window and late in the next, the real gap between stops can reach about N + 2 × window (290 km for N = 200 km at ±15%). For EV trips that gap could exceed the car's range, so the docs must say this is spacing, not range planning.
  * Settled by author: there are several intent types, each its own anchor shape: offset (before/after the origin, destination or a waypoint), every N (frequency from the start), and fraction ("halfway"). Others can be added as new tagged variants.
  * Still open: the window size.
  * Does the detour include time spent at the stop?
  * What charging dwell time is assumed?
  * Will city-boundary anchors ever be supported?
* **D6. Terms for each provider on its own.** Research done (see [provider_capabilities.md](provider_capabilities.md)). Still open: confirm TomTom's terms on the live page (only an archived copy was readable), and decide whether to get a legal opinion on HERE §3c(vii) and on Google's EEA terms before the first release.
* **D7. Success metric.** Adopt the "README to correct plan in under an hour, at most about 12 map calls" test, or define another.
* **D8. Default call budget** and behaviour when it runs out.
* **D9. Coordinate format at the agent boundary:** named `{lat, lng}` (recommended) or `[lon, lat]` everywhere.
* **D10. Project license and fixture policy:** synthetic fixtures only, or private recordings.

**Before writing code:**

1. Read the three providers' terms on caching, attribution, display on other companies' maps, and sending data to an LLM.
2. Write the Plan and Output JSON Schemas, and fix the inconsistencies listed in section 5.
3. Write the quickstart README first, as the test for the developer experience.
4. Done 2026-11-24, in [provider_capabilities.md](provider_capabilities.md): research all three providers' official docs and fill in the capability matrix:
   * routing response shape and segment timing;
   * along-route search and native detour;
   * matrix routing;
   * EV charger search and filters;
   * geocoding;
   * auth and app restrictions;
   * rate limits and free tier;
   * polyline format;
   * terms on caching, attribution and display.
   
   Then run the Amsterdam → Berlin request against all three and record the responses as fixtures for the conformance suite.
