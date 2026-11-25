//! Choosing one place per stop and routing through the chosen places.

use std::collections::HashSet;

use waylingo_geo::{AnchorPoint, LatLng, Window, haversine_m};
use waylingo_http::{HttpRequest, HttpResponse};
use waylingo_providers::{Candidate, RouteQuery, RouteResult};

use super::{Kind, Outcome, Session, Slot};
use crate::choices::StopPick;
use crate::result::UserChoice;
use crate::select::{Choice, Placed, Rules, choose, nearest_cum_m};

impl Session {
    /// The end user's choice for this stop, carried into a re-search.
    pub(super) fn pin_for(&self, s: &Slot) -> Option<&StopPick> {
        let id = &self.plan.as_ref()?.stop_requests.get(s.stop)?.id;
        self.pins.get(&(id.clone(), s.occurrence))
    }

    /// Choose stops in stop-request order, so one place is not used twice. Places the
    /// end user chose are reserved first.
    pub(super) fn choose_stops(&mut self) {
        let (Some(plan), Some(base)) = (self.plan.clone(), self.base.clone()) else {
            return;
        };
        let radius_m = f64::from(self.ctx.defaults.anchor_radius_m);
        let mut taken: HashSet<String> = HashSet::new();
        for s in &self.slots {
            if let (Some(StopPick::Place { provider_place_id }), Outcome::Candidates(c)) =
                (self.pin_for(s), &s.outcome)
                && c.iter().any(|x| &x.provider_place_id == provider_place_id)
            {
                taken.insert(provider_place_id.clone());
            }
        }
        // A choice for a stop whose search failed or found nothing cannot be kept.
        let mut dropped = self
            .slots
            .iter()
            .filter(|s| {
                matches!(self.pin_for(s), Some(StopPick::Place { .. }))
                    && !matches!(s.outcome, Outcome::Candidates(_))
            })
            .count();
        for i in 0..self.slots.len() {
            let pinned = match self.pin_for(&self.slots[i]) {
                Some(StopPick::Place { provider_place_id }) => Some(provider_place_id.clone()),
                _ => None,
            };
            let s = &mut self.slots[i];
            let (Outcome::Candidates(c), Some(anchor)) = (&s.outcome, s.anchor) else {
                continue;
            };
            let sr = &plan.stop_requests[s.stop];
            let rules = Rules {
                radius_m,
                strict: self.ctx.defaults.strict_radius,
                max_detour_s: plan.max_detour_s,
                top_n: sr.top_n,
                window: s.window,
            };
            let mut own_taken = taken.clone();
            if let Some(p) = &pinned {
                own_taken.remove(p);
            }
            let mut choice = choose(c, &base.route, &anchor, &rules, &own_taken);
            if let Some(p) = &pinned {
                let found = c.iter().find(|x| &x.provider_place_id == p).cloned();
                match (found, choice.as_mut()) {
                    (Some(cand), Some(ch)) => {
                        promote(ch, placed(cand, &base, &anchor, s.window), &rules);
                        ch.user_choice = Some(UserChoice::Chosen);
                    }
                    (Some(cand), None) => {
                        let pick = placed(cand, &base, &anchor, s.window);
                        choice = Some(Choice {
                            beyond_radius: pick.distance_m > radius_m,
                            pick,
                            alternatives: Vec::new(),
                            user_choice: Some(UserChoice::Chosen),
                        });
                    }
                    (None, Some(ch)) => {
                        ch.user_choice = Some(UserChoice::Dropped);
                        dropped += 1;
                    }
                    (None, None) => dropped += 1,
                }
            }
            s.outcome = match choice {
                Some(choice) => {
                    taken.insert(choice.pick.c.provider_place_id.clone());
                    Outcome::Found(choice)
                }
                None => {
                    let limit = f64::from(plan.max_detour_s);
                    Outcome::NoCandidates {
                        returned: c.len(),
                        within_detour: c
                            .iter()
                            .filter(|x| x.detour_s.is_none_or(|d| d <= limit))
                            .count(),
                    }
                }
            };
        }
        self.drop_picked_alternatives();
        if dropped > 0 {
            self.warnings.push(format!(
                "{dropped} earlier stop choice(s) were not found along the new route; the best match was used instead"
            ));
        }
    }

