//! Map provider phases before stop choice: geocoding, the base route, the
//! along-route searches, and detour matrices for providers that need them.

use waylingo_geo::{LatLng, Window, haversine_m, resolve, simplify_to_max_points};
use waylingo_http::{HttpRequest, HttpResponse};
use waylingo_plan::Location;
use waylingo_providers::{Candidate, DetourStrategy, RouteQuery, SearchQuery};

use super::anchor::spec_for;
use super::{Kind, Outcome, Session, Slot};
use crate::choices::StopPick;

/// Candidates per stop sent to a travel-time matrix (HERE flexible mode allows 15 origins).
const MATRIX_SHORTLIST: usize = 8;
/// Most anchors one stop request may create ("every N" on a long route). With the
/// call budget this bounds the work any plan can cause.
const MAX_ANCHORS_PER_STOP: usize = 30;
/// Geocoding matches kept per place, offered as "did you mean" candidates.
const GEOCODE_CANDIDATES: usize = 5;

impl Session {
    /// Once-per-session provider requests (for example, category lists).
    pub(super) fn setup_requests(&mut self) -> Vec<HttpRequest> {
        let reqs = self.adapter.setup_requests("");
        reqs.into_iter()
            .map(|r| self.register(Kind::Setup, r))
            .collect()
    }

    pub(super) fn prepare_requests(&mut self) -> Vec<HttpRequest> {
        let mut out = self.setup_requests();
        let bias = self.ctx.current_location.map(|c| LatLng::new(c.lat, c.lng));
        for (i, loc) in self.plan_locations().into_iter().enumerate() {
            match loc {
                Location::Coords { lat, lng, .. } => {
                    self.locations[i] = Some(LatLng::new(lat, lng));
                }
                Location::PlaceQuery { text } => {
                    let r = self.adapter.geocode_request(String::new(), &text, bias);
                    out.push(self.register(Kind::Geocode(i), r));
                }
            }
        }
        out
    }

    pub(super) fn apply_geocode(&mut self, i: usize, resp: &HttpResponse) {
        match self.adapter.parse_geocode(resp) {
            Ok(mut hits) if !hits.is_empty() => {
                hits.truncate(GEOCODE_CANDIDATES);
                self.locations[i] = Some(hits[0].position);
                self.chosen[i] = Some(hits[0].id.clone());
                if hits.len() > 1 {
                    let text = self.place_text(i);
                    self.assumptions.push(format!(
                        "{text:?} matched {} places; used the provider's top match ({}); the others are in the location's candidates",
                        hits.len(),
                        hits[0].label
                    ));
                }
                self.geocoded[i] = hits;
            }
            Ok(_) => {
                let text = self.place_text(i);
                self.fail("NO_GEOCODE", format!("could not find a place for {text:?}"));
            }
            Err(e) => self.fail_provider(&e, "geocoding failed"),
        }
    }

    pub(super) fn base_route_request(&mut self) -> Vec<HttpRequest> {
        let Some(plan) = &self.plan else {
            return vec![];
        };
        let Some(points) = self
            .locations
            .iter()
            .copied()
            .collect::<Option<Vec<LatLng>>>()
        else {
            self.fail("NO_GEOCODE", "a place could not be resolved");
            return vec![];
        };
        let q = RouteQuery {
            points,
            avoid: plan.avoid.clone(),
            departure_time: plan.departure.time.clone(),
            electric: plan.vehicle.ev.is_some(),
        };
        let r = self.adapter.route_request(String::new(), &q);
        vec![self.register(Kind::Route, r)]
    }

    pub(super) fn apply_base_route(&mut self, resp: &HttpResponse) {
        match self.adapter.parse_route(resp) {
            Ok(r) => {
                // Origin at 0; each later location at the end of its leg.
                self.along = (0..self.locations.len())
                    .map(|i| {
                        if i == 0 {
                            Some(0.0)
                        } else {
                            r.leg_end_m.get(i - 1).copied()
                        }
                    })
                    .collect();
                self.base = Some(r);
            }
            Err(e) => self.fail_provider(&e, "routing failed"),
        }
    }

