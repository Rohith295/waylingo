//! HERE adapter: Routing v8, Geocoding & Search v7 (geocode, browse, discover),
//! Matrix Routing v8.
//!
//! HERE's along-route search returns a distance off the route, not detour time, so
//! this adapter uses [`DetourStrategy::Matrix`]: the engine asks for one travel-time
//! matrix per stop. HERE's terms forbid using its content with AI systems; Waylingo
//! never sends provider results to the LLM.
//!
//! Docs: <https://docs.here.com/routing/reference>, <https://docs.here.com/geocoding-and-search/reference>.

use serde::Deserialize;
use serde_json::json;
use waylingo_geo::{Checkpoint, LatLng, MeasuredRoute, polyline};
use waylingo_http::{AuthPlacement, CredentialSlot, Header, HttpRequest, HttpResponse};
use waylingo_plan::{Avoid, Category, What};

use crate::category::query_text;
use crate::encode::{ll, position_id, url};
use crate::{
    Candidate, Capabilities, DetourStrategy, ErrorKind, GeocodeCandidate, ProviderAdapter,
    ProviderError, ProviderKind, RouteQuery, RouteResult, SearchQuery,
};

const TIMEOUT_MS: u32 = 8_000;
/// Keeps the GET URL with the encoded corridor well under HERE's 2048-byte guidance.
const MAX_SEARCH_POINTS: usize = 120;
const MAX_RESULTS: u8 = 100;

/// HERE category ids (HERE Places category system).
fn category_id(c: Category) -> &'static str {
    match c {
        Category::EvCharging => "700-7600-0322",
        Category::Fuel => "700-7600-0116",
        Category::Restaurant => "100-1000-0000",
        Category::FastFood => "100-1000-0009",
        Category::Coffee => "100-1100-0010",
        Category::RestArea => "400-4300-0000",
        Category::Hotel => "500-5000-0053",
        Category::Parking => "800-8500-0000",
        Category::Supermarket => "600-6300-0066",
        Category::Pharmacy => "600-6400-0070",
    }
}

/// Service endpoints. A proxy base maps each service to `{base}/{service}`.
#[derive(Debug, Clone)]
struct Hosts {
    router: String,
    geocode: String,
    browse: String,
    discover: String,
    matrix: String,
}

/// HERE adapter.
#[derive(Debug, Clone)]
pub struct Here {
    hosts: Hosts,
}

impl Default for Here {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Here {
    /// `base_url` routes every service through a proxy as `{base}/router`,
    /// `{base}/geocode`, `{base}/browse`, `{base}/discover`, `{base}/matrix`.
    #[must_use]
    pub fn new(base_url: Option<&str>) -> Self {
        let hosts = match base_url.map(|b| b.trim_end_matches('/')) {
            Some(b) => Hosts {
                router: format!("{b}/router/v8/routes"),
                geocode: format!("{b}/geocode/v1/geocode"),
                browse: format!("{b}/browse/v1/browse"),
                discover: format!("{b}/discover/v1/discover"),
                matrix: format!("{b}/matrix/v8/matrix"),
            },
            None => Hosts {
                router: "https://router.hereapi.com/v8/routes".into(),
                geocode: "https://geocode.search.hereapi.com/v1/geocode".into(),
                browse: "https://browse.search.hereapi.com/v1/browse".into(),
                discover: "https://discover.search.hereapi.com/v1/discover".into(),
                matrix: "https://matrix.router.hereapi.com/v8/matrix".into(),
            },
        };
        Self { hosts }
    }

    fn request(
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
            auth: AuthPlacement::QueryParam {
                name: "apiKey".into(),
            },
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
        .map_err(|e| ProviderError::bad_response(format!("HERE JSON: {e}")))
}

// ---- response shapes (only the fields we use) ----

#[derive(Deserialize)]
struct Pos {
    lat: f64,
    lng: f64,
}

#[derive(Deserialize, Default)]
struct Address {
    label: Option<String>,
}

#[derive(Deserialize)]
struct GeocodeResp {
    #[serde(default)]
    items: Vec<GeocodeItem>,
}
#[derive(Deserialize)]
struct GeocodeItem {
    id: Option<String>,
    title: Option<String>,
    position: Pos,
    #[serde(default)]
    address: Address,
    scoring: Option<Scoring>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scoring {
    query_score: Option<f64>,
}

#[derive(Deserialize)]
struct RouteResp {
    #[serde(default)]
    routes: Vec<Route>,
}
#[derive(Deserialize)]
struct Route {
    sections: Vec<Section>,
}
#[derive(Deserialize)]
struct Section {
    polyline: String,
    summary: Summary,
    #[serde(default)]
    actions: Vec<Action>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    length: f64,
    duration: f64,
    base_duration: Option<f64>,
}
#[derive(Deserialize)]
struct Action {
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    length: f64,
    #[serde(default)]
    duration: f64,
}

#[derive(Deserialize)]
struct SearchResp {
    #[serde(default)]
    items: Vec<SearchItem>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchItem {
    id: Option<String>,
    title: Option<String>,
    position: Option<Pos>,
    #[serde(default)]
    address: Address,
    excursion_distance: Option<f64>,
    #[serde(default)]
    chains: Vec<Chain>,
}
#[derive(Deserialize)]
struct Chain {
    name: Option<String>,
}

#[derive(Deserialize)]
struct MatrixResp {
    matrix: Matrix,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Matrix {
    #[serde(default)]
    travel_times: Vec<f64>,
    #[serde(default)]
    error_codes: Vec<i64>,
}

fn midpoint(line: &[LatLng]) -> LatLng {
    line.get(line.len() / 2)
        .copied()
        .unwrap_or(LatLng::new(0.0, 0.0))
}

impl ProviderAdapter for Here {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Here
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            detour: DetourStrategy::Matrix,
            ev_power_filter: true,
            ev_connector_filter: false,
            brand_filter: true,
            avoid_tolls: true,
            avoid_highways: true,
            avoid_ferries: true,
            max_search_points: MAX_SEARCH_POINTS,
            max_search_results: MAX_RESULTS,
        }
    }

