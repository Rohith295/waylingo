# Map providers

Waylingo supports two map providers, TomTom and HERE. An app uses exactly one,
chosen in its configuration. There is no mixing and no fallback from one to the other.

Why one provider per app: detour times subtract travel times, and times from two
providers' traffic models cannot be meaningfully subtracted. Mixing would also need
several keys per app and could conflict with providers' terms about combining their
content.

## The adapter trait

Each provider is a `ProviderAdapter` (`crates/waylingo-providers/src/types.rs`). Every
method is pure: it builds an `HttpRequest` or parses an `HttpResponse`. Adapters never
send anything and never hold a key.

```rust
pub trait ProviderAdapter {
    fn kind(&self) -> ProviderKind;
    fn capabilities(&self) -> Capabilities;
    fn attribution(&self) -> &'static str;
    fn setup_requests(&self, id_prefix: &str) -> Vec<HttpRequest>;
    fn apply_setup(&mut self, resp: &HttpResponse) -> Result<(), ProviderError>;
    fn geocode_request(&self, id: String, query: &str, bias: Option<LatLng>) -> HttpRequest;
    fn parse_geocode(&self, resp: &HttpResponse) -> Result<Vec<GeocodeCandidate>, ProviderError>;
    fn route_request(&self, id: String, q: &RouteQuery) -> HttpRequest;
    fn parse_route(&self, resp: &HttpResponse) -> Result<RouteResult, ProviderError>;
    fn search_request(&self, id: String, q: &SearchQuery) -> HttpRequest;
    fn parse_search(&self, resp: &HttpResponse) -> Result<Vec<Candidate>, ProviderError>;
    fn matrix_request(/* ... */) -> Option<HttpRequest> { None }
    fn parse_matrix(/* ... */) -> Result<Vec<Option<f64>>, ProviderError> { /* unsupported */ }
}
```

