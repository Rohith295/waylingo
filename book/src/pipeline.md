# How a trip is planned

This chapter follows one request from the app to the GeoJSON result.

## The engine does no I/O

The engine is a synchronous state machine. It never opens a connection, never holds a
key and never reads a clock. Instead it tells the host which HTTP requests to send, and
continues when the host passes the responses back. This style is often called
*sans-IO*.

```rust
let (mut session, mut step) = engine.start(request)?;
loop {
    match step {
        Step::Done { geojson } => break geojson,
        Step::NeedRequests { requests } => {
            let responses = send_all(&requests); // your own function: send, add keys
            step = session.step(responses)?;
        }
    }
}
```

`send_all` is the reader's own function: it sends each request with the host's HTTP
client, adding the key the request names, and returns one response per request.
(`Engine::start`, `Session::step` and `Step` are in `crates/waylingo/src/engine/mod.rs`.)

Each `HttpRequest` carries an `id`, method, URL without any key, headers, body, a
`credential` slot (`Llm` or `Map`), an `auth` placement (a query parameter name, or a
header name and prefix such as `Authorization: Bearer `), a `timeout_ms` hint and a
`not_before_ms` backoff hint (`crates/waylingo-http/src/lib.rs`). The host echoes the
`id` in the `HttpResponse`, or reports a transport error (`Timeout`, `ConnectionFailed`,
`Cancelled`) instead of a status.

Why this design:

* **Keys stay with the host.** The engine cannot leak what it never has.
* **Bindings are simple.** Everything crossing the boundary is plain owned data, so
  UniFFI can generate Swift and Kotlin without bridging async Rust callbacks.
* **The app keeps its own network stack.** A mobile app can send the requests with its
  own client, certificate pinning or proxy.
* **Tests need no HTTP mocking.** A test answers requests from fixtures; the same
  answers always give byte-identical output (`same_inputs_give_identical_output` in
  `crates/waylingo/tests/e2e.rs`).
* **Cancellation is free.** The host stops calling `step`.

Rust users do not have to write the loop: `waylingo-reqwest` provides a blocking
`drive` and an `AsyncHost` (see [Map providers](providers.md#keys-and-proxies) and
[Routing and stops](routing-and-stops.md#call-budget-retries-and-pacing)).

### Rules for the host

`Session::step` takes the responses to the *whole* last batch. The requests in a batch
are independent and may be sent in any order or in parallel. The engine returns
`WaylingoError::UnknownResponse` for an id it did not issue,
`WaylingoError::MissingResponses` when some requests of the batch got no response, and
`WaylingoError::Finished` after the session is done. A `Session` is single use.

## The phases

```text
 Host                                       Engine (crates/waylingo/src/engine/)
 ─────────────────────────────────          ───────────────────────────────────────────
 start(query, context) ───────────────────▶ Parse: build the LLM request from the text only
        ◀── [LLM request] ──────────────────
 step(LLM response) ──────────────────────▶ compile + validate the plan
        ◀── [repair or retry] ─────────────  (up to 2 repairs, 2 retries)
                                            ── no LLM request after this point ──
        ◀── [setup?, geocode × places] ───── Prepare: category list (TomTom), geocoding
 step(...) ───────────────────────────────▶
        ◀── [base route] ─────────────────── BaseRoute: origin → waypoints → destination
 step(...) ───────────────────────────────▶ place anchors on the route (local maths)
        ◀── [search × anchors] ───────────── Search: one along-route search per anchor
 step(...) ───────────────────────────────▶
        ◀── [matrix × stops] ─────────────── Detour: HERE only, one matrix per stop
 step(...) ───────────────────────────────▶ choose one place per stop
        ◀── [final route] ────────────────── FinalRoute: through the chosen places
 step(...) ───────────────────────────────▶ build GeoJSON
        ◀── Done { geojson } ───────────────
```

The `Phase` enum (`Parse`, `Prepare`, `BaseRoute`, `Search`, `Detour`, `FinalRoute`,
`Done`) only moves forward (`Session::advance` in `crates/waylingo/src/engine/mod.rs`).
A phase with nothing to send is skipped in the same call: a destination given as
coordinates needs no geocoding, TomTom needs no matrix, and a trip without stops needs
no category list, no search and no final route.

What each phase does, with the code that does it:

| Phase | Requests | Code |
| :--- | :--- | :--- |
| Parse | 1 LLM call, plus repairs and retries | `crates/waylingo-parse/src/parser.rs` |
| Prepare | TomTom's category list (only when the plan has stops); one geocode per place given as text | `prepare_requests`, `setup_requests` in `engine/phases.rs` |
| BaseRoute | 1 route through origin, waypoints and destination | `base_route_request`, `apply_base_route` |
| Search | 1 along-route search per anchor | `search_requests` |
| Detour | HERE: 1 travel-time matrix per stop with candidates; TomTom: none | `detour_requests`, `apply_matrix` |
| FinalRoute | 1 route through the chosen places, if any were found | `choose_stops`, `route_through_stops` in `engine/stops.rs` |

For the reference request in the offline tests ("Drive to Berlin, a fast charger every
200 km and a McDonald's 15 minutes before we arrive", TomTom fixtures) the engine makes
1 LLM call and 8 map calls: category list, geocode, base route, four searches (three
charger marks and one food stop), final route (`query_in_geojson_out` in
`crates/waylingo/tests/e2e.rs`).

## What can stop a trip early

A failure in one phase sets a single failure (`Session::fail`; the first one wins) and
the next `advance` goes straight to building the output with `status: "failed"` and an
`error` object. Examples:

* the LLM step fails (`NEEDS_DESTINATION`, `LLM_AUTH`, ...): no map request is ever
  sent (`llm_failure_gives_failed_geojson_and_no_map_calls` in `tests/e2e.rs`);
* a place cannot be geocoded: `NO_GEOCODE`;
* the base route fails: the provider error's code, for example `NO_ROUTE`.

A failure that affects one stop does not fail the trip: the stop becomes an
`unresolved_stop` feature and the status is `partial`. Failures of optional calls
(TomTom's category list, a HERE matrix, the final route) become warnings. See
[Errors](errors.md).

## Output

`Session::finish` hands the collected state to `output::build`
(`crates/waylingo/src/output.rs`), which writes one RFC 7946 `FeatureCollection` with
extra top-level members (plan, usage, warnings, signature and so on). The format is
described in [The result document](result-document.md).