    fn attribution(&self) -> &'static str {
        "© HERE"
    }

    fn setup_requests(&self, _id_prefix: &str) -> Vec<HttpRequest> {
        vec![]
    }

    fn apply_setup(&mut self, _resp: &HttpResponse) -> Result<(), ProviderError> {
        Ok(())
    }

    fn geocode_request(&self, id: String, query: &str, bias: Option<LatLng>) -> HttpRequest {
        let mut params = vec![("q", query.to_string()), ("limit", "5".to_string())];
        if let Some(b) = bias {
            params.push(("at", ll(b)));
        }
        Self::request(id, "GET", url(&self.hosts.geocode, &params), None)
    }

    fn parse_geocode(&self, resp: &HttpResponse) -> Result<Vec<GeocodeCandidate>, ProviderError> {
        let r: GeocodeResp = parse_json(resp)?;
        Ok(r.items
            .into_iter()
            .map(|i| {
                let position = LatLng::new(i.position.lat, i.position.lng);
                GeocodeCandidate {
                    id: i.id.unwrap_or_else(|| position_id(position)),
                    label: i.address.label.or(i.title).unwrap_or_default(),
                    position,
                    score: i.scoring.and_then(|s| s.query_score),
                }
            })
            .filter(|c| c.position.is_valid())
            .collect())
    }

    fn route_request(&self, id: String, q: &RouteQuery) -> HttpRequest {
        let mut params = vec![
            ("transportMode", "car".to_string()),
            ("return", "polyline,summary,actions".to_string()),
        ];
        if let Some(o) = q.points.first() {
            params.push(("origin", ll(*o)));
        }
        if let Some(d) = q.points.last() {
            params.push(("destination", ll(*d)));
        }
        let n = q.points.len();
        for p in q.points.iter().take(n.saturating_sub(1)).skip(1) {
            params.push(("via", ll(*p)));
        }
        let avoid: Vec<&str> = q
            .avoid
            .iter()
            .map(|a| match a {
                Avoid::Tolls => "tollRoad",
                Avoid::Highways => "controlledAccessHighway",
                Avoid::Ferries => "ferry",
            })
            .collect();
        if !avoid.is_empty() {
            params.push(("avoid[features]", avoid.join(",")));
        }
        if let Some(t) = &q.departure_time {
            params.push(("departureTime", t.clone()));
        }
        Self::request(id, "GET", url(&self.hosts.router, &params), None)
    }

    fn parse_route(&self, resp: &HttpResponse) -> Result<RouteResult, ProviderError> {
        let r: RouteResp = parse_json(resp)?;
        let Some(route) = r.routes.into_iter().next() else {
            return Err(ProviderError {
                kind: ErrorKind::NoRoute,
                message: "HERE returned no route".into(),
                retryable: false,
            });
        };

        let mut coords: Vec<LatLng> = Vec::new();
        let mut cps = vec![Checkpoint {
            index: 0,
            cum_m: 0.0,
            cum_s: 0.0,
        }];
        let mut leg_end_m = Vec::with_capacity(route.sections.len());
        let (mut base_m, mut base_s, mut traffic) = (0.0, 0.0, 0.0);
        for sec in &route.sections {
            let pts = polyline::decode_here(&sec.polyline)
                .map_err(|e| ProviderError::bad_response(format!("HERE polyline: {e}")))?;
            // Index of this section's first point in the concatenated line.
            let start = match (coords.last(), pts.first()) {
                (Some(l), Some(f)) if l == f => coords.len() - 1,
                _ => coords.len(),
            };
            let skip = usize::from(start < coords.len());
            coords.extend(pts.iter().skip(skip).copied());

            // Each action covers the stretch from its offset to the next action.
            let (mut run_m, mut run_s) = (0.0, 0.0);
            for a in &sec.actions {
                let index = start + a.offset;
                let prev = cps[cps.len() - 1];
                let (cm, cs) = (base_m + run_m, base_s + run_s);
                if index > prev.index && cm >= prev.cum_m && cs >= prev.cum_s {
                    cps.push(Checkpoint {
                        index,
                        cum_m: cm,
                        cum_s: cs,
                    });
                }
                run_m += a.length;
                run_s += a.duration;
            }
            base_m += sec.summary.length;
            base_s += sec.summary.duration;
            traffic +=
                sec.summary.duration - sec.summary.base_duration.unwrap_or(sec.summary.duration);
            leg_end_m.push(base_m);
            // A checkpoint at each section end keeps legs aligned exactly.
            let end = coords.len().saturating_sub(1);
            let prev = cps[cps.len() - 1];
            if end > prev.index && base_m >= prev.cum_m && base_s >= prev.cum_s {
                cps.push(Checkpoint {
                    index: end,
                    cum_m: base_m,
                    cum_s: base_s,
                });
            }
        }
        let last = coords.len().saturating_sub(1);
        // The final checkpoint must be the last coordinate with the route totals.
        while cps.len() > 1 && cps[cps.len() - 1].index >= last {
            cps.pop();
        }
        cps.retain(|c| c.cum_m < base_m || c.index == 0);
        cps.push(Checkpoint {
            index: last,
            cum_m: base_m,
            cum_s: base_s,
        });

        let measured = MeasuredRoute::new(coords, &cps)
            .map_err(|e| ProviderError::bad_response(format!("HERE route geometry: {e}")))?;
        Ok(RouteResult {
            route: measured,
            leg_end_m,
            traffic_delay_s: Some(traffic.max(0.0)),
        })
    }

