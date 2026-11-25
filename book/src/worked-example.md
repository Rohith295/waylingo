# One trip, step by step

This page follows one request through the engine with numbers. The route, the
candidates and their detours are **illustrative**: invented to show how the code
decides, not output from a real provider. The rules applied to them are the ones in
[Routing and stops](routing-and-stops.md).

The request, sent with the user's current location and the default settings (TomTom,
anchor radius 5 km, search span 15 km, `top_n` 3, maximum detour 600 s):

> Drive to Berlin, a fast charger every 200 km and a McDonald's 15 minutes before we
> arrive

## 1. The LLM writes the intent

```json
{"destination": "Berlin",
 "stops": [
   {"id": "charger", "what": {"category": "ev_charging", "min_power_kw": 150},
    "anchor": {"kind": "every", "amount": {"value": 200, "unit": "km"}}},
   {"id": "food", "what": {"brand": "McDonald's"},
    "anchor": {"kind": "offset", "from": "destination", "direction": "before",
               "amount": {"value": 15, "unit": "min"}}}]}
```

"Fast charger" with no number becomes 150 kW (a rule in the prompt). "McDonald's" is a
brand, not a category. One LLM call; no coordinates in it.

## 2. The compiler makes the plan

Units become metres and seconds, the missing origin becomes the current location,
defaults are filled in (trimmed):

```json
{"origin": {"kind": "coords", "lat": 52.3702, "lng": 4.8952, "label": "current_location"},
 "destination": {"kind": "place_query", "text": "Berlin"},
 "max_detour_s": 600,
 "stop_requests": [
   {"id": "charger",
    "what": {"category": "ev_charging", "text": null, "brand": null,
             "ev": {"min_power_kw": 150.0, "connector_types": []}},
    "anchor": {"kind": "every", "interval": {"distance_m": 200000.0}, "window_pct": 15},
    "top_n": 3},
   {"id": "food",
    "what": {"category": null, "text": null, "brand": "McDonald's", "ev": null},
    "anchor": {"kind": "offset", "from": "destination", "direction": "before",
               "amount": {"duration_s": 900.0}},
    "top_n": 3}]}
```

## 3. Geocoding and the base route

Map calls so far: TomTom's category list (the plan has stops), one geocode for
"Berlin", then one route from the current location to the top geocoding match.

Suppose the route is 650 km and 6.5 h (23,400 s), with checkpoints showing a fast
first 600 km (reached at 19,800 s) and a slow last 50 km through the city (3,600 s).

## 4. Anchors

**Charger, every 200 km.** Marks at 200, 400 and 600 km. The next one, 800 km, is past
the end. The 600 km mark is kept because 50 km remain after it, more than 15% of 200 km
(30 km). That gives occurrences 0, 1 and 2.

**Food, 15 minutes before arrival.** Target time 23,400 − 900 = 22,500 s. That is
2,700 s into the city section, which covers 50 km in 3,600 s, so 37.5 km past the
600 km checkpoint: the anchor is at 637.5 km.

```text
0 km        200        400        600  637.5  650
|------------x----------x----------x----●------|
            c#0        c#1        c#2  food
```

## 5. Windows and searches

Each anchor gets a window of 15 km each side, clamped to the route:

| Stop | Anchor | Window |
| :--- | ---: | :--- |
| charger #0 | 200 km | 185 to 215 km |
| charger #1 | 400 km | 385 to 415 km |
| charger #2 | 600 km | 585 to 615 km |
| food | 637.5 km | 622.5 to 650 km (clamped at the end) |

Four along-route searches go out in one batch. The charger searches send `*` with
TomTom's EV charging category and `minPowerKW=150`; the food search sends
"McDonald's" as text. All ask for places within a 600 s detour.

## 6. Choosing charger #2

Suppose the search in the 585 to 615 km window returns three places (fictional):

| Place | Distance from anchor | Detour |
| :--- | ---: | ---: |
| Example Charge Hub | 1.2 km | 240 s |
| Sample Fast Charge | 3.8 km | 150 s |
| Demo Chargepoint | 7.5 km | 90 s |

Two are within the 5 km radius; they come first and are ranked by detour time.
Sample Fast Charge (150 s) is the **pick** and Example Charge Hub (240 s) the
**alternative**. Demo Chargepoint has the smallest detour but is beyond the radius, and
alternatives never cross the radius, so it is not offered. Had nothing been within
5 km, it would have become the pick, flagged `beyond_radius`.

Stops are chosen in plan order, so charger #0 and #1 have already taken their places;
none of those can be chosen again here. The food stop is chosen last the same way.

## 7. The final route

The chosen places are ordered by where they sit along the base route (`along_m`), and
one route request goes through them:

```text
current location → charger #0 → charger #1 → charger #2 → McDonald's → Berlin
```

That route gives the real total distance and time, including detours, and each stop's
`eta_s`.

## 8. The count

| Calls | Number |
| :--- | ---: |
| LLM | 1 |
| Category list | 1 |
| Geocode | 1 |
| Base route | 1 |
| Searches | 4 |
| Final route | 1 |
| **Map calls in total** | **8** |

The offline test `query_in_geojson_out` (`crates/waylingo/tests/e2e.rs`) runs this
request against fixtures and checks the same 1 LLM call and 8 map calls. With HERE
there would be no category list but up to four matrix calls, one per stop with
candidates. If the end user then picks Example Charge Hub instead,
[reroute](reroute.md) makes one more map call: a new final route.