    /// Never offer a place as an alternative when another stop already uses it: the
    /// end user could not choose it.
    pub(super) fn drop_picked_alternatives(&mut self) {
        let picks: HashSet<String> = self
            .slots
            .iter()
            .filter_map(|s| match &s.outcome {
                Outcome::Found(c) => Some(c.pick.c.provider_place_id.clone()),
                _ => None,
            })
            .collect();
        for s in &mut self.slots {
            if let Outcome::Found(c) = &mut s.outcome {
                c.alternatives
                    .retain(|a| !picks.contains(&a.c.provider_place_id));
            }
        }
    }

    /// Route from the origin through waypoints and chosen stops, in order along the
    /// base route, to the destination.
    pub(super) fn route_through_stops(&mut self) -> Vec<HttpRequest> {
        let Some(plan) = self.plan.clone() else {
            return vec![];
        };
        let mut inter: Vec<(f64, LatLng, Option<usize>)> = Vec::new();
        let n = self.locations.len();
        for i in 1..n.saturating_sub(1) {
            if let (Some(p), Some(m)) = (self.locations[i], self.along.get(i).copied().flatten()) {
                inter.push((m, p, None));
            }
        }
        for (i, s) in self.slots.iter().enumerate() {
            if let Outcome::Found(c) = &s.outcome {
                inter.push((c.pick.along_m, c.pick.c.position, Some(i)));
            }
        }
        // Without stops the base route is the answer. Only a continuation that kept
        // the previous stops has no base route and must still ask for one.
        if inter.iter().all(|x| x.2.is_none()) && self.base.is_some() {
            return vec![];
        }
        inter.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.stop_legs = inter
            .iter()
            .enumerate()
            .filter_map(|(k, x)| x.2.map(|s| (s, k)))
            .collect();

        let (Some(origin), Some(dest)) = (
            self.locations.first().copied().flatten(),
            self.locations.last().copied().flatten(),
        ) else {
            return vec![];
        };
        let mut points = vec![origin];
        points.extend(inter.iter().map(|x| x.1));
        points.push(dest);
        let q = RouteQuery {
            points,
            avoid: plan.avoid.clone(),
            departure_time: plan.departure.time.clone(),
            electric: plan.vehicle.ev.is_some(),
        };
        let r = self.adapter.route_request(String::new(), &q);
        vec![self.register(Kind::FinalRoute, r)]
    }

    pub(super) fn apply_final_route(&mut self, resp: &HttpResponse) {
        match self.adapter.parse_route(resp) {
            Ok(r) => self.final_route = Some(r),
            // Without a base route there is nothing to fall back to, and a silent
            // fallback would hide that the end user's choice was not routed.
            Err(e) if self.base.is_none() => {
                self.fail_provider(&e, "routing through the chosen places failed");
            }
            Err(e) => self.warnings.push(format!(
                "final route with stops failed ({:?}); totals and arrival times are from the base route",
                e.kind
            )),
        }
    }
}

fn placed(
    c: Candidate,
    base: &RouteResult,
    anchor: &AnchorPoint,
    window: Option<Window>,
) -> Placed {
    Placed {
        along_m: nearest_cum_m(&base.route, c.position, window),
        distance_m: haversine_m(c.position, anchor.point),
        c,
    }
}

/// Make `pick` the pick of `ch`, keeping at most `top_n - 1` alternatives and
/// recomputing whether the pick lies beyond the radius.
fn promote(ch: &mut Choice, pick: Placed, rules: &Rules) {
    if ch.pick.c.provider_place_id != pick.c.provider_place_id {
        ch.alternatives
            .retain(|a| a.c.provider_place_id != pick.c.provider_place_id);
        let old = std::mem::replace(&mut ch.pick, pick);
        ch.alternatives.insert(0, old);
        ch.alternatives
            .truncate(usize::from(rules.top_n.max(1)).saturating_sub(1));
    }
    ch.beyond_radius = ch.pick.distance_m > rules.radius_m;
}
