# Provider Capability Matrix: HERE, Google Maps Platform, TomTom

* **Author:** rohithrangaraju
* **Date:** 2026-11-24
* **Companion to:** `concepts_brief.md`
* **Method:** three researchers read each provider's official docs, pricing and terms on 2026-11-24. Items marked **[unconfirmed]** could not be checked on an official page. Re-check all prices, limits and terms before release: they change.

## 1. Summary for the common interface

| Capability the engine needs | HERE | Google | TomTom |
| :--- | :--- | :--- | :--- |
| Route with time and distance along it | Routing v8 `spans` (`length`, `duration`) with polyline offsets | Routes API steps: `distanceMeters` and `staticDuration` only (no traffic per step) | v1: instruction `routeOffsetInMeters` + `travelTimeInSeconds`; Orbis v3: `progressPoints` |
| Route geometry format | Flexible Polyline only, no GeoJSON | Encoded polyline or `GEO_JSON_LINESTRING` | v1: `{latitude, longitude}` objects; Orbis v3: GeoJSON |
| Along-route POI search | Browse/Discover `route=` (**BETA**), GET, keep under ~300 points | Text Search (New) `searchAlongRouteParameters`, encoded polyline only | Search v2 `searchAlongRoute`, POST, route points in body |
| Detour time returned | **No** (`excursionDistance` only, a distance) | **No** detour field; `routingSummaries` gives origin→place and place→destination legs to compute it | **Yes**: `detourTime`, `detourDistance` |
| Results per search call | `limit` up to 100 | 20 per page, 60 total | `limit` up to 20 |
| Matrix routing | Matrix v8, async by default; sync 15×100 (flexible) or 500×500 (region) | `computeRouteMatrix`, 625 elements (100 with optimal traffic) | Matrix v2 sync 100 cells (200 with conditions) |
| EV charger filters in request | `evStation[minPower]`, `connectorTypeIds`, `current` on Browse | `evOptions.minimumChargingRateKw`, `connectorTypes` | `minPowerKW`, `connectorSet` on `searchAlongRoute` |
| EV range routing with charging stops | Yes: `ev[makeReachable]=true` (needs charging curve, consumption model) | Not documented | Yes: Long Distance EV Routing (v1 only) |
| Truck mode | Yes | Yes (`TRUCK`, Enterprise SKU) | v1 yes; Orbis v3 no |
| Geocoding region bias | `in=countryCode:NLD` (filter), `at=` (bias) | `regionCode`, `locationBias` | `countrySet` (filter), `lat`/`lon` (bias) |
| Auth | API key or OAuth 2.0 (tokens up to 24 h) | API key | API key only |
| Key restriction options | Trusted domains (keys only); no mobile bundle restriction | iOS/Android headers documented generally, **not confirmed** for Routes or Places (New); Google recommends IP restriction or a proxy | Domain allowlist, per-product restriction; no mobile restriction |
| Free usage | Base Plan, **card required**; car routing 30K/mo, Search 5K/mo, Matrix 2.5K/mo, EV routing 2.5K/mo | Per-SKU monthly caps: Essentials 10K, Pro 5K, Enterprise 1K; billing account required | Monthly, **no card required**: Routing/Geocoding 20K, Search 2.5K, Matrix 2.5K, LD EV Routing 2.5K |
| Rate limits | Not published for Base Plan; 429/503 on surges | Not covered in research | Default 5 QPS for Routing and Search, 10 for Matrix |
| Caching allowed | Per HTTP cache headers, else up to 30 days | `place_id` indefinitely; lat/lng up to 30 days; nothing else | Only within Cache-Control max-age; no shared cache; no derived database |
| Show results on another company's map | No ban found; attribution of both required | **Banned outside the EEA** (ToS §3.2.3(e), SST §15.2, §20.2). EEA: place details (names, ratings, EV info) may not be shown on any map | Allowed (§13.5) **[from archived terms, live page unreadable]** |
| Use with AI / LLMs | **Banned** "in connection with" any AI system, including NLP and generative AI, unless the subscription plan allows it (§3c vii) | Training/fine-tuning banned; using at inference time allowed, with conditions (model must not train on input, no caching, attribution shown) | No AI clause found **[archived terms]** |

## 2. What this means for the design

### 2.1 HERE's AI clause: handled by design

HERE Platform Terms §3c(vii) (read directly from here.com on 2026-11-24) forbid using HERE Materials "in connection with" a machine-learning or AI system, including natural-language processing and generative AI. This applies "except as otherwise permitted in your Subscription Plan or a separate written agreement with HERE".

In Waylingo, the LLM only parses the end user's text into a plan. Ordinary program code then calls HERE with the developer's key, and HERE results never reach any LLM. That keeps HERE Materials out of the AI system. The design rules that follow:

