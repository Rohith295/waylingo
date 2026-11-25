//! TomTom adapter: Routing API v1, Search API v2 (geocode, searchAlongRoute,
//! poiCategories). Orbis v3 is not used yet: it lacks truck and EV consumption.
//!
//! Docs: <https://docs.tomtom.com/routing-api/documentation/tomtom-maps/calculate-route>,
//! <https://docs.tomtom.com/search-api/documentation/search-service/along-route-search>.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::json;
use waylingo_geo::{Checkpoint, LatLng, MeasuredRoute};
use waylingo_http::{AuthPlacement, CredentialSlot, Header, HttpRequest, HttpResponse};
use waylingo_plan::{Avoid, Category, Connector};

use crate::category::{provider_names, query_text};
use crate::encode::{ll, pct, position_id, url};
use crate::{
    Candidate, Capabilities, DetourStrategy, ErrorKind, GeocodeCandidate, ProviderAdapter,
    ProviderError, ProviderKind, RouteQuery, RouteResult, SearchQuery,
};

const DEFAULT_BASE: &str = "https://api.tomtom.com";
const MAX_SEARCH_POINTS: usize = 250;
const TIMEOUT_MS: u32 = 8_000;

/// TomTom adapter. Holds the category id table learned from `setup_requests`.
#[derive(Debug, Clone)]
pub struct TomTom {
    base: String,
    category_ids: HashMap<Category, u32>,
}

impl Default for TomTom {
    fn default() -> Self {
        Self::new(None)
    }
}

impl TomTom {
    /// `base_url` overrides `https://api.tomtom.com`, for a proxy.
    #[must_use]
    pub fn new(base_url: Option<&str>) -> Self {
        Self {
            base: base_url
                .unwrap_or(DEFAULT_BASE)
                .trim_end_matches('/')
                .to_string(),
            category_ids: HashMap::new(),
        }
    }

    /// Category id learned from the provider's category list, if any.
    #[must_use]
    pub fn category_id(&self, c: Category) -> Option<u32> {
        self.category_ids.get(&c).copied()
    }

    fn request(
        &self,
        id: String,
        method: &str,
        url: String,
        body: Option<serde_json::Value>,
    ) -> HttpRequest {
        let mut headers = vec![];
        if body.is_some() {
            headers.push(Header {
                name: "Content-Type".into(),
                value: "application/json".into(),
            });
        }
        HttpRequest {
            id,
            method: method.into(),
            url,
            headers,
            body: body.map(|b| serde_json::to_vec(&b).unwrap_or_default()),
            credential: CredentialSlot::Map,
            auth: AuthPlacement::QueryParam { name: "key".into() },
            timeout_ms: TIMEOUT_MS,
            not_before_ms: 0,
        }
    }
}

fn parse_json<'a, T: Deserialize<'a>>(resp: &'a HttpResponse) -> Result<T, ProviderError> {
    if let Some(e) = ProviderError::from_status(resp) {
        return Err(e);
    }
    serde_json::from_slice(&resp.body)
        .map_err(|e| ProviderError::bad_response(format!("TomTom JSON: {e}")))
}

fn connector_code(c: Connector) -> &'static str {
    match c {
        Connector::Ccs1 => "IEC62196Type1CCS",
        Connector::Ccs2 => "IEC62196Type2CCS",
        Connector::Chademo => "Chademo",
        Connector::Type1 => "IEC62196Type1",
        Connector::Type2 => "IEC62196Type2CableAttached,IEC62196Type2Outlet",
        Connector::Nacs => "Tesla",
    }
}

// ---- response shapes (only the fields we use) ----

#[derive(Deserialize)]
struct Pos {
    lat: f64,
    lon: f64,
}

#[derive(Deserialize, Default)]
struct Address {
    #[serde(rename = "freeformAddress")]
    freeform: Option<String>,
}

#[derive(Deserialize)]
struct GeocodeResp {
    #[serde(default)]
    results: Vec<GeocodeHit>,
}
#[derive(Deserialize)]
struct GeocodeHit {
    id: Option<String>,
    position: Pos,
    #[serde(default)]
    address: Address,
    score: Option<f64>,
}