A parsed route is a `RouteResult`: a `MeasuredRoute` (see
[Routing and stops](routing-and-stops.md#the-measured-route)), the distance at the end of
each leg, and the provider's traffic delay. A search result is a list of `Candidate`s
with an id, name, position, address, brand and, where known, detour time and distance.
Everything an adapter parses is provider content: it goes into the GeoJSON output only.
The provider crate does not depend on the LLM crate, and `scripts/check_deps.sh` fails
CI if either ever depends on the other.

`Capabilities` tells the engine how to get detour times (`Native` or `Matrix`), the
most route points one search accepts, and the most results one search returns. It
also lists filter and avoid support, which is published as a support matrix; the engine
reads only the detour strategy and the two limits.

`ProviderKind` and `DetourStrategy` also have Google values (`Google`,
`FromRoutingSummaries`), left from the original design. There is no Google adapter.

## TomTom and HERE compared

| | TomTom (`tomtom.rs`) | HERE (`here.rs`) |
| :--- | :--- | :--- |
| Routing | Routing API v1 `calculateRoute`, GET, `traffic=true`, `travelMode=car` | Routing v8 `routes`, GET, `return=polyline,summary,actions`, `via` for intermediate points |
| Route timing | guidance instructions | section actions |
| Geocoding | Search v2 `geocode`, 5 results, biased to the current location | Geocoding v1 `geocode`, 5 results, `at=` the current location |
| Along-route search | Search v2 `searchAlongRoute`, POST with the window's points | Browse (category only) or Discover (brand or free text), GET with `route=<flexible polyline>;w=<metres>` |
| Search limit | 20 results, 250 route points | 100 results, 120 route points |
| Search bound | `maxDetourTime` | corridor width `w` (the anchor radius) |
| Detour times | in the search results (`detourTime`) | one Matrix v8 call per stop (`async=false`) |
| Category ids | fetched once per session from `poiCategories` and matched by name and synonyms | fixed table of HERE category ids |
| Charger power filter | `minPowerKW` | `evStation[minPower]` (category searches only) |
| Connector filter | `connectorSet` | not supported |
| Avoid | `tollRoads`, `motorways`, `ferries` | `avoid[features]=tollRoad,controlledAccessHighway,ferry` |
| Electric vehicle flag on routes | `vehicleEngineType=electric` | not sent |
| Departure time | `departAt` | `departureTime` |
| Key | query parameter `key` | query parameter `apiKey` |
| Attribution | `© TomTom` | `© HERE` |

Notes from the code:

* **TomTom's category list** is the only setup request. It is made only when the plan
  has stops. If it fails, the result gets a warning and searches use text only.
* **TomTom and brands.** A brand is sent as the search text, not as `brandSet`, because
  `brandSet` only knows chains in TomTom's brand registry. As text, a local name ("Cafe
  de Jaren") and a misspelt chain ("macdonalds") are both found.
* **TomTom and categories.** A category stop without a brand sends `*` plus the
  `categorySet` id, because generic text together with `categorySet` returned nothing in
  live tests. A brand with a category ("a Shell petrol station") sends the brand as the
  text plus `categorySet`, which works (`search_request` in `tomtom.rs`).
* **HERE and detours.** HERE's along-route search reports a distance off the route
  (`excursionDistance`, kept as `detour_m`), not a time. Ranking by distance off the road
  would prefer a place that is close but slow to reach, so the engine asks for a
  travel-time matrix (see [Detour time](routing-and-stops.md#detour-time)).
* **HERE's URL length.** The corridor is encoded into a GET URL; 120 route points keeps
  it under HERE's 2,048-byte guidance.

Both adapters set an 8-second `timeout_ms` hint on every request.

## Errors from providers

`ProviderError::from_status` (`crates/waylingo-providers/src/error.rs`) classifies a
failed response:

| Response | Kind | Code in the result |
| :--- | :--- | :--- |
| 401, 403 | `Auth` | `PROVIDER_AUTH` |
| 402 | `QuotaExceeded` | `PROVIDER_QUOTA_EXCEEDED` |
| 429 | `RateLimited` | `PROVIDER_RATE_LIMITED` |
| other 4xx | `BadRequest` | `PROVIDER_BAD_REQUEST` |
| 5xx | `Upstream` | `PROVIDER_UNAVAILABLE` |
| no response | `Transport` | `PROVIDER_UNREACHABLE` |
| 2xx that does not parse | `BadResponse` | `PROVIDER_BAD_RESPONSE` |
| route reply with no route | `NoRoute` | `NO_ROUTE` |

TomTom also answers 403 for some over-limit cases, so a used-up TomTom quota can appear
as `PROVIDER_AUTH`. The message includes the provider's own error text (HERE `title`
and `cause`, TomTom `errorText`, and so on), cut to 300 characters, with words that look
like keys (`key=`, `apikey=`, or longer than 40 characters) masked. URLs are never put
in messages, because a URL may carry a key.

## Keys and proxies

The engine emits requests with `credential: Map` (or `Llm`) and an `auth` placement.
The host adds the key at send time. In `waylingo-reqwest` this happens in one place,
`prepare` in `crates/waylingo-reqwest/src/wire.rs`: a query-parameter key is
percent-encoded and appended to the URL, a header key is added with its prefix, and an
empty key adds nothing (a local LLM server needs none). `Keys` prints as
`<redacted>`. Errors from the hosts never include URLs.

A key shipped inside an app can be extracted, so the bindings support proxies:

* **Map proxy.** `ProviderChoice::TomTom { base_url }` replaces
  `https://api.tomtom.com`. `ProviderChoice::Here { base_url }` routes every HERE
  service through `{base}/router`, `{base}/geocode`, `{base}/browse`, `{base}/discover`
  and `{base}/matrix`. The proxy adds the real key; the app's map key may be empty or
  carry the proxy's own token.
* **LLM proxy.** Give the LLM backend a `base_url`; the LLM key may then be left out.

## Attribution and terms

Every result carries the provider's `attribution` string, which must be shown with the
results. Each developer's use of a provider is governed by their own agreement with that
provider; Waylingo does not check it.