* no provider result reaches any LLM. Parsing finishes before the first map call, the LLM interface only accepts the user's text, and there is no summary for an LLM agent. This is enforced by types, module boundaries and a canary test (see the brief's "Hard Rule" section);
* the docs state that each developer's HERE use is governed by their own HERE agreement.

Asking HERE to confirm this reading is optional, because "in connection with" is broad wording.

The same §3c also forbids:

* **(i)** integrating HERE Materials into a vehicle system, including in-car display screens. Phone projection onto a car screen is expressly allowed.
* **(iii)** exposing HERE Materials to open-source projects or databases. Recorded HERE responses must not be committed to the public repo.

### 2.2 Google's map-display ban breaks "GeoJSON on any map"

Outside the EEA, Google Routes and Places content may not be shown on or used with a non-Google map. Inside the EEA (billing address in the EEA), the GeoJSON line and points can go on any map, but place details (names, ratings, EV info) may not be shown on any map at all, unless the developer uses Google's Places UI Kit.

**Action:** the Google adapter's output carries a `display_terms` field in its metadata, and the docs say plainly:
* outside the EEA, draw Google results on a Google map only;
* inside the EEA, show place details outside the map.

The library cannot enforce this.

Google also allows at-inference LLM use only if the model does not train on the input. The docs must pass that condition on to developers.

### 2.3 TomTom is the least restricted, with one EV catch

TomTom has native detour time, the simplest terms of the three (from an archived copy of its terms; confirm against the live page), and a free tier with no card. The catch: the dedicated **EV Search API is sales-only**, not in the free or pay-as-you-go plans. The free path for chargers is `searchAlongRoute` with an EV category plus `minPowerKW` and `connectorSet`. EV station and fast-food category IDs must be fetched at runtime from `GET /search/2/poiCategories.json`. The commonly quoted `7309` was not confirmed.

TomTom is mid-migration from v1 ("Genesis") to Orbis. Orbis v3 routing is GeoJSON and recommended for new work, but it has no truck mode, no EV consumption and no Long Distance EV Routing. Orbis along-route search is in public preview. **Use v1 Routing and Search v2 for v0.1**, and plan an Orbis move later.

### 2.4 Detour strategy per provider

| Provider | Strategy | Calls per stop request |
| :--- | :--- | :--- |
| TomTom | Native `detourTime` from `searchAlongRoute`, `sortBy=detourTime` | 1 search |
| Google | Text Search with `searchAlongRouteParameters` + `routingParameters.origin` and `routingSummaries` → detour = leg0 + leg1 − base duration from origin to destination | 1 search (billed at the Enterprise + Atmosphere SKU, $40 per 1,000 after the free cap) |
| HERE | Browse with `route=` and `ranking=excursionDistance`, pre-filter top k, then one Matrix call (A→Pi, Pi→B) | 1 search + 1 matrix |

### 2.5 Time along the route per provider

* **HERE:** request `return=polyline,summary&spans=length,duration,segmentId`. Map each span's polyline offset to cumulative distance and time. This is the finest data of the three.
* **TomTom v1:** request `instructionsType=coded`. Each instruction gives `routeOffsetInMeters` and `travelTimeInSeconds` at a point index. Interpolate between instructions.
* **Google:** steps carry `distanceMeters` and `staticDuration` (free-flow time, no traffic). Scale the step times so they sum to the leg's traffic-aware `duration`. This is the coarsest data, and anchors on Google will be less exact in heavy traffic.

### 2.6 Coordinate and geometry normalization

The adapters must convert five input formats into one internal measured route, with GeoJSON `[lon, lat]` on export:

* HERE Flexible Polyline (`flexpolyline` crate 1.0.0)
* Google encoded polyline, precision 5 (`polyline` crate), or `GEO_JSON_LINESTRING`
* TomTom v1 `{latitude, longitude}`
* TomTom Search v2 `{lat, lon}`
* TomTom Orbis / EV Search GeoJSON `[lon, lat]`

Along-route search inputs differ too:
* Google needs an encoded polyline, so request `ENCODED_POLYLINE` and decode it yourself.
* HERE needs a Flexible Polyline in the URL, kept under ~300 points.
* TomTom takes points in a POST body.

All three need the route window around each anchor simplified before sending.

### 2.7 Credentials per provider

* **HERE:** prefer OAuth 2.0 tokens minted by the developer's backend (lifetime up to 24 h, set shorter with `expiresIn`). API keys can only be restricted by domain.
* **Google:** the iOS/Android key headers are not confirmed to work for Routes or Places (New). Google itself recommends IP restriction or a proxy for these APIs, so the mobile path for Google is "proxy only".
* **TomTom:** key only, sent as a `key=` query parameter (v1/v2), with domain and per-product restrictions. Mobile keys are unprotected, so a proxy is advised.

