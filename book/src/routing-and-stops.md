# Routing and stops

This chapter covers what the map providers' APIs do not: placing "40 minutes in" or
"every 200 km" on a real route, searching the road near that point, and picking the
place that adds the least driving time. All of it is ordinary code with tests; the LLM
is not involved. [One trip, step by step](worked-example.md) walks through it with
numbers.

The steps, for each stop request in the plan:

```text
base route ──▶ measured route (distance and time at every point)
                   │
anchor ────────────┼──▶ anchor point(s) on the route   (local maths, no API call)
                   │
                   ├──▶ search window: a stretch of route around each anchor point
                   │
                   ├──▶ along-route search in that stretch          (1 call per anchor)
                   │
                   ├──▶ detour times (TomTom: in the search reply; HERE: 1 matrix call)
                   │
                   └──▶ choose: one place per stop, plus alternatives
                             │
                             ▼
              final route through all chosen places               (1 call)
```

## The measured route

A provider returns a route as a line of coordinates plus some timing data. Waylingo
turns it into a `MeasuredRoute` (`crates/waylingo-geo/src/measured.rs`): the
coordinates plus the cumulative distance `cum_m` and cumulative driving time `cum_s` at
every coordinate.

The provider reports distance and time at certain points along the route
(*checkpoints*). Between two checkpoints, Waylingo spreads the provider's distance and
time over the coordinates in proportion to the haversine length of each segment. So
the totals always match the provider's, and the speed can change from one stretch to
the next.

```text
checkpoints:  0 km / 0 s ───────── 600 km / 19,800 s ──── 650 km / 23,400 s
              (motorway: about 109 km/h)            (city: 50 km/h)
```

This matters for time anchors. With an even spread the average speed is 650 km in
6.5 h, 100 km/h, so "15 minutes before we arrive" would land 900 s × 100 km/h = 25 km
before the end. With the checkpoints it lands in the city section at 50 km/h:
900 s × 50 km/h = 12.5 km before the end, which is what the driver will experience
(`minutes_before_end_uses_slow_city_section` in `crates/waylingo-geo/src/anchor.rs`).

Where the checkpoints come from:

* **TomTom:** the guidance instructions' `pointIndex`, `routeOffsetInMeters` and
  `travelTimeInSeconds`; an instruction is kept only when its point index increases and
  its distance and time do not go backwards; the last checkpoint is
  the route summary's length and travel time (`crates/waylingo-providers/src/tomtom.rs`).
  Routes are requested with `traffic=true`.
* **HERE:** each section's Flexible Polyline is decoded and joined; each action's
  `offset`, `length` and `duration` add a checkpoint, and each section end adds one so
  legs line up exactly (`crates/waylingo-providers/src/here.rs`).

Both adapters also report `leg_end_m`, the distance at the end of each leg, which
places waypoints on the route.

**Lookup.** `position_at_distance(m)` and `position_at_time(s)` binary-search the
`cum_m` or `cum_s` array (`partition_point`) for the segment that contains the value,
then interpolate the position and the other axis linearly within it. Values outside the
route return nothing.

## Anchors

The engine converts each plan anchor into an `AnchorSpec`
(`crates/waylingo/src/engine/anchor.rs`), and `resolve`
(`crates/waylingo-geo/src/anchor.rs`) turns it into one or more `AnchorPoint`s, each
with a position, `cum_m`, `cum_s` and an `occurrence` number.

