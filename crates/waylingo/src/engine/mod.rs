//! The session state machine: parse, prepare, base route, search, detour, final
//! route, done. [`Engine::resume`] continues a finished result after the end user
//! picks places, without calling the LLM.
//!
//! * `phases`: requests for geocoding, the base route, searches and detour matrices.
//! * `stops`: choosing stops and routing through them.
//! * `resume`: rebuilding a session from a previous result.
//! * `anchor`: turning plan anchors into route geometry.

mod anchor;
mod phases;
mod resume;
mod stops;

use std::collections::HashMap;

use waylingo_geo::{AnchorPoint, LatLng, Window};
use waylingo_http::{HttpRequest, HttpResponse};
use waylingo_parse::{ParseInput, ParseStep, Parser};
use waylingo_plan::{Location, Plan, TripContext};
use waylingo_providers::{
    Candidate, ErrorKind, GeocodeCandidate, Here, ProviderAdapter, ProviderError, RouteResult,
    TomTom,
};

use crate::choices::StopPick;
use crate::output::{self, Failure, LocationOut, Role};
use crate::result::SearchEcho;
use crate::result::UnresolvedReason;
use crate::select::{Choice, Placed};
use crate::{EngineConfig, ProviderChoice};

/// One request to the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct TripRequest {
    pub query: String,
    pub context: TripContext,
    /// Echoed in the output.
    pub request_id: Option<String>,
}

/// What the host must do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Send every request (in any order or in parallel) and pass all responses to
    /// [`Session::step`]. Add the key named by each request's `credential` as its
    /// `auth` says. Honour `not_before_ms` and `timeout_ms`.
    NeedRequests { requests: Vec<HttpRequest> },
    /// The RFC 7946 `FeatureCollection`, serialized. See [`crate::TripResult`].
    Done { geojson: String },
}

/// Misuse of the API. Provider and LLM failures are not errors: they are reported
/// inside the GeoJSON.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WaylingoError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("response for unknown request id {0:?}")]
    UnknownResponse(String),
    #[error("{0} requests of the last batch got no response")]
    MissingResponses(usize),
    #[error("the session has already finished")]
    Finished,
    /// The previous result cannot be continued: not a Waylingo result, another
    /// schema version or provider, or no plan.
    #[error("cannot continue this result: {0}")]
    IncompatibleResult(String),
    /// A choice names something that is not in the previous result.
    #[error("unknown choice: {0}")]
    UnknownChoice(String),
}

pub struct Engine {
    config: EngineConfig,
}