#[derive(Deserialize)]
struct RouteResp {
    #[serde(default)]
    routes: Vec<Route>,
}
#[derive(Deserialize)]
struct Route {
    summary: Summary,
    legs: Vec<Leg>,
    guidance: Option<Guidance>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    length_in_meters: f64,
    travel_time_in_seconds: f64,
    traffic_delay_in_seconds: Option<f64>,
}
#[derive(Deserialize)]
struct Leg {
    summary: Summary,
    points: Vec<LegPoint>,
}
#[derive(Deserialize)]
struct LegPoint {
    latitude: f64,
    longitude: f64,
}
#[derive(Deserialize)]
struct Guidance {
    #[serde(default)]
    instructions: Vec<Instruction>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Instruction {
    point_index: usize,
    route_offset_in_meters: f64,
    travel_time_in_seconds: f64,
}

#[derive(Deserialize)]
struct SearchResp {
    #[serde(default)]
    results: Vec<SearchHit>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchHit {
    id: Option<String>,
    poi: Option<Poi>,
    position: Pos,
    #[serde(default)]
    address: Address,
    detour_time: Option<f64>,
    detour_distance: Option<f64>,
}
#[derive(Deserialize)]
struct Poi {
    name: Option<String>,
    #[serde(default)]
    brands: Vec<Brand>,
}
#[derive(Deserialize)]
struct Brand {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CategoriesResp {
    #[serde(default)]
    poi_categories: Vec<PoiCategory>,
}
#[derive(Deserialize)]
struct PoiCategory {
    id: u32,
    name: String,
    #[serde(default)]
    synonyms: Vec<String>,
}

impl ProviderAdapter for TomTom {
    fn kind(&self) -> ProviderKind {
        ProviderKind::TomTom
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            detour: DetourStrategy::Native,
            ev_power_filter: true,
            ev_connector_filter: true,
            brand_filter: true,
            avoid_tolls: true,
            avoid_highways: true,
            avoid_ferries: true,
            max_search_points: MAX_SEARCH_POINTS,
            max_search_results: 20,
        }
    }

    fn attribution(&self) -> &'static str {
        "© TomTom"
    }

    fn setup_requests(&self, id_prefix: &str) -> Vec<HttpRequest> {
        let u = format!("{}/search/2/poiCategories.json", self.base);
        vec![self.request(
            format!("{id_prefix}categories"),
            "GET",
            url(&u, &[("language", "en-GB".into())]),
            None,
        )]
    }

    fn apply_setup(&mut self, resp: &HttpResponse) -> Result<(), ProviderError> {
        let r: CategoriesResp = parse_json(resp)?;
        for (_, cat) in Category::ALL {
            let names = provider_names(cat);
            let hit = r.poi_categories.iter().find(|pc| {
                names.contains(&pc.name.to_lowercase().as_str())
                    || pc
                        .synonyms
                        .iter()
                        .any(|s| names.contains(&s.to_lowercase().as_str()))
            });
            if let Some(pc) = hit {
                self.category_ids.insert(cat, pc.id);
            }
        }
        Ok(())
    }

    fn geocode_request(&self, id: String, query: &str, bias: Option<LatLng>) -> HttpRequest {
        let base = format!("{}/search/2/geocode/{}.json", self.base, pct(query));
        let mut params = vec![("limit", "5".to_string())];
        if let Some(b) = bias {
            params.push(("lat", format!("{:.6}", b.lat)));
            params.push(("lon", format!("{:.6}", b.lng)));
        }
        self.request(id, "GET", url(&base, &params), None)
    }

    fn parse_geocode(&self, resp: &HttpResponse) -> Result<Vec<GeocodeCandidate>, ProviderError> {
        let r: GeocodeResp = parse_json(resp)?;
        Ok(r.results
            .into_iter()
            .map(|h| {
                let position = LatLng::new(h.position.lat, h.position.lon);
                GeocodeCandidate {
                    id: h.id.unwrap_or_else(|| position_id(position)),
                    label: h.address.freeform.unwrap_or_default(),
                    position,
                    score: h.score,
                }
            })
            .filter(|c| c.position.is_valid())
            .collect())
    }

    fn route_request(&self, id: String, q: &RouteQuery) -> HttpRequest {
        let locs: Vec<String> = q.points.iter().map(|p| ll(*p)).collect();
        let base = format!(
            "{}/routing/1/calculateRoute/{}/json",
            self.base,
            locs.join(":")
        );
        let mut params = vec![
            ("instructionsType", "coded".to_string()),
            ("routeRepresentation", "polyline".to_string()),
            ("traffic", "true".to_string()),
            ("travelMode", "car".to_string()),
        ];
        for a in &q.avoid {
            let v = match a {
                Avoid::Tolls => "tollRoads",
                Avoid::Highways => "motorways",
                Avoid::Ferries => "ferries",
            };
            params.push(("avoid", v.to_string()));
        }
        if let Some(t) = &q.departure_time {
            params.push(("departAt", t.clone()));
        }
        if q.electric {
            params.push(("vehicleEngineType", "electric".to_string()));
        }
        self.request(id, "GET", url(&base, &params), None)
    }