**Offset.** Find the reference point (start, end, or a waypoint's `leg_end_m`), then
move along the axis of the amount: a distance moves along `cum_m`, a time along
`cum_s`.

```text
"15 min before we arrive":  target cum_s = total_s - 900
"after 100 km":             target cum_m = 0 + 100,000
"10 km after <waypoint 0>": target cum_m = leg_end_m[0] + 10,000
```

A target beyond either end of the route is an error. The stop is then reported as
`UNSUPPORTED` with the reason in `detail`.

**Fraction.** `total_m × value`. "Halfway" is half the *distance*, not half the time.

**Every N.** Marks at N, 2N, 3N, ... from the start, measured on `cum_m` for a distance
interval or `cum_s` for a time interval. A mark is dropped when it is at or past the end,
or when less than `window_pct` percent of N (default 15%) is left after it: a stop that
close to the destination is not useful.

```text
650 km route, "every 200 km", 15% of 200 km = 30 km tail:
0        200        400        600   650
|---------x----------x----------x-----|     600 km mark kept: 50 km left ≥ 30 km
```

If no mark fits ("every 200 km" on a 150 km route) the stop is `UNSUPPORTED` with "the
route is shorter than the interval". One stop request may create at most 30 marks
(`MAX_ANCHORS_PER_STOP` in `engine/phases.rs`); further marks are dropped with a
warning.

Marks are counted from the start of the trip, not from the previous chosen stop. That
keeps every search independent, so all searches go out in one parallel batch. The cost
is that the real gap between two chosen stops can be longer than N (see
[Design decisions](design-decisions.md)).

## Search windows

For each anchor point the engine searches a stretch of the route around it, by
distance (`Window::by_distance` in `crates/waylingo-geo/src/window.rs`):

```text
                    anchor
                      │
 ───────[─────────────●─────────────]───────▶ route
        start_m                    end_m
        ◀── search_span_m ──▶◀── search_span_m ──▶   (clamped to the route)
```

Two developer settings control it (`Defaults` in `crates/waylingo-plan/src/context.rs`):

| Setting | Default | Meaning |
| :--- | :--- | :--- |
| `anchor_radius_m` | 5,000 m | places within this straight-line distance of the anchor point are preferred |
| `search_span_m` | 3 × radius (15 km) | route searched on each side of the anchor; at least the radius, at most 150 km |

Why a fixed span and not a percentage of the offset: a stop "every 200 km" and a stop
"in 20 minutes" both want a place near a point, and the useful search area depends on
how far a driver will go off the road, not on how far the point is from the start. The
span is wider than the radius so that places just beyond the radius are still found
when nothing is closer (see "beyond the radius" below).

The window's slice of the route (with interpolated end points,
`MeasuredRoute::slice`) is simplified with Douglas-Peucker before it is sent, to stay
within each provider's limit on route points: 250 for TomTom and 120 for HERE, whose
search is a GET request with the corridor encoded in the URL
(`simplify_to_max_points` in `crates/waylingo-geo/src/simplify.rs`). The simplifier
always drops points within 0.5 m of the line, then finds by bisection the smallest
tolerance that fits the limit, and always keeps the first and last point.

## Along-route search

One request per anchor point, all in one batch (`search_requests` in
`engine/phases.rs`). What the request asks for:

