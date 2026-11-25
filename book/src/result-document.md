# The result document

Every `plan` and `reroute` returns one document: an RFC 7946 GeoJSON
`FeatureCollection` with extra top-level members (`TripResult` in
`crates/waylingo/src/result.rs`, written by `crates/waylingo/src/output.rs`). An app can
draw it on any GeoJSON map, read its fields, and pass it back to `reroute`. Positions
are `[lng, lat]`, as GeoJSON requires.

## A sample

A trimmed, hand-written sample for "Drive to Dornholm, coffee halfway". The place names,
ids and numbers are invented for illustration, not taken from a provider; the route
line is shortened to three points and the hash to a placeholder. The field names, their
order and the number formats follow the code.

```json
{
  "type": "FeatureCollection",
  "result_schema": "1.0.0",
  "plan_schema": "0.1.0",
  "request_id": "req-1",
  "status": "complete",
  "error": null,
  "provider": "tomtom",
  "attribution": "© TomTom",
  "plan": {
    "schema_version": "0.1.0",
    "origin": {"kind": "coords", "lat": 52.37, "lng": 4.90, "label": "current_location"},
    "destination": {"kind": "place_query", "text": "Dornholm"},
    "waypoints": [],
    "departure": {"time": null, "timezone": null},
    "vehicle": {"ev": null},
    "avoid": [],
    "max_detour_s": 600,
    "stop_requests": [
      {"id": "stop_0",
       "what": {"category": "coffee", "text": null, "brand": null, "ev": null},
       "anchor": {"kind": "fraction", "value": 0.5},
       "top_n": 3}
    ]
  },
  "plan_hash": "<64 hex characters>",
  "assumptions": [
    "\"Dornholm\" matched 2 places; used the provider's top match (Dornholm, Example Province); the others are in the location's candidates"
  ],
  "warnings": [],
  "usage": {"llm_calls": 1, "map_calls": 5},
  "search": {
    "anchor_radius_m": 5000, "search_span_m": 15000, "search_limit": 20,
    "search_limit_requested": 20, "strict_radius": false, "max_detour_s": 600,
    "every_window_pct": 15, "top_n": 3
  },
  "features": [
    {"type": "Feature",
     "geometry": {"type": "LineString",
                  "coordinates": [[4.90, 52.37], [5.00, 52.21], [5.12, 52.09]]},
     "properties": {"kind": "route", "total_m": 42600.0, "total_s": 2100.0, "includes_stops": true}},
    {"type": "Feature",
     "geometry": {"type": "Point", "coordinates": [4.90, 52.37]},
     "properties": {"kind": "origin", "index": null, "text": "current_location",
                    "chosen_id": null, "along_m": 0.0, "candidates": []}},
    {"type": "Feature",
     "geometry": {"type": "Point", "coordinates": [5.12, 52.09]},
     "properties": {"kind": "destination", "index": null, "text": "Dornholm",
                    "chosen_id": "example-geo-1", "along_m": 42000.0,
                    "candidates": [
                      {"id": "example-geo-1", "label": "Dornholm, Example Province",
                       "position": [5.12, 52.09], "score": 0.98},
                      {"id": "example-geo-2", "label": "Dornholm, Other Province",
                       "position": [6.50, 53.11], "score": 0.71}]}},
    {"type": "Feature",
     "geometry": {"type": "Point", "coordinates": [5.00, 52.21]},
     "properties": {
       "kind": "stop", "stop_request_id": "stop_0", "occurrence": 0,
       "anchor": {"position": [5.00, 52.21], "cum_m": 21000.0, "cum_s": 990.0},
       "window": {"start_m": 6000.0, "end_m": 36000.0},
       "pick": {"provider_place_id": "example-place-1", "name": "Cafe Example",
                "brand": null, "address": "Dornholm",
                "position": [5.00, 52.21], "detour_s": 95.0, "detour_m": 800.0,
                "along_m": 21200.0, "distance_from_anchor_m": 230.0},
       "eta_s": 1010.0,
       "beyond_radius": false,
       "alternatives": [
         {"provider_place_id": "example-place-2", "name": "Example Roasters",
          "brand": null, "address": null, "position": [4.99, 52.23],
          "detour_s": 180.0, "detour_m": 1500.0, "along_m": 19100.0,
          "distance_from_anchor_m": 2240.0}],
       "user_choice": null}}
  ],
  "signature": null
}
```