impl Engine {
    #[must_use]
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }

    fn adapter(&self) -> Box<dyn ProviderAdapter + Send> {
        match &self.config.provider {
            ProviderChoice::TomTom { base_url } => Box::new(TomTom::new(base_url.as_deref())),
            ProviderChoice::Here { base_url } => Box::new(Here::new(base_url.as_deref())),
        }
    }

    fn session(&self, ctx: TripContext, request_id: Option<String>) -> Session {
        Session {
            cfg: self.config.clone(),
            ctx,
            request_id,
            adapter: self.adapter(),
            phase: Phase::Parse,
            parser: None,
            plan: None,
            pending: HashMap::new(),
            next_id: 0,
            llm_calls: 0,
            map_calls: 0,
            locations: Vec::new(),
            geocoded: Vec::new(),
            chosen: Vec::new(),
            along: Vec::new(),
            base: None,
            slots: Vec::new(),
            final_route: None,
            stop_legs: Vec::new(),
            pins: HashMap::new(),
            resumed: false,
            assumptions: Vec::new(),
            warnings: Vec::new(),
            failure: None,
        }
    }

    /// Start one query.
    ///
    /// # Errors
    /// [`WaylingoError::InvalidRequest`] for an empty or oversized query or
    /// out-of-range settings.
    pub fn start(&self, req: TripRequest) -> Result<(Session, Step), WaylingoError> {
        req.context
            .defaults
            .validate()
            .map_err(WaylingoError::InvalidRequest)?;
        let input = ParseInput::new(&req.query, &req.context)
            .map_err(|e| WaylingoError::InvalidRequest(e.to_string()))?;
        let (parser, first) = Parser::start(&input, self.config.llm.clone());
        let mut s = self.session(req.context, req.request_id);
        s.parser = Some(parser);
        s.llm_calls = 1;
        s.pending.insert(
            first.id.clone(),
            Pending {
                kind: Kind::Llm,
                req: first.clone(),
                attempts: 0,
            },
        );
        Ok((
            s,
            Step::NeedRequests {
                requests: vec![first],
            },
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Parse,
    Prepare,
    BaseRoute,
    Search,
    Detour,
    FinalRoute,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Llm,
    Setup,
    Geocode(usize),
    Route,
    Search(usize),
    Matrix(usize),
    FinalRoute,
}

struct Pending {
    kind: Kind,
    req: HttpRequest,
    attempts: u8,
}

pub(crate) enum Outcome {
    Waiting,
    Candidates(Vec<Candidate>),
    Found(Choice),
    /// Nothing usable: how many results the provider returned, and how many of those
    /// were within the detour limit (the rest were already used by another stop).
    NoCandidates {
        returned: usize,
        within_detour: usize,
    },
    SkippedBudget,
    /// Left out by the end user. Keeps the places that were offered, so the stop can
    /// be brought back with a later choice.
    SkippedByUser {
        options: Vec<Placed>,
    },
    Unsupported(String),
    ProviderError(ProviderError),
    /// An unresolved stop copied from a previous result.
    Carried {
        result: UnresolvedReason,
        detail: Option<String>,
    },
}

pub(crate) struct Slot {
    pub stop: usize,
    pub occurrence: usize,
    pub anchor: Option<AnchorPoint>,
    pub window: Option<Window>,
    pub outcome: Outcome,
}

impl Slot {
    fn new(stop: usize, occurrence: usize, anchor: Option<AnchorPoint>, outcome: Outcome) -> Self {
        Self {
            stop,
            occurrence,
            anchor,
            window: None,
            outcome,
        }
    }
}

/// One query in progress. Single use.
pub struct Session {
    cfg: EngineConfig,
    ctx: TripContext,
    request_id: Option<String>,
    adapter: Box<dyn ProviderAdapter + Send>,
    phase: Phase,
    parser: Option<Parser>,
    plan: Option<Plan>,
    pending: HashMap<String, Pending>,
    next_id: u32,
    llm_calls: u32,
    map_calls: u32,
    /// Origin, waypoints in order, destination.
    locations: Vec<Option<LatLng>>,
    /// Geocoding matches per location (empty for given coordinates).
    geocoded: Vec<Vec<GeocodeCandidate>>,
    /// The candidate id in use per location.
    chosen: Vec<Option<String>>,
    /// Distance along the base route to each location, metres.
    along: Vec<Option<f64>>,
    base: Option<RouteResult>,
    slots: Vec<Slot>,
    final_route: Option<RouteResult>,
    /// (slot index, index of the stop among the final route's intermediate points).
    stop_legs: Vec<(usize, usize)>,
    /// Stop choices carried into a re-search: (stop request id, occurrence) -> pick.
    pins: HashMap<(String, usize), StopPick>,
    /// True for a session started by [`Engine::resume`].
    resumed: bool,
    assumptions: Vec<String>,
    warnings: Vec<String>,
    failure: Option<Failure>,
}

impl Session {
    /// Continue with the responses to the last batch.
    ///
    /// # Errors
    /// Unknown or missing responses, or a call after the session finished.
    pub fn step(&mut self, responses: Vec<HttpResponse>) -> Result<Step, WaylingoError> {
        if self.phase == Phase::Done {
            return Err(WaylingoError::Finished);
        }
        let mut next = Vec::new();
        for resp in responses {
            let p = self
                .pending
                .remove(&resp.id)
                .ok_or_else(|| WaylingoError::UnknownResponse(resp.id.clone()))?;
            if let Some(r) = self.handle(p, &resp) {
                next.push(r);
            }
        }
        let missing = self.pending.len() - next.len();
        if missing > 0 {
            return Err(WaylingoError::MissingResponses(missing));
        }
        if !next.is_empty() {
            return Ok(Step::NeedRequests { requests: next });
        }
        Ok(self.advance())
    }

    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}-{}", self.next_id)
    }

    fn register(&mut self, kind: Kind, mut req: HttpRequest) -> HttpRequest {
        if kind != Kind::Llm {
            self.map_calls += 1;
        }
        req.id = self.id(match kind {
            Kind::Llm => "llm",
            Kind::Setup => "setup",
            Kind::Geocode(_) => "geocode",
            Kind::Route => "route",
            Kind::Search(_) => "search",
            Kind::Matrix(_) => "matrix",
            Kind::FinalRoute => "final-route",
        });
        self.pending.insert(
            req.id.clone(),
            Pending {
                kind,
                req: req.clone(),
                attempts: 0,
            },
        );
        req
    }

    fn fail(&mut self, code: &str, message: impl Into<String>) {
        if self.failure.is_none() {
            self.failure = Some(Failure {
                code: code.to_string(),
                message: message.into(),
            });
        }
    }

    fn fail_provider(&mut self, e: &ProviderError, what: &str) {
        let code = match e.kind {
            ErrorKind::Auth => "PROVIDER_AUTH",
            ErrorKind::QuotaExceeded => "QUOTA_EXCEEDED",
            ErrorKind::RateLimited => "RATE_LIMITED",
            ErrorKind::NoRoute => "NO_ROUTE",
            ErrorKind::Transport => "PROVIDER_UNREACHABLE",
            ErrorKind::BadRequest | ErrorKind::Upstream | ErrorKind::BadResponse => {
                "PROVIDER_ERROR"
            }
        };
        self.fail(code, format!("{what}: {}", e.message));
    }

    /// Process one response. Returns a follow-up request (LLM repair or retry), if any.
    fn handle(&mut self, p: Pending, resp: &HttpResponse) -> Option<HttpRequest> {
        if p.kind == Kind::Llm {
            return self.handle_llm(resp);
        }
        if resp.is_retryable() && p.attempts < self.cfg.limits.max_map_retries {
            let attempts = p.attempts + 1;
            let mut again = p.req;
            again.not_before_ms = retry_after_ms(resp)
                .unwrap_or(1_000 << (attempts - 1))
                .min(30_000);
            let again = self.register(p.kind, again);
            if let Some(entry) = self.pending.get_mut(&again.id) {
                entry.attempts = attempts;
            }
            return Some(again);
        }
        match p.kind {
            Kind::Llm => {}
            Kind::Setup => {
                if let Err(e) = self.adapter.apply_setup(resp) {
                    self.warnings.push(format!(
                        "provider category list unavailable ({:?}); searching by text only",
                        e.kind
                    ));
                }
            }
            Kind::Geocode(i) => self.apply_geocode(i, resp),
            Kind::Route => self.apply_base_route(resp),
            Kind::Search(slot) => {
                self.slots[slot].outcome = match self.adapter.parse_search(resp) {
                    Ok(c) => Outcome::Candidates(c),
                    Err(e) => Outcome::ProviderError(e),
                };
            }
            Kind::Matrix(slot) => self.apply_matrix(slot, resp),
            Kind::FinalRoute => self.apply_final_route(resp),
        }
        None
    }

    fn handle_llm(&mut self, resp: &HttpResponse) -> Option<HttpRequest> {
        let parser = self.parser.as_mut()?;
        match parser.on_response(resp, &self.ctx) {
            ParseStep::Request(r) => {
                self.llm_calls += 1;
                self.pending.insert(
                    r.id.clone(),
                    Pending {
                        kind: Kind::Llm,
                        req: r.clone(),
                        attempts: 0,
                    },
                );
                Some(r)
            }
            ParseStep::Done(plan) => {
                self.reset_locations(plan.waypoints.len() + 2);
                self.plan = Some(*plan);
                self.parser = None;
                None
            }
            ParseStep::Failed(f) => {
                let code = json_code(&f.code);
                let mut msg = f.message;
                if !f.errors.is_empty() {
                    let list: Vec<String> = f.errors.iter().map(ToString::to_string).collect();
                    msg = format!("{msg}: {}", list.join("; "));
                }
                self.fail(&code, msg);
                self.parser = None;
                None
            }
        }
    }

    fn reset_locations(&mut self, n: usize) {
        self.locations = vec![None; n];
        self.geocoded = vec![Vec::new(); n];
        self.chosen = vec![None; n];
        self.along = vec![None; n];
    }

    fn place_text(&self, i: usize) -> String {
        self.plan_locations()
            .get(i)
            .map_or_else(String::new, |l| match l {
                Location::PlaceQuery { text } => text.clone(),
                Location::Coords { label, .. } => label.clone().unwrap_or_default(),
            })
    }

    /// Origin, waypoints in order, destination.
    fn plan_locations(&self) -> Vec<Location> {
        let Some(plan) = &self.plan else {
            return vec![];
        };
        let mut v = vec![plan.origin.clone()];
        v.extend(plan.waypoints.iter().cloned());
        v.push(plan.destination.clone());
        v
    }

    fn role(&self, i: usize) -> Role {
        if i == 0 {
            Role::Origin
        } else if i + 1 == self.locations.len() {
            Role::Destination
        } else {
            Role::Waypoint(u32::try_from(i - 1).unwrap_or(u32::MAX))
        }
    }

    fn advance(&mut self) -> Step {
        loop {
            if self.failure.is_some() {
                return self.finish();
            }
            let reqs = match self.phase {
                Phase::Parse => {
                    self.phase = Phase::Prepare;
                    self.prepare_requests()
                }
                Phase::Prepare => {
                    self.phase = Phase::BaseRoute;
                    self.base_route_request()
                }
                Phase::BaseRoute => {
                    self.phase = Phase::Search;
                    self.search_requests()
                }
                Phase::Search => {
                    self.phase = Phase::Detour;
                    self.detour_requests()
                }
                Phase::Detour => {
                    self.phase = Phase::FinalRoute;
                    self.choose_stops();
                    self.route_through_stops()
                }
                Phase::FinalRoute | Phase::Done => return self.finish(),
            };
            if !reqs.is_empty() {
                return Step::NeedRequests { requests: reqs };
            }
        }
    }

    fn finish(&mut self) -> Step {
        self.phase = Phase::Done;
        self.parser = None;
        let locations: Vec<LocationOut> = (0..self.locations.len())
            .map(|i| LocationOut {
                role: self.role(i),
                text: self.place_text(i),
                chosen_id: self.chosen.get(i).cloned().flatten(),
                position: self.locations[i],
                along_m: self.along.get(i).copied().flatten(),
                candidates: self.geocoded.get(i).cloned().unwrap_or_default(),
            })
            .collect();
        let geojson = output::build(&output::Inputs {
            request_id: self.request_id.as_deref(),
            provider: self.adapter.kind(),
            attribution: self.adapter.attribution(),
            plan: self.plan.as_ref(),
            base: self.base.as_ref(),
            final_route: self.final_route.as_ref(),
            locations,
            slots: &self.slots,
            stop_legs: &self.stop_legs,
            assumptions: &self.assumptions,
            warnings: &self.warnings,
            failure: self.failure.as_ref(),
            llm_calls: self.llm_calls,
            map_calls: self.map_calls,
            search: SearchEcho::new(
                &self.ctx.defaults,
                self.adapter.capabilities().max_search_results,
            ),
            signing_key: self.cfg.signing_key.as_ref(),
        });
        Step::Done { geojson }
    }
}

fn retry_after_ms(resp: &HttpResponse) -> Option<u32> {
    let secs: f64 = resp.header("Retry-After")?.trim().parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| (secs * 1000.0).min(30_000.0) as u32)
}

fn json_code<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "UNKNOWN".into())
}