## 3. Gotchas per provider

**HERE**
1. AI clause §3c(vii): never pass HERE results to an LLM (the engine-wide hard rule covers this). Also: no built-in in-car integration (§3c(i), phone projection is fine), and no HERE responses in the public repo (§3c(iii)).
2. Browse `route=` is BETA, and has a ~2048-byte URL guideline.
3. No detour time: budget one Matrix call per stop request.
4. Matrix is async by default: pass `async=false` for small requests.
5. EV Charge Points API v2 is deprecated: use Search with `show=ev`, and EV Charge Points v3 for tariffs and availability.
6. Use `saeJ3400`, not `tesla`, for connectors.

**Google**
1. Map-display ban outside the EEA; place details may not be shown on any map inside the EEA.
2. Search along route only takes encoded polylines.
3. `routingSummaries` and `evChargeOptions` move every search to the $40 per 1,000 SKU after the free cap.
4. Traffic-aware routing or more than 10 waypoints moves routing to the Pro SKU.
5. `X-Goog-FieldMask` is required on every call.
6. Directions, Distance Matrix and Places (legacy) are Legacy since 2025-03-01: do not use them.

**TomTom**
1. EV Search API and probably EV availability are sales-only.
2. Category IDs: fetch at runtime, never hard-code.
3. Three coordinate formats across its own APIs.
4. `limit` max 20 and `maxDetourTime` max 3600 s on along-route search; maximum route points not documented, so test it.
5. Orbis v3 lacks truck mode and EV consumption: stay on v1 for v0.1.
6. Default 5 QPS on Routing and Search: the engine's concurrency limit per provider must respect this.

## 4. Other findings

* The reference Google Maps MCP server was archived (read-only since 2025-05-29). Google's replacement is "Maps Grounding Lite" (`mapstools.googleapis.com/mcp`) with `compute_routes` and `search_along_route` tools. That makes it the closest existing competitor for the Google case. It places no time or distance anchors, as far as its docs show.
* Gemini "Grounding with Google Maps" takes a point location; along-route search is not documented there.

## 5. Sources

* HERE Routing v8 OpenAPI: https://router.hereapi.com/v8/openapi
* HERE spans: https://docs.here.com/routing/docs/routing-v8-span
* HERE Browse: https://docs.here.com/geocoding-and-search/reference/get_browse
* HERE Matrix modes: https://docs.here.com/routing/docs/matrix-v8-matrix-modes
* HERE EV routing: https://docs.here.com/routing/docs/routing-v8-calculate-route-with-charging
* HERE pricing: https://www.here.com/get-started/pricing/supplemental-pricing
* HERE terms: https://www.here.com/en-gb/terms/here-platform-terms
* HERE OAuth: https://docs.here.com/identity-and-access-management/docs/manage-api-oauth
* Google Routes response: https://developers.google.com/maps/documentation/routes/understand-route-response
* Google Routes billing: https://developers.google.com/maps/documentation/routes/usage-and-billing
* Google search along route: https://developers.google.com/maps/documentation/places/web-service/search-along-route
* Google routing summaries: https://developers.google.com/maps/documentation/places/web-service/routing-summary-sar
* Google Places reference (EV): https://developers.google.com/maps/documentation/places/web-service/reference/rest/v1/places
* Google pricing: https://developers.google.com/maps/billing-and-pricing/pricing
* Google key security: https://developers.google.com/maps/api-security-best-practices
* Google terms: https://cloud.google.com/maps-platform/terms and https://cloud.google.com/maps-platform/terms/maps-service-terms
* Google EEA terms: https://cloud.google.com/terms/maps-platform/eea
* Google Places policies: https://developers.google.com/maps/documentation/places/web-service/policies
* Google Grounding Lite: https://developers.google.com/maps/ai/grounding-lite
* TomTom routing v1: https://docs.tomtom.com/routing-api/documentation/tomtom-maps/calculate-route
* TomTom Orbis v3 routing: https://docs.tomtom.com/routing-api/documentation/tomtom-orbis-maps/v3/calculate-route
* TomTom along-route search: https://docs.tomtom.com/search-api/documentation/search-service/along-route-search
* TomTom EV Search: https://docs.tomtom.com/ev-search-api/documentation/product-information/introduction
* TomTom Long Distance EV Routing: https://docs.tomtom.com/long-distance-ev-routing-api/documentation/tomtom-maps/long-distance-ev-routing
* TomTom Matrix v2: https://docs.tomtom.com/matrix-routing-v2-api/documentation/synchronous-matrix
* TomTom QPS: https://docs.tomtom.com/platform/documentation/api-best-practices/qps-limits
* TomTom pricing: https://docs.tomtom.com/pricing
* TomTom POI categories: https://docs.tomtom.com/search-api/documentation/poi-categories-service/poi-categories