    pub(super) fn search_requests(&mut self) -> Vec<HttpRequest> {
        let (Some(plan), Some(base)) = (self.plan.clone(), self.base.clone()) else {
            return vec![];
        };
        let caps = self.adapter.capabilities();
        let span_m = f64::from(self.ctx.defaults.search_span_m());
        let limit = self.ctx.defaults.search_limit.min(caps.max_search_results);
        let mut out = Vec::new();
        for (si, sr) in plan.stop_requests.iter().enumerate() {
            let anchors = match spec_for(&sr.anchor, &base)
                .and_then(|spec| resolve(&base.route, &spec).map_err(|e| e.to_string()))
            {
                Ok(a) if !a.is_empty() => a,
                Ok(_) => {
                    let why = "the route is shorter than the interval".to_string();
                    self.slots
                        .push(Slot::new(si, 0, None, Outcome::Unsupported(why)));
                    continue;
                }
                Err(why) => {
                    self.slots
                        .push(Slot::new(si, 0, None, Outcome::Unsupported(why)));
                    continue;
                }
            };
            let mut anchors = anchors;
            if anchors.len() > MAX_ANCHORS_PER_STOP {
                self.warnings.push(format!(
                    "stop {:?} has {} marks on this route; only the first {MAX_ANCHORS_PER_STOP} are searched",
                    sr.id,
                    anchors.len()
                ));
                anchors.truncate(MAX_ANCHORS_PER_STOP);
            }
            for a in anchors {
                // A stop the end user skipped earlier is not searched again.
                if self.pins.get(&(sr.id.clone(), a.occurrence)) == Some(&StopPick::Skip) {
                    self.slots.push(Slot::new(
                        si,
                        a.occurrence,
                        Some(a),
                        Outcome::SkippedByUser {
                            options: Vec::new(),
                        },
                    ));
                    continue;
                }
                // Search a stretch of route around the anchor wide enough to find
                // places within the radius on either side; selection then prefers
                // places within the radius.
                let w = Window::by_distance(&base.route, &a, span_m);
                let idx = self.slots.len();
                let mut s = Slot::new(si, a.occurrence, Some(a), Outcome::Waiting);
                s.window = Some(w);
                // Keep one call in reserve for the final route.
                if self.map_calls + 1 >= self.cfg.limits.max_map_calls {
                    s.outcome = Outcome::SkippedBudget;
                    self.slots.push(s);
                    continue;
                }
                self.slots.push(s);
                let line = simplify_to_max_points(
                    &base.route.slice(w.start_m, w.end_m),
                    caps.max_search_points,
                );
                let q = SearchQuery {
                    window_line: line,
                    what: sr.what.clone(),
                    max_detour_s: plan.max_detour_s,
                    corridor_m: self.ctx.defaults.anchor_radius_m,
                    limit,
                };
                let r = self.adapter.search_request(String::new(), &q);
                out.push(self.register(Kind::Search(idx), r));
            }
        }
        out
    }

    /// For providers without native detour times: shortlist the candidates nearest
    /// each anchor and ask for one travel-time matrix per stop. Detour for place P in
    /// window A..B is t(A->P) + t(P->B) - t(A->B).
    pub(super) fn detour_requests(&mut self) -> Vec<HttpRequest> {
        if self.adapter.capabilities().detour != DetourStrategy::Matrix {
            return vec![];
        }
        let Some(base) = self.base.clone() else {
            return vec![];
        };
        let radius_m = f64::from(self.ctx.defaults.anchor_radius_m);
        let mut out = Vec::new();
        let mut skipped = 0;
        for i in 0..self.slots.len() {
            let (Some(anchor), Some(w)) = (self.slots[i].anchor, self.slots[i].window) else {
                continue;
            };
            let Outcome::Candidates(c) = &mut self.slots[i].outcome else {
                continue;
            };
            if c.is_empty() {
                continue;
            }
            let pinned = match self.pin_for(&self.slots[i]) {
                Some(StopPick::Place { provider_place_id }) => Some(provider_place_id.clone()),
                _ => None,
            };
            let Outcome::Candidates(c) = &mut self.slots[i].outcome else {
                continue;
            };
            shortlist(c, anchor.point, radius_m, pinned.as_deref());
            if self.map_calls + 1 >= self.cfg.limits.max_map_calls {
                skipped += 1;
                continue;
            }
            let (Some(a), Some(b)) = (
                base.route.position_at_distance(w.start_m),
                base.route.position_at_distance(w.end_m),
            ) else {
                continue;
            };
            let pts: Vec<LatLng> = c.iter().map(|x| x.position).collect();
            let mut origins = vec![a.point];
            origins.extend(&pts);
            let mut destinations = pts;
            destinations.push(b.point);
            if let Some(r) = self
                .adapter
                .matrix_request(String::new(), &origins, &destinations)
            {
                out.push(self.register(Kind::Matrix(i), r));
            }
        }
        if skipped > 0 {
            self.warnings.push(format!(
                "{skipped} stop(s) have no detour times: the call budget ran out before the matrix step"
            ));
        }
        out
    }