## Top-level members

| Member | Content |
| :--- | :--- |
| `type` | `"FeatureCollection"` |
| `result_schema` | `"1.0.0"`; a result with a different major version is not resumed |
| `plan_schema` | `"0.1.0"`, the version of the `Plan` format |
| `request_id` | echoed from the request |
| `status` | `complete`, `partial` or `failed` |
| `error` | `{ code, message }` when failed, else null (see [Errors](errors.md)) |
| `provider` | `tomtom` or `here` |
| `attribution` | text that must be shown with the results |
| `plan`, `plan_hash` | the compiled plan and the SHA-256 of its JSON (see [The plan](plan.md)) |
| `assumptions` | choices made for the user (below) |
| `warnings` | degraded steps: budget, missing detour times, final route failed, a pick beyond the radius, ... |
| `usage` | `{ llm_calls, map_calls }` for this call only; a reroute starts from zero |
| `search` | the search settings in effect, so a reroute uses the same ones |
| `features` | see below |
| `signature` | hex HMAC-SHA256 when the engine has a signing key, else null (see [Signing](reroute.md#signing)) |

`assumptions` lists choices Waylingo made where the request was ambiguous. Today that is
the geocoding case: when a place name matched more than one candidate, the provider's
top match is used, the assumption says so, and the other matches are in that
location's `candidates` for a "did you mean ...?" prompt.

## Features

Features come in a fixed order: the route, the origin, the waypoints, the destination,
then the stops in plan order. `properties.kind` says what a feature is.

| Kind | Geometry | Properties |
| :--- | :--- | :--- |
| `route` | LineString | `total_m`, `total_s`, `includes_stops` (false when it is the base route because no stop was found or the final route failed) |
| `origin`, `waypoint`, `destination` | Point, or null if unresolved | `index` (waypoints only), `text` as the user said it or the label of given coordinates, `chosen_id`, `along_m`, `candidates` (up to 5: `id`, `label`, `position`, `score`) |
| `stop` | Point (the pick) | `stop_request_id`, `occurrence`, `anchor`, `window`, `pick`, `eta_s`, `beyond_radius`, `alternatives`, `user_choice` |
| `unresolved_stop` | null | `stop_request_id`, `occurrence`, `anchor`, `window`, `result`, `error_code` (only for `PROVIDER_ERROR`), `detail`, `options` |

A place (`pick`, an alternative, or an option of a skipped stop) has
`provider_place_id`, `name`, `brand`, `address`, `position`, `detour_s`, `detour_m`,
`along_m` (distance along the base route to the route point nearest the place) and
`distance_from_anchor_m` (straight line). Places are map provider content: show them,
never send them to an LLM.

`anchor` is where the stop was asked for (`position`, `cum_m`, `cum_s` on the base
route); `window` is the stretch of route that was searched (`start_m`, `end_m`).
`eta_s` is seconds from departure. `user_choice` is `chosen` when the end user picked
the place, `dropped` when an earlier choice was not found again after a re-search.

## Number formats

Output is deterministic: the same inputs give byte-identical output. Field order is
fixed by the types; positions are rounded to 6 decimals, distances and times to whole
metres and seconds, geocoding scores to 3 decimals (`output.rs`). This is what makes
signatures stable and lets tests compare whole documents.

## In the bindings

The bindings parse the document into typed fields (`TripOutput` in
`crates/waylingo-ffi/src/result.rs`), so an app can show options and build choices
without parsing JSON, and pass `geojson` back to `reroute`. See
[Bindings](bindings.md).