    fn parse_route(&self, resp: &HttpResponse) -> Result<RouteResult, ProviderError> {
        let r: RouteResp = parse_json(resp)?;
        let Some(route) = r.routes.into_iter().next() else {
            return Err(ProviderError {
                kind: ErrorKind::NoRoute,
                message: "TomTom returned no route".into(),
                retryable: false,
            });
        };

        // Concatenate legs, dropping a leg's first point when it repeats the previous leg's last.
        let mut coords: Vec<LatLng> = Vec::new();
        let mut leg_end_m = Vec::with_capacity(route.legs.len());
        let mut cum = 0.0;
        for leg in &route.legs {
            for p in &leg.points {
                let pt = LatLng::new(p.latitude, p.longitude);
                if coords.last() != Some(&pt) {
                    coords.push(pt);
                }
            }
            cum += leg.summary.length_in_meters;
            leg_end_m.push(cum);
        }
        let total_m = route.summary.length_in_meters;
        let total_s = route.summary.travel_time_in_seconds;
        let last = coords.len().saturating_sub(1);

        // Checkpoints from guidance instructions; keep only strictly increasing ones.
        let mut cps = vec![Checkpoint {
            index: 0,
            cum_m: 0.0,
            cum_s: 0.0,
        }];
        if let Some(g) = &route.guidance {
            let mut ins: Vec<&Instruction> = g.instructions.iter().collect();
            ins.sort_by_key(|i| i.point_index);
            for i in ins {
                let prev = cps[cps.len() - 1];
                let ok = i.point_index > prev.index
                    && i.point_index < last
                    && i.route_offset_in_meters >= prev.cum_m
                    && i.travel_time_in_seconds >= prev.cum_s
                    && i.route_offset_in_meters < total_m
                    && i.travel_time_in_seconds <= total_s;
                if ok {
                    cps.push(Checkpoint {
                        index: i.point_index,
                        cum_m: i.route_offset_in_meters,
                        cum_s: i.travel_time_in_seconds,
                    });
                }
            }
        }
        cps.push(Checkpoint {
            index: last,
            cum_m: total_m,
            cum_s: total_s,
        });

        let measured = MeasuredRoute::new(coords, &cps)
            .map_err(|e| ProviderError::bad_response(format!("TomTom route geometry: {e}")))?;
        if let Some(l) = leg_end_m.last_mut() {
            *l = total_m;
        }
        Ok(RouteResult {
            route: measured,
            leg_end_m,
            traffic_delay_s: route.summary.traffic_delay_in_seconds,
        })
    }

    fn search_request(&self, id: String, q: &SearchQuery) -> HttpRequest {
        let category_id = q.what.category.and_then(|c| self.category_id(c));
        // Found against the live API: free text plus `categorySet` returns no results,
        // while `*` plus `categorySet` works. A brand name plus `categorySet` also works.
        let text = match (&q.what.brand, category_id) {
            (None, Some(_)) => "*".to_string(),
            _ => query_text(&q.what),
        };
        let base = format!(
            "{}/search/2/searchAlongRoute/{}.json",
            self.base,
            pct(&text)
        );
        let mut params = vec![
            ("maxDetourTime", q.max_detour_s.clamp(1, 3600).to_string()),
            ("limit", q.limit.clamp(1, 20).to_string()),
            ("sortBy", "detourTime".to_string()),
            ("spreadingMode", "auto".to_string()),
        ];
        if let Some(cat) = category_id {
            params.push(("categorySet", cat.to_string()));
        }
        // No `brandSet`: it only matches chains in TomTom's brand registry, so a local
        // name ("Cafe de Jaren") would find nothing. The name as search text finds both
        // chains (even misspelt, "macdonalds") and single places.
        if let Some(ev) = &q.what.ev {
            if let Some(kw) = ev.min_power_kw {
                params.push(("minPowerKW", format!("{kw}")));
            }
            if !ev.connector_types.is_empty() {
                let codes: Vec<&str> = ev
                    .connector_types
                    .iter()
                    .map(|c| connector_code(*c))
                    .collect();
                params.push(("connectorSet", codes.join(",")));
            }
        }
        let points: Vec<serde_json::Value> = q
            .window_line
            .iter()
            .map(|p| json!({ "lat": p.lat, "lon": p.lng }))
            .collect();
        self.request(
            id,
            "POST",
            url(&base, &params),
            Some(json!({ "route": { "points": points } })),
        )
    }

    fn parse_search(&self, resp: &HttpResponse) -> Result<Vec<Candidate>, ProviderError> {
        let r: SearchResp = parse_json(resp)?;
        Ok(r.results
            .into_iter()
            .enumerate()
            .filter_map(|(i, h)| {
                let position = LatLng::new(h.position.lat, h.position.lon);
                if !position.is_valid() {
                    return None;
                }
                let poi = h.poi;
                let name = poi
                    .as_ref()
                    .and_then(|p| p.name.clone())
                    .unwrap_or_default();
                let brand = poi.and_then(|p| p.brands.into_iter().next().map(|b| b.name));
                Some(Candidate {
                    provider_place_id: h.id.unwrap_or_else(|| format!("tomtom-result-{i}")),
                    name,
                    position,
                    address: h.address.freeform,
                    brand,
                    detour_s: h.detour_time,
                    detour_m: h.detour_distance,
                })
            })
            .collect())
    }
}