    pub(super) fn apply_matrix(&mut self, slot: usize, resp: &HttpResponse) {
        let Outcome::Candidates(c) = &self.slots[slot].outcome else {
            return;
        };
        let k = c.len();
        match self.adapter.parse_matrix(resp, k + 1, k + 1) {
            Ok(t) => {
                let at = |o: usize, d: usize| t.get(o * (k + 1) + d).copied().flatten();
                let direct = at(0, k);
                let detours: Vec<Option<f64>> = (0..k)
                    .map(|i| Some((at(0, i)? + at(i + 1, k)? - direct?).max(0.0)))
                    .collect();
                if let Outcome::Candidates(c) = &mut self.slots[slot].outcome {
                    for (cand, d) in c.iter_mut().zip(detours) {
                        cand.detour_s = d;
                    }
                }
            }
            Err(e) => self.warnings.push(format!(
                "detour times unavailable for one stop ({:?}); ranked by distance from the anchor",
                e.kind
            )),
        }
    }
}

/// Keep the candidates for a detour matrix: places within the radius before
/// others, nearest to the anchor first, at most [`MATRIX_SHORTLIST`]. A place the
/// end user chose is always kept, even when it is not among the nearest.
fn shortlist(c: &mut Vec<Candidate>, anchor: LatLng, radius_m: f64, pinned: Option<&str>) {
    c.sort_by(|a, b| {
        let (da, db) = (
            haversine_m(a.position, anchor),
            haversine_m(b.position, anchor),
        );
        (da > radius_m)
            .cmp(&(db > radius_m))
            .then(da.total_cmp(&db))
    });
    let keep = pinned.and_then(|id| {
        c.iter()
            .skip(MATRIX_SHORTLIST)
            .find(|x| x.provider_place_id == id)
            .cloned()
    });
    c.truncate(MATRIX_SHORTLIST);
    c.extend(keep);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: &str, lat: f64) -> Candidate {
        Candidate {
            provider_place_id: id.into(),
            name: id.into(),
            position: LatLng::new(lat, 13.0),
            address: None,
            brand: None,
            detour_s: None,
            detour_m: None,
        }
    }

    #[test]
    fn the_shortlist_keeps_a_chosen_place() {
        let anchor = LatLng::new(52.0, 13.0);
        // Twelve places, each further from the anchor than the last.
        let all: Vec<Candidate> = (0..12)
            .map(|k| cand(&format!("p{k}"), 52.0 + 0.01 * f64::from(k)))
            .collect();
        let mut c = all.clone();
        shortlist(&mut c, anchor, 5_000.0, None);
        assert_eq!(c.len(), MATRIX_SHORTLIST);
        assert!(!c.iter().any(|x| x.provider_place_id == "p11"));

        let mut c = all;
        shortlist(&mut c, anchor, 5_000.0, Some("p11"));
        assert_eq!(c.len(), MATRIX_SHORTLIST + 1);
        assert!(c.iter().any(|x| x.provider_place_id == "p11"));
        assert_eq!(c[0].provider_place_id, "p0", "nearest first");
    }
}