    fn search_request(&self, id: String, q: &SearchQuery) -> HttpRequest {
        let corridor = polyline::encode_here(&q.window_line).unwrap_or_default();
        let w = q.corridor_m.clamp(100, 50_000);
        let route = format!("{corridor};w={w}");
        let at = ll(midpoint(&q.window_line));
        let limit = q.limit.clamp(1, MAX_RESULTS).to_string();
        let What {
            category,
            text,
            brand,
            ev,
        } = &q.what;
        // Category without a brand or free text: browse by category id.
        // Otherwise discover by name, which handles brands and free text.
        if let (Some(c), None, None) = (category, brand, text) {
            let mut params = vec![
                ("at", at),
                ("route", route),
                ("categories", category_id(*c).to_string()),
                ("limit", limit),
            ];
            if *c == Category::EvCharging
                && let Some(kw) = ev.as_ref().and_then(|e| e.min_power_kw)
            {
                params.push(("evStation[minPower]", format!("{kw}")));
                params.push(("show", "ev".to_string()));
            }
            return Self::request(id, "GET", url(&self.hosts.browse, &params), None);
        }
        let params = vec![
            ("at", at),
            ("route", route),
            ("q", query_text(&q.what)),
            ("limit", limit),
        ];
        Self::request(id, "GET", url(&self.hosts.discover, &params), None)
    }

    fn parse_search(&self, resp: &HttpResponse) -> Result<Vec<Candidate>, ProviderError> {
        let r: SearchResp = parse_json(resp)?;
        Ok(r.items
            .into_iter()
            .enumerate()
            .filter_map(|(i, it)| {
                let p = it.position?;
                let position = LatLng::new(p.lat, p.lng);
                if !position.is_valid() {
                    return None;
                }
                Some(Candidate {
                    provider_place_id: it.id.unwrap_or_else(|| format!("here-result-{i}")),
                    name: it.title.unwrap_or_default(),
                    position,
                    address: it.address.label,
                    brand: it.chains.into_iter().find_map(|c| c.name),
                    // Filled by the engine from a matrix call.
                    detour_s: None,
                    detour_m: it.excursion_distance,
                })
            })
            .collect())
    }

    fn matrix_request(
        &self,
        id: String,
        origins: &[LatLng],
        destinations: &[LatLng],
    ) -> Option<HttpRequest> {
        let pts = |v: &[LatLng]| {
            v.iter()
                .map(|p| json!({ "lat": p.lat, "lng": p.lng }))
                .collect::<Vec<_>>()
        };
        let body = json!({
            "origins": pts(origins),
            "destinations": pts(destinations),
            "regionDefinition": { "type": "world" },
            "matrixAttributes": ["travelTimes"],
            "transportMode": "car",
        });
        Some(Self::request(
            id,
            "POST",
            url(&self.hosts.matrix, &[("async", "false".into())]),
            Some(body),
        ))
    }

    fn parse_matrix(
        &self,
        resp: &HttpResponse,
        origins: usize,
        destinations: usize,
    ) -> Result<Vec<Option<f64>>, ProviderError> {
        let r: MatrixResp = parse_json(resp)?;
        let n = origins * destinations;
        if r.matrix.travel_times.len() != n {
            return Err(ProviderError::bad_response(format!(
                "HERE matrix has {} entries, expected {n}",
                r.matrix.travel_times.len()
            )));
        }
        Ok((0..n)
            .map(|i| {
                let failed = r.matrix.error_codes.get(i).is_some_and(|c| *c != 0);
                (!failed).then(|| r.matrix.travel_times[i])
            })
            .collect())
    }
}
