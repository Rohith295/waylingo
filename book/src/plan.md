# The plan

There are two plan types, on purpose:

* `SurfaceIntent` is what the LLM writes. It stays close to how people talk.
* `Plan` is what the engine runs. It is exact: SI units, coordinates or geocoding text,
  checked references and bounded values.

A deterministic compiler turns one into the other (`crates/waylingo-plan/src/compile.rs`).

## Why two tiers

Asking a model for metres, seconds and coordinates invites arithmetic and invention.
Asking it to copy "200" and "km" from the sentence does not. So the LLM writes the units
and names as the user said them, and Rust code does the conversion, the lookups and the
checks. When the model gets something wrong, the compiler can say exactly what and
where, which is what the [repair loop](llm-step.md#validation-repair-and-retry) needs.

## SurfaceIntent

Defined in `crates/waylingo-plan/src/surface.rs`. Unknown fields are rejected.
Enum-like fields are plain strings, so a wrong value produces "unknown value, allowed:
..." rather than a parse failure.

```json
{
  "destination": "Berlin",
  "stops": [
    {"id": "charger",
     "what": {"category": "ev_charging", "min_power_kw": 150},
     "anchor": {"kind": "every", "amount": {"value": 200, "unit": "km"}}},
    {"id": "food",
     "what": {"brand": "McDonald's"},
     "anchor": {"kind": "offset", "from": "destination", "direction": "before",
                "amount": {"value": 15, "unit": "min"}}}
  ]
}
```

The prompt's rules for chargers: "fast charger" with no number means `min_power_kw`
150, and "rapid" means 50 (see [Prompt styles](llm-step.md#prompt-styles)).

| Field | Meaning |
| :--- | :--- |
| `error` | `"not_understood"` when the text is not a trip request |
| `origin`, `destination`, `waypoints` | place text as written, `saved:<name>`, or `current_location` |
| `avoid` | `tolls`, `highways`, `ferries` |
| `max_detour` | a quantity, if the user said one ("within 5 minutes") |
| `stops[].what` | `category` (one of ten), `text` (a generic kind with no category, such as "bakery"), `brand` (a name, copied as written), `min_power_kw`, `connector_types` |
| `stops[].anchor` | `offset` (`from` origin, destination or `waypoint:<n>`; `direction` after or before; `amount`), `every` (`amount`), or `fraction` (`value`) |
| `stops[].top_n` | options wanted for this stop, 1 to 5 |

The ten categories are `ev_charging`, `fuel`, `restaurant`, `fast_food`, `coffee`,
`rest_area`, `hotel`, `parking`, `supermarket`, `pharmacy`. A brand is never mapped to
a category by the model: "a McDonald's" is `brand: "McDonald's"`, and the map search
finds the name. A category is added only when the user also says the kind ("a Shell
petrol station").

## Compiling

`compile(&SurfaceIntent, &TripContext) -> Result<Plan, Vec<PlanError>>` returns every
problem it finds, not just the first.

**Places.**

| Written | Becomes |
| :--- | :--- |
| origin omitted or `current_location` | the host's current location as coordinates, or a `CurrentLocation` need |
| destination omitted | the host's `active_destination`, or a `Destination` need |
| `saved:<name>` | the saved place's coordinates (name matched case-insensitively), or an error listing the saved names |
| anything else | `PlaceQuery { text }`, geocoded later by the map provider |

**Units.** Distances: `m`, `km`, `mi` (and `meter`, `kilometer`, `mile`). Durations:
`s`, `min`, `h` (and `sec`, `second`, `minute`, `hour`). Units are lower-cased, `metre`
becomes `meter`, `hr`/`hrs`/`mins`/`secs`/`miles` are normalised, and a trailing `s`
is dropped from units longer than three letters.

**Defaults and context.** `avoid` merges the host's list with the request's.
`max_detour_s` is the request's value, else the host's, else the default (600 s).
`top_n` defaults to 3. An `every` anchor gets `window_pct` from the defaults (15).
Departure time, timezone and vehicle come from the host, never from the LLM.

**EV filters.** If the stop gives a power or connectors, those are used, and missing
connectors are taken from the vehicle. If the stop is `ev_charging` with neither, the
vehicle's EV filter is used.

**Anchors.** `from: origin` with `direction: before`, and `from: destination` with
`direction: after`, are rejected: they point off the route. A `waypoint:<n>` must
exist. A fraction must be strictly between 0 and 1.

**Ids.** A stop without an `id` gets `stop_<i>`. Ids must be unique, because choices
and results refer to stops by `(stop_request_id, occurrence)`.

## The canonical Plan

`crates/waylingo-plan/src/plan.rs`:

```text
Plan {
  schema_version: "0.1.0"
  origin, destination: Location          // Coords { lat, lng, label } | PlaceQuery { text }
  waypoints: Vec<Location>
  departure: { time: Option<RFC 3339>, timezone: Option<IANA> }
  vehicle: { ev: Option<EvFilter> }
  avoid: Vec<Tolls | Highways | Ferries>
  max_detour_s: u32
  stop_requests: Vec<StopRequest { id, what, anchor, top_n }>
}
Anchor = Offset   { from: Origin | Destination | Waypoint(u8), direction, amount }
       | Every    { interval: DistanceM | DurationS, window_pct }
       | Fraction { value }
```

The destination is always set: a plan without one cannot be compiled. A plan with no
stops is valid ("take me to Utrecht Centraal"); the engine then returns just the route.

`plan_hash` (`crates/waylingo-plan/src/schema.rs`) is the SHA-256 of the plan's JSON.
Field order is fixed by the type definitions, so equal plans hash equally. The result
document carries it, so an app can tell whether two results came from the same
understanding of the request.

## Validation limits

`Plan::validate` (`crates/waylingo-plan/src/validate.rs`) runs last in `compile`, and
again on every plan read back from a previous result by `reroute`.

| Limit | Value |
| :--- | :--- |
| Stops per plan | 10 |
| Waypoints per plan | 10 |
| Options per stop (`top_n`) | 1 to 5 |
| Offset or interval | positive, at most 5,000 km or 72 h |
| Shortest "every" | 1 km or 5 minutes |
| `every` window percentage | 1 to 50 |
| Fraction | strictly between 0 and 1 |
| Place text, brand, free text, stop id | at most 200 characters |
| Charger power | above 0, at most 1,000 kW |
| `max_detour_s` | 1 s to 3,600 s |
| Departure time and timezone strings | at most 64 bytes |
| Coordinates | finite, latitude within ±90, longitude within ±180 |
| Plan schema version | exactly `0.1.0` |

Why these exist: the plan decides how much work the engine does (how many anchors,
searches and routes). A plan can come from a model that misread the request, and on
`reroute` it comes from a document the client sent back. Without bounds, "a coffee
every metre" or a tampered plan with thousands of stops could make a server send
thousands of map requests. The limits keep the work of any plan small, and they work
together with the per-stop cap of 30 anchors and the map call budget described in
[Routing and stops](routing-and-stops.md#call-budget-retries-and-pacing).

The developer's own settings are checked too (`Defaults::validate` in
`crates/waylingo-plan/src/context.rs`): anchor radius 100 to 50,000 m, search span from
the radius up to 150,000 m, search limit at least 1, maximum detour 1 to 3,600 s,
`top_n` 1 to 5, `every_window_pct` 1 to 50.