* **TomTom** `searchAlongRoute` (POST, the window's points in the body): `maxDetourTime`
  is the plan's `max_detour_s`, results are sorted by detour time, up to
  `min(search_limit, 20)` results. A category stop without a brand sends `*` as the
  query text plus the `categorySet` id learned from TomTom's category list, because
  generic text together with `categorySet` returned nothing in live tests. A brand with
  a category ("a Shell petrol station") sends the brand as the text plus `categorySet`,
  which works. A brand is sent as the query text, not
  as `brandSet`, because `brandSet` only matches chains in TomTom's brand registry and
  would miss a local name. EV stops add `minPowerKW` and `connectorSet`.
* **HERE** Browse (category only) or Discover (brand or free text), with
  `route=<flexible polyline>;w=<corridor>`: the corridor width is `anchor_radius_m`,
  clamped to 100 to 50,000 m. An EV category stop with a power adds
  `evStation[minPower]`.

The details are in [Map providers](providers.md).

## Detour time

The detour of a place is the extra driving time it adds compared with staying on the
route.

**TomTom** returns `detourTime` (and `detourDistance`) with each result, relative to the
route points that were sent.

**HERE** returns only a distance off the route, so the engine computes detours with one
travel-time matrix per stop (`detour_requests`, `apply_matrix` in `engine/phases.rs`):

```text
A = window start      B = window end      P1..Pk = candidates
origins      = [A, P1, ..., Pk]
destinations = [P1, ..., Pk, B]
detour(Pi)   = t(A→Pi) + t(Pi→B) − t(A→B)        (never below 0)
```

To keep the matrix small, the candidates are first shortlisted: places within the
anchor radius before others, nearest to the anchor first, at most 8
(`MATRIX_SHORTLIST`). A place the end user chose earlier is always kept, even if it is
not among the nearest. Candidates outside the shortlist are dropped. If the matrix call
fails, the stop keeps its candidates without detour times, and a warning says they were
ranked by distance from the anchor.

## Choosing one place per stop

`choose` in `crates/waylingo/src/select.rs`, called for each stop in plan order and,
within a stop, in occurrence order (`choose_stops` in `engine/stops.rs`):

1. Drop places already used by another stop, so one place never serves two stops.
   Places the end user chose are reserved first.
2. Drop places whose known detour is over `max_detour_s`. A place with an unknown
   detour is kept.
3. If `strict_radius` is set, drop places beyond `anchor_radius_m`.
4. Sort: places within the radius first, ranked by detour time (unknown last), then by
   straight-line distance to the anchor. Places beyond the radius are ranked by distance
   only.
5. The first place is the **pick**. The next `top_n − 1` places on the same side of the
   radius are the **alternatives**.

```text
within radius (5 km)        beyond radius
┌──────────────────────┐    ┌─────────────────┐
│ detour 120 s  ← pick │    │ 6.2 km          │
│ detour 300 s  ← alt  │    │ 9.8 km          │
│ detour ?      ← alt  │    └─────────────────┘
└──────────────────────┘
```

Alternatives never cross the radius: if the pick is within it, places beyond it are not
offered.

When nothing is within the radius (and `strict_radius` is off), the nearest place
beyond it becomes the pick, the stop is flagged `beyond_radius: true`, and the result
gets a warning. The app can show "nearest match is 9 km from where you asked". With
`strict_radius` on, such a stop is reported as not found instead.

A stop with no usable place becomes an `unresolved_stop` with `NO_CANDIDATES` and a
detail such as "provider returned 3 result(s), 1 within the detour limit". With
`strict_radius`, that count is of results within the detour limit only; it does not say
how many were also within the radius. After all
stops are chosen, a place that became another stop's pick is removed from every
alternatives list, because the end user could not choose it.

Each place also gets `along_m`: the distance along the base route of the route point
nearest to it, looking only inside the stop's window (on a route that passes the same
area twice, the place belongs to the stretch that was searched). If the window holds no
route point, the whole route is used (`nearest_cum_m` in `select.rs`). This orders the
stops on the final route.

## The final route

`route_through_stops` (`engine/stops.rs`) builds one route request: origin, then the
waypoints and chosen places sorted by `along_m`, then the destination. If no stop was
found, the base route is already the answer and no request is made.

The final route gives the real totals, including detours, and the arrival time at each
stop (`eta_s`, seconds from departure, read from the final route at the end of the leg
that reaches the stop). If the final route fails, the trip does not fail: the result
keeps the base route, ETAs fall back to the anchor's time on the base route, and a
warning says so.

Anchors are placed on the base route and are not moved by earlier stops' detours or
time spent at a stop. The final route reports the true arrival times, but a stop "two
hours in" is two hours into the base route, not two hours of driving after earlier
stops.

## Call budget, retries and pacing

**Budget.** `Limits.max_map_calls` (default 25) counts every map request a query
makes, retries included (`crates/waylingo/src/config.rs`). It is checked before each
search and each matrix request, keeping one call in reserve for the final route: a
search that does not fit is reported as `SKIPPED_BUDGET`; a matrix that does not fit
leaves that stop without detour times and adds a warning. Setup, geocoding, the base
route, the final route and retries are counted but not refused, so a query with many
retries can end slightly above the limit. The result's `usage` reports the calls made.

**Retries.** A map response with 429, a 5xx or a transport failure (other than
cancellation) is sent again up to `max_map_retries` (default 2) times, after the
`Retry-After` value or 1 s, then 2 s (at most 30 s), passed as `not_before_ms`
(`Session::handle` in `engine/mod.rs`). Other errors are classified and reported (see
[Errors](errors.md)).

**Pacing** is the host's job, because only the host knows how many queries run at
once. The hosts in `crates/waylingo-reqwest/`:

* the blocking `drive` sends one request at a time and waits 220 ms before each map
  request (`MAP_SPACING`, under TomTom's default 5 requests per second);
* `AsyncHost` sends each batch in parallel, but map requests pass through one gate
  shared by every query on that host, so their *starts* are at least `map_spacing`
  apart; the whole query has a deadline, and time spent waiting at the gate counts
  against it. The bindings set the spacing from `map_requests_per_second` (default 4.5)
  and the deadline from `timeout_ms` (default 60 s).
